use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{SecondsFormat, Utc};
use futures::stream::{self, StreamExt};
use reqwest::Client;

use crate::models::agent_metrics::{AgentMetrics, SystemInfoResponse};
use crate::models::app_state::{AlertConfig, AppState, HostRecord};
use crate::models::sse_payloads::{HostStatusPayload, SseBroadcast};
use crate::repositories::{alert_configs_repo, hosts_repo, metrics_repo};
use crate::services::alert_service;
use crate::services::hosts_snapshot;
use crate::services::metrics_service::{self, STATUS_PERIODIC_INTERVAL_SECS};
use chrono::DateTime;

/// Result of a single host scrape — carries data needed for batch DB persistence.
enum ScrapeOutcome {
    /// Scrape succeeded; metrics should be batch-inserted as online.
    Online(Box<AgentMetrics>),
    /// Agent unreachable; an offline record should be batch-inserted.
    Offline,
    /// Non-recoverable error (e.g., deserialization); no DB insert needed.
    Failed(String),
}

/// Server version (from Cargo.toml at build time)
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Minimum agent version the server fully supports
const MIN_AGENT_VERSION: &str = "0.1.0";

/// Semantic version comparison: returns true if `a < b`.
/// Handles multi-digit segments correctly (e.g. "0.9.0" < "0.10.0" → true).
fn semver_less_than(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u32> {
        s.split('.')
            .filter_map(|seg| seg.parse::<u32>().ok())
            .collect()
    };
    let va = parse(a);
    let vb = parse(b);
    va < vb // Vec<u32> lexicographic comparison on numeric segments
}
/// HTTP request timeout for each agent scrape (seconds)
const SCRAPE_TIMEOUT_SECS: u64 = 5;
/// Cooldown to suppress repeated UP/DOWN alert flapping (seconds)
const FLAP_COOLDOWN_SECS: u64 = 60;
/// Maximum backoff multiplier (2^4 = 16x base interval → 160s at 10s interval)
const MAX_BACKOFF_POWER: u32 = 4;
/// Reuse the agent scrape JWT until it is older than this. Agent tokens expire
/// after 60s (see `auth::generate_jwt`); rotating at 40s leaves a 20s safety
/// window for clock drift and in-flight requests.
const JWT_ROTATE_AFTER_SECS: u64 = 40;

/// Cached agent JWT shared across all hosts in a scrape cycle.
struct JwtCache {
    token: String,
    minted_at: Instant,
}

impl JwtCache {
    fn get_or_refresh(slot: &mut Option<JwtCache>) -> Result<&str, String> {
        let needs_refresh = slot
            .as_ref()
            .is_none_or(|c| c.minted_at.elapsed() >= Duration::from_secs(JWT_ROTATE_AFTER_SECS));
        if needs_refresh {
            let token = crate::services::auth::generate_jwt()
                .map_err(|e| format!("JWT Generation Error: {}", e))?;
            *slot = Some(JwtCache {
                token,
                minted_at: Instant::now(),
            });
        }
        Ok(slot
            .as_ref()
            .expect("slot always Some after refresh above")
            .token
            .as_str())
    }
}

/// Per-host failure tracking for exponential backoff
struct HostBackoff {
    consecutive_failures: u32,
    last_attempt: Instant,
}

/// Starts the pull-model scraper as a background task.
/// Reads target list from the `hosts` DB table and alert rules from `alert_configs` each cycle.
pub fn start_scraper(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let client = match Client::builder()
            .timeout(Duration::from_secs(SCRAPE_TIMEOUT_SECS))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(err = ?e, "❌ [Scraper] Failed to build HTTP client");
                return;
            }
        };

        let interval_secs = state.scrape_interval_secs;
        tracing::info!(interval = interval_secs, "🔍 [Scraper] Started (DB-driven)");

        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
        let _ = interval.tick().await; // skip first immediate tick

        let mut backoff_map: HashMap<String, HostBackoff> = HashMap::new();
        let mut jwt_cache: Option<JwtCache> = None;

        loop {
            interval.tick().await;
            scrape_all(&client, &state, &mut backoff_map, &mut jwt_cache).await;
        }
    })
}

async fn scrape_all(
    client: &Client,
    state: &Arc<AppState>,
    backoff_map: &mut HashMap<String, HostBackoff>,
    jwt_cache: &mut Option<JwtCache>,
) {
    // Read hosts + alert_configs from the in-memory snapshot instead of
    // hitting the DB every 10 s. The snapshot is refreshed synchronously
    // on every mutation handler (create/update/delete host, upsert/delete
    // alert config) and also by a 60 s background tick as a backstop.
    // Top-10 review finding #10.
    let snapshot = hosts_snapshot::load(&state.hosts_snapshot);
    let hosts = snapshot.hosts.clone();
    let alert_map = snapshot.alert_map.clone();

    // Pre-register any newly added hosts in last_known_status
    state.pre_populate_status(&hosts);

    // Mint (or reuse) a single agent JWT for this entire cycle — agents accept
    // any token that is unexpired, so all hosts share the same one.
    let jwt_token = match JwtCache::get_or_refresh(jwt_cache) {
        Ok(t) => t.to_string(),
        Err(e) => {
            tracing::error!(err = %e, "❌ [Scraper] Failed to mint agent JWT");
            return;
        }
    };

    let base_interval = Duration::from_secs(state.scrape_interval_secs);

    let futures = hosts
        .into_iter()
        .filter(|host| {
            // Skip hosts that are in backoff (consecutive failures → exponential wait)
            if let Some(backoff) = backoff_map.get(&host.host_key)
                && backoff.consecutive_failures > 0
            {
                let power = backoff.consecutive_failures.min(MAX_BACKOFF_POWER);
                let wait = base_interval * 2u32.pow(power);
                if backoff.last_attempt.elapsed() < wait {
                    return false;
                }
            }
            true
        })
        .map(|host| {
            let ctx = ScrapeContext {
                client: client.clone(),
                target: host.host_key.clone(),
                display_name: host.display_name.clone(),
                ports: host.ports.iter().map(|&p| p as u16).collect(),
                containers: host.containers.clone(),
                alert_config: alert_configs_repo::resolve_alert_config(
                    &host.host_key,
                    host.load_threshold,
                    &alert_map,
                ),
                state: state.clone(),
                jwt_token: jwt_token.clone(),
                system_info_updated_at: host.system_info_updated_at,
                is_known_host: true,
            };

            async move {
                let result = scrape_one(&ctx).await;
                (ctx.target, ctx.display_name, result)
            }
        });

    let results = stream::iter(futures)
        .buffer_unordered(10)
        .collect::<Vec<_>>()
        .await;

    // ── Collect persist data and update backoff tracking ──
    let mut success_count = 0;
    let mut fail_count = 0;
    let mut online_batch: Vec<(String, AgentMetrics)> = Vec::new();
    let mut offline_batch: Vec<(String, String)> = Vec::new();

    for (url, display_name, outcome) in results {
        match outcome {
            ScrapeOutcome::Online(metrics) => {
                success_count += 1;
                backoff_map.remove(&url);
                online_batch.push((url, *metrics));
            }
            ScrapeOutcome::Offline => {
                fail_count += 1;
                let entry = backoff_map.entry(url.clone()).or_insert(HostBackoff {
                    consecutive_failures: 0,
                    last_attempt: Instant::now(),
                });
                entry.consecutive_failures += 1;
                entry.last_attempt = Instant::now();
                offline_batch.push((url, display_name));
            }
            ScrapeOutcome::Failed(e) => {
                tracing::warn!(url = %url, error = %e, "🔴 [Scraper] Target failed (no DB insert)");
                fail_count += 1;
                let entry = backoff_map.entry(url).or_insert(HostBackoff {
                    consecutive_failures: 0,
                    last_attempt: Instant::now(),
                });
                entry.consecutive_failures += 1;
                entry.last_attempt = Instant::now();
            }
        }
    }

    // ── Batch DB persistence (single round-trip per type) ──
    if !online_batch.is_empty() {
        let batch_refs: Vec<(&str, &AgentMetrics)> = online_batch
            .iter()
            .map(|(hk, m)| (hk.as_str(), m))
            .collect();
        if let Err(e) = metrics_repo::insert_metrics_batch(&state.db_pool, &batch_refs).await {
            tracing::error!(err = ?e, count = online_batch.len(), "⚠️ [Scraper] Batch online INSERT failed");
        }
    }

    if !offline_batch.is_empty() {
        let batch_refs: Vec<(&str, &str)> = offline_batch
            .iter()
            .map(|(hk, dn)| (hk.as_str(), dn.as_str()))
            .collect();
        if let Err(e) =
            metrics_repo::insert_offline_metrics_batch(&state.db_pool, &batch_refs).await
        {
            tracing::error!(err = ?e, count = offline_batch.len(), "⚠️ [Scraper] Batch offline INSERT failed");
        }
    }

    if fail_count > 0 {
        tracing::info!(
            success = success_count,
            fail = fail_count,
            "📊 [Scraper Summary]"
        );
    }
}

/// Per-host scrape context — groups parameters that flow through
/// `scrape_one` → `handle_success` without long parameter lists.
struct ScrapeContext {
    client: Client,
    target: String,
    display_name: String,
    ports: Vec<u16>,
    containers: Vec<String>,
    alert_config: AlertConfig,
    state: Arc<AppState>,
    jwt_token: String,
    system_info_updated_at: Option<DateTime<Utc>>,
    is_known_host: bool,
}

async fn scrape_one(ctx: &ScrapeContext) -> ScrapeOutcome {
    let ports_str = ctx
        .ports
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let containers_str = ctx.containers.join(",");

    let mut url_str = format!("http://{}/metrics?", ctx.target);
    if !ports_str.is_empty() {
        url_str.push_str(&format!("ports={}&", ports_str));
    }
    if !containers_str.is_empty() {
        url_str.push_str(&format!("containers={}", containers_str));
    }

    match ctx
        .client
        .get(&url_str)
        .header("Authorization", format!("Bearer {}", ctx.jwt_token))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => match resp.bytes().await {
            Ok(bytes) if bytes.len() > 10 * 1024 * 1024 => {
                ScrapeOutcome::Failed(format!("Payload too large: {} bytes", bytes.len()))
            }
            Ok(bytes) => match bincode::deserialize::<AgentMetrics>(&bytes) {
                Ok(mut metrics) => {
                    // Defense-in-depth: cap untrusted Vec fields to sane maximums
                    metrics.cpu_cores.truncate(1024);
                    metrics.network_interfaces.truncate(256);
                    metrics.docker_stats.truncate(512);
                    metrics.system.processes.truncate(100);

                    if metrics.agent_version.is_empty() {
                        tracing::warn!(target = %ctx.target, "⚠️ [Scraper] Agent has no version field — consider upgrading");
                    } else if semver_less_than(&metrics.agent_version, MIN_AGENT_VERSION) {
                        tracing::warn!(
                            target = %ctx.target,
                            agent_version = %metrics.agent_version,
                            min_version = MIN_AGENT_VERSION,
                            server_version = SERVER_VERSION,
                            "⚠️ [Scraper] Agent version below minimum — consider upgrading"
                        );
                    }
                    handle_success(metrics, ctx).await
                }
                Err(e) => ScrapeOutcome::Failed(format!("Bincode deserialization error: {}", e)),
            },
            Err(e) => ScrapeOutcome::Failed(format!("Failed to read response body: {}", e)),
        },
        Ok(_resp) => {
            handle_down(&ctx.target, &ctx.display_name, &ctx.state).await;
            ScrapeOutcome::Offline
        }
        Err(_e) => {
            handle_down(&ctx.target, &ctx.display_name, &ctx.state).await;
            ScrapeOutcome::Offline
        }
    }
}

// ──────────────────────────────────────────────
// Success path
// ──────────────────────────────────────────────

/// System info refresh interval: 24 hours
const SYSTEM_INFO_REFRESH_SECS: i64 = 24 * 3600;

async fn handle_success(metrics: AgentMetrics, ctx: &ScrapeContext) -> ScrapeOutcome {
    // SP-01: Only call ensure_host_registered for unknown hosts — avoids N
    // unnecessary DB writes per scrape cycle for already-registered hosts.
    if !ctx.is_known_host
        && let Err(e) =
            hosts_repo::ensure_host_registered(&ctx.state.db_pool, &ctx.target, &metrics.hostname)
                .await
    {
        tracing::warn!(err = ?e, "⚠️ [Scraper] Failed to auto-register host");
    }

    match metrics_service::process_metrics(&metrics, &ctx.target, &ctx.state, &ctx.alert_config)
        .await
    {
        Ok(result) => {
            tracing::info!(target = %ctx.target, "✅ [Scraper] {}", result.log_msg);

            let _ = ctx
                .state
                .sse_tx
                .send(SseBroadcast::Metrics(result.metrics_payload));

            if let Some(status_payload) = result.status_payload {
                // SAFETY: no .await while lock is held
                if let Ok(mut lks) = ctx.state.last_known_status.write() {
                    lks.insert(ctx.target.clone(), status_payload.clone());
                }
                let _ = ctx.state.sse_tx.send(SseBroadcast::Status(status_payload));
            }
        }
        Err(e) => {
            tracing::error!(target = %ctx.target, err = ?e, "⚠️  [Scraper] process_metrics error");
            return ScrapeOutcome::Failed(format!("process_metrics error: {}", e));
        }
    }

    // Recovery (host back online) alert.
    //
    // Fast path: most cycles the host is already "online" (offline_alerted == false),
    // so we peek at the state under a read lock first and skip the write lock entirely.
    // Only when a transition is actually needed do we re-acquire as writer.
    let needs_recovery_check = {
        // SAFETY: no .await while lock is held
        match ctx.state.store.read() {
            Ok(store) => store
                .hosts
                .get(ctx.target.as_str())
                .is_some_and(|r| r.alert_state.offline_alerted),
            Err(e) => {
                tracing::warn!(err = %e, "⚠️ [Scraper] Store read lock poisoned in recovery check");
                return ScrapeOutcome::Online(Box::new(metrics));
            }
        }
    };

    let recovery_msg = if needs_recovery_check {
        // SAFETY: no .await while lock is held
        let mut store = match ctx.state.store.write() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(err = %e, "⚠️ [Scraper] Store write lock poisoned in recovery check");
                return ScrapeOutcome::Online(Box::new(metrics));
            }
        };
        let Some(record) = store.hosts.get_mut(ctx.target.as_str()) else {
            return ScrapeOutcome::Online(Box::new(metrics));
        };

        // Re-check under the write lock in case another task flipped the flag
        // between our read and write acquisitions.
        if record.alert_state.offline_alerted {
            let last_offline = record.alert_state.last_offline_alert;
            let cooldown_passed =
                last_offline.is_none_or(|t| t.elapsed() > Duration::from_secs(FLAP_COOLDOWN_SECS));

            if cooldown_passed {
                record.alert_state.offline_alerted = false;
                record.alert_state.last_recovery_alert = Some(Instant::now());
                Some(format!(
                    "✅ **[Host Recovery]** `{}` — agent is back online.",
                    metrics.hostname
                ))
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    if let Some(msg) = recovery_msg {
        alert_service::send_alert(&ctx.state.http_client, &ctx.state.db_pool, &msg).await;
        let _ = crate::repositories::alert_history_repo::insert_alert(
            &ctx.state.db_pool,
            &ctx.target,
            "host_recovery",
            &msg,
        )
        .await;
    }

    // ── System info fetch (on reconnection or stale > 24h) ──
    let was_offline = needs_recovery_check;
    let sys_info_stale = ctx.system_info_updated_at.is_none_or(|t| {
        Utc::now().signed_duration_since(t).num_seconds() > SYSTEM_INFO_REFRESH_SECS
    });

    if was_offline || sys_info_stale {
        let target_owned = ctx.target.clone();
        let client = ctx.client.clone();
        let jwt = ctx.jwt_token.clone();
        let state = Arc::clone(&ctx.state);
        tokio::spawn(async move {
            fetch_and_store_system_info(&client, &target_owned, &jwt, &state).await;
        });
    }

    ScrapeOutcome::Online(Box::new(metrics))
}

/// Fetch system info from the agent and persist to DB + in-memory status.
async fn fetch_and_store_system_info(
    client: &Client,
    target: &str,
    jwt_token: &str,
    state: &Arc<AppState>,
) {
    let url = format!("http://{}/system-info", target);
    let resp = match client
        .get(&url)
        .header("Authorization", format!("Bearer {}", jwt_token))
        .timeout(Duration::from_secs(SCRAPE_TIMEOUT_SECS))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            tracing::warn!(target = %target, status = %r.status(), "⚠️ [SystemInfo] Non-success response");
            return;
        }
        Err(e) => {
            tracing::warn!(target = %target, err = %e, "⚠️ [SystemInfo] Request failed (agent may not support /system-info)");
            return;
        }
    };

    let info: SystemInfoResponse = match resp.json().await {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!(target = %target, err = %e, "⚠️ [SystemInfo] JSON parse failed");
            return;
        }
    };

    // Persist to DB
    if let Err(e) = hosts_repo::update_system_info(
        &state.db_pool,
        target,
        &info.os,
        &info.cpu_model,
        info.memory_total_mb as i64,
        info.boot_time as i64,
        &info.ip_address,
    )
    .await
    {
        tracing::warn!(target = %target, err = %e, "⚠️ [SystemInfo] DB update failed");
        return;
    }

    // Update in-memory SSE status
    if let Ok(mut lks) = state.last_known_status.write()
        && let Some(status) = lks.get_mut(target)
    {
        status.os_info = Some(info.os);
        status.cpu_model = Some(info.cpu_model);
        status.memory_total_mb = Some(info.memory_total_mb as i64);
        status.boot_time = Some(info.boot_time as i64);
        status.ip_address = Some(info.ip_address);
    }

    tracing::info!(target = %target, "✅ [SystemInfo] Updated");
}

// ──────────────────────────────────────────────
// Failure path
// ──────────────────────────────────────────────

async fn handle_down(target: &str, display_name: &str, state: &Arc<AppState>) {
    let now = Instant::now();
    let host_key = target.to_string();

    // DB persistence is deferred — the caller (scrape_all) collects offline hosts
    // and batch-inserts them in a single query per scrape cycle.

    // ── Phase 1: store write lock (lightweight — alert state only) ──
    let (alert_msg, hostname, should_broadcast) = {
        let mut store = match state.store.write() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(err = %e, "⚠️ [Scraper] Store write lock poisoned in handle_down");
                return;
            }
        };

        let hostname = store
            .hosts
            .get(target)
            .map(|r| r.last_known_hostname.clone())
            .unwrap_or_else(|| display_name.to_string());

        let record = store
            .hosts
            .entry(target.to_string())
            .or_insert_with(|| HostRecord::new(hostname.clone()));

        // Throttle status broadcasts for offline hosts — same pattern as handle_success().
        // Without this, N offline hosts generate N unnecessary SSE events every scrape cycle.
        let periodic_elapsed = record
            .last_status_sent
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(STATUS_PERIODIC_INTERVAL_SECS));
        if periodic_elapsed {
            record.last_status_sent = Some(now);
        }

        let alert = if record.alert_state.offline_alerted {
            None
        } else {
            let last_recovery = record.alert_state.last_recovery_alert;
            let cooldown_passed =
                last_recovery.is_none_or(|t| t.elapsed() > Duration::from_secs(FLAP_COOLDOWN_SECS));

            if cooldown_passed {
                record.alert_state.offline_alerted = true;
                record.alert_state.last_offline_alert = Some(now);
                Some(format!(
                    "🔴 **[Host Down]** `{}` (target: `{}`) — no response",
                    hostname, target
                ))
            } else {
                None
            }
        };

        // Broadcast on first offline (alert fired) or periodic interval
        let should_broadcast = alert.is_some() || periodic_elapsed;

        (alert, hostname, should_broadcast)
        // ← store write lock released here
    };

    // ── Phase 2: last_known_status update + SSE broadcast (no store lock held) ──
    if should_broadcast {
        let server_ts = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

        if let Ok(mut lks) = state.last_known_status.write() {
            let status = lks
                .entry(host_key.clone())
                .or_insert_with(|| HostStatusPayload {
                    host_key: host_key.clone(),
                    display_name: hostname.clone(),
                    is_online: false,
                    last_seen: String::new(),
                    docker_containers: vec![],
                    ports: vec![],
                    disks: vec![],
                    processes: vec![],
                    temperatures: vec![],
                    gpus: vec![],
                    docker_stats: vec![],
                    os_info: None,
                    cpu_model: None,
                    memory_total_mb: None,
                    boot_time: None,
                    ip_address: None,
                });
            // Update only the fields that change — reuse existing Vec data (no clone)
            status.is_online = false;
            status.last_seen = server_ts;
            status.processes = vec![];

            let _ = state.sse_tx.send(SseBroadcast::Status(status.clone()));
        }
    }

    // ── Phase 3: alert delivery (async I/O, no locks held) ──
    if let Some(msg) = alert_msg {
        alert_service::send_alert(&state.http_client, &state.db_pool, &msg).await;
        let _ = crate::repositories::alert_history_repo::insert_alert(
            &state.db_pool,
            &host_key,
            "host_down",
            &msg,
        )
        .await;
    }
}
