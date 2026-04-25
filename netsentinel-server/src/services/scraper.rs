use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{SecondsFormat, Utc};
use futures::StreamExt;
use futures::stream;
use reqwest::Client;

use crate::models::agent_metrics::{AgentMetrics, SystemInfoResponse, deserialize_agent_metrics};
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
/// Cap the decoded agent response body before deserialization.
const MAX_AGENT_PAYLOAD_BYTES: usize = 10 * 1024 * 1024;

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

        let default_interval_secs = state.scrape_interval_secs;
        tracing::info!(
            default_interval = default_interval_secs,
            scheduler_resolution = 1,
            "🔍 [Scraper] Started (DB-driven)"
        );

        let mut interval = tokio::time::interval(Duration::from_secs(1));
        let _ = interval.tick().await; // skip first immediate tick

        let mut backoff_map: HashMap<String, HostBackoff> = HashMap::new();
        let mut last_scrape_attempt: HashMap<String, Instant> = HashMap::new();
        let mut jwt_cache: Option<JwtCache> = None;

        loop {
            interval.tick().await;
            scrape_all(
                &client,
                &state,
                &mut backoff_map,
                &mut last_scrape_attempt,
                &mut jwt_cache,
            )
            .await;
        }
    })
}

async fn scrape_all(
    client: &Client,
    state: &Arc<AppState>,
    backoff_map: &mut HashMap<String, HostBackoff>,
    last_scrape_attempt: &mut HashMap<String, Instant>,
    jwt_cache: &mut Option<JwtCache>,
) {
    // Read hosts + alert_configs from the in-memory snapshot instead of
    // hitting the DB every 10 s. The snapshot is refreshed synchronously
    // on every mutation handler (create/update/delete host, upsert/delete
    // alert config) and also by a 60 s background tick as a backstop.
    // Top-10 review finding #10.
    let snapshot = hosts_snapshot::load(&state.hosts_snapshot);

    // Pre-register any newly added hosts in last_known_status
    state.pre_populate_status(&snapshot.hosts);

    // Mint (or reuse) a single agent JWT for this entire cycle — agents accept
    // any token that is unexpired, so all hosts share the same one.
    let jwt_token = match JwtCache::get_or_refresh(jwt_cache) {
        Ok(t) => t.to_string(),
        Err(e) => {
            tracing::error!(err = %e, "❌ [Scraper] Failed to mint agent JWT");
            return;
        }
    };

    last_scrape_attempt
        .retain(|host_key, _| snapshot.hosts.iter().any(|host| host.host_key == *host_key));
    backoff_map.retain(|host_key, _| snapshot.hosts.iter().any(|host| host.host_key == *host_key));

    let mut due_contexts = Vec::new();
    for host in &snapshot.hosts {
        let scrape_interval_secs = u64::try_from(host.scrape_interval_secs)
            .ok()
            .filter(|secs| *secs > 0)
            .unwrap_or(state.scrape_interval_secs);
        let host_interval = Duration::from_secs(scrape_interval_secs);

        // Slack on the "is host due?" check to absorb 1-Hz scheduler
        // jitter. Without it the cadence drifts by exactly one tick
        // (1 s) per scrape: the outer loop tick fires at T = 0, 1, 2, …
        // but `last_attempt` is stamped via `Instant::now()` *after*
        // tick fire (T = 0 + ε). At T = host_interval the elapsed comes
        // out as `host_interval − ε`, which is `< host_interval`, so the
        // comparison treats the host as "not yet due" and we skip until
        // T = host_interval + 1. That turned a configured 10 s scrape
        // into an effective ~11 s SSE cadence. 500 ms is generous
        // enough for any realistic clock jitter while still firing
        // strictly before the next interval boundary.
        const SCHEDULER_SLACK: Duration = Duration::from_millis(500);
        if let Some(last_attempt) = last_scrape_attempt.get(&host.host_key)
            && last_attempt.elapsed() + SCHEDULER_SLACK < host_interval
        {
            continue;
        }

        if let Some(backoff) = backoff_map.get(&host.host_key)
            && backoff.consecutive_failures > 0
        {
            let power = backoff.consecutive_failures.min(MAX_BACKOFF_POWER);
            let wait = host_interval * 2u32.pow(power);
            // Same `SCHEDULER_SLACK` rationale as above — the backoff
            // wait is wall-clock derived but checked at 1-Hz tick
            // resolution, so without slack a `wait = 10 s` would also
            // drift by one tick per cycle.
            if backoff.last_attempt.elapsed() + SCHEDULER_SLACK < wait {
                continue;
            }
        }

        last_scrape_attempt.insert(host.host_key.clone(), Instant::now());
        due_contexts.push(ScrapeContext {
            client: client.clone(),
            target: host.host_key.clone(),
            display_name: host.display_name.clone(),
            ports: host.ports.iter().map(|&p| p as u16).collect(),
            containers: host.containers.clone(),
            alert_config: alert_configs_repo::resolve_alert_config(
                &host.host_key,
                host.load_threshold,
                &snapshot.alert_map,
            ),
            state: state.clone(),
            jwt_token: jwt_token.clone(),
            system_info_updated_at: host.system_info_updated_at,
            scrape_interval_secs,
        });
    }

    let results = stream::iter(due_contexts.into_iter().map(|ctx| async move {
        let target = ctx.target.clone();
        let display_name = ctx.display_name.clone();
        let result = scrape_one(&ctx).await;
        (target, display_name, result)
    }))
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
    scrape_interval_secs: u64,
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
        Ok(resp) if resp.status().is_success() => {
            let mut bytes = Vec::new();
            let mut resp = resp;
            loop {
                match resp.chunk().await {
                    Ok(Some(chunk)) => {
                        let next_len = bytes.len().saturating_add(chunk.len());
                        if next_len > MAX_AGENT_PAYLOAD_BYTES {
                            return ScrapeOutcome::Failed(format!(
                                "Payload too large: exceeds {} bytes",
                                MAX_AGENT_PAYLOAD_BYTES
                            ));
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                    Ok(None) => break,
                    Err(e) => {
                        return ScrapeOutcome::Failed(format!(
                            "Failed to read response body chunk: {}",
                            e
                        ));
                    }
                }
            }

            match deserialize_agent_metrics(&bytes) {
                Ok(mut metrics) => {
                    // Defense-in-depth: cap untrusted Vec fields to sane maximums
                    metrics.cpu_cores.truncate(1024);
                    metrics.network_interfaces.truncate(256);
                    metrics.docker_stats.truncate(512);
                    metrics.system.processes.truncate(100);

                    sanitize_metrics(&mut metrics);

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
            }
        }
        Ok(_resp) => {
            handle_down(
                &ctx.target,
                &ctx.display_name,
                ctx.scrape_interval_secs,
                &ctx.state,
            )
            .await;
            ScrapeOutcome::Offline
        }
        Err(_e) => {
            handle_down(
                &ctx.target,
                &ctx.display_name,
                ctx.scrape_interval_secs,
                &ctx.state,
            )
            .await;
            ScrapeOutcome::Offline
        }
    }
}

fn sanitize_metrics(metrics: &mut AgentMetrics) {
    metrics.system.cpu_usage_percent =
        metrics_service::sanitize_f32(metrics.system.cpu_usage_percent);
    metrics.system.memory_usage_percent =
        metrics_service::sanitize_f32(metrics.system.memory_usage_percent);

    metrics.load_average.one_min = metrics_service::sanitize_f64(metrics.load_average.one_min);
    metrics.load_average.five_min = metrics_service::sanitize_f64(metrics.load_average.five_min);
    metrics.load_average.fifteen_min =
        metrics_service::sanitize_f64(metrics.load_average.fifteen_min);

    metrics.network.rx_bytes_per_sec =
        metrics_service::sanitize_f64(metrics.network.rx_bytes_per_sec);
    metrics.network.tx_bytes_per_sec =
        metrics_service::sanitize_f64(metrics.network.tx_bytes_per_sec);

    for disk in &mut metrics.system.disks {
        disk.usage_percent = metrics_service::sanitize_f32(disk.usage_percent);
        disk.read_bytes_per_sec = metrics_service::sanitize_f64(disk.read_bytes_per_sec);
        disk.write_bytes_per_sec = metrics_service::sanitize_f64(disk.write_bytes_per_sec);
        disk.total_gb = metrics_service::sanitize_f64(disk.total_gb);
        disk.available_gb = metrics_service::sanitize_f64(disk.available_gb);
    }

    for core in &mut metrics.cpu_cores {
        *core = metrics_service::sanitize_f32(*core);
    }

    for temperature in &mut metrics.system.temperatures {
        temperature.temperature_c = metrics_service::sanitize_f32(temperature.temperature_c);
    }

    for gpu in &mut metrics.system.gpus {
        if let Some(power_watts) = gpu.power_watts {
            gpu.power_watts = Some(metrics_service::sanitize_f32(power_watts));
        }
    }

    for stats in &mut metrics.docker_stats {
        stats.cpu_percent = metrics_service::sanitize_f32(stats.cpu_percent);
    }

    for process in &mut metrics.system.processes {
        process.cpu_usage = metrics_service::sanitize_f32(process.cpu_usage);
    }
}

// ──────────────────────────────────────────────
// Success path
// ──────────────────────────────────────────────

/// System info refresh interval: 24 hours
const SYSTEM_INFO_REFRESH_SECS: i64 = 24 * 3600;

async fn handle_success(mut metrics: AgentMetrics, ctx: &ScrapeContext) -> ScrapeOutcome {
    match metrics_service::process_metrics(
        &metrics,
        &ctx.target,
        &ctx.state,
        &ctx.alert_config,
        ctx.scrape_interval_secs,
    )
    .await
    {
        Ok(result) => {
            tracing::info!(target = %ctx.target, "✅ [Scraper] {}", result.log_msg);

            metrics.network.rx_bytes_per_sec = result.metrics_payload.network_rate.rx_bytes_per_sec;
            metrics.network.tx_bytes_per_sec = result.metrics_payload.network_rate.tx_bytes_per_sec;

            // Wrap once so every subscriber gets an `Arc::clone` via the
            // broadcast fan-out rather than a full `HostMetricsPayload` clone
            // (see `SseBroadcast` docs).
            let _ = ctx
                .state
                .sse_tx
                .send(SseBroadcast::Metrics(Arc::new(result.metrics_payload)));

            if let Some(status_payload) = result.status_payload {
                let arc = Arc::new(status_payload);
                // SAFETY: no .await while lock is held
                if let Ok(mut lks) = ctx.state.last_known_status.write() {
                    lks.insert(ctx.target.clone(), Arc::clone(&arc));
                }
                let _ = ctx.state.sse_tx.send(SseBroadcast::Status(arc));
            }
        }
        Err(e) => {
            tracing::error!(target = %ctx.target, err = ?e, "⚠️  [Scraper] process_metrics error");
            return ScrapeOutcome::Failed(format!("process_metrics error: {}", e));
        }
    }

    // Recovery (host back online) alert.
    //
    // Single write-lock acquisition: previously this path peeked under a
    // read lock to decide whether a transition was pending, then reopened
    // as writer when one was. That two-step pattern let another task slip
    // between the read and the write — which we then had to re-check —
    // and it doubled lock entries on the hot path for zero latency win:
    // the write guard itself is cheap, and the recovery branch is rare.
    // Taking write once up front eliminates the TOCTOU and one whole
    // `store` critical section per scrape cycle.
    let recovery_msg = {
        // SAFETY: no .await while lock is held
        let mut store = match ctx.state.store.write() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(err = %e, "⚠️ [Scraper] Store write lock poisoned in recovery check");
                return ScrapeOutcome::Online(Box::new(metrics));
            }
        };
        match store.hosts.get_mut(ctx.target.as_str()) {
            Some(record) => {
                mark_recovery_if_cooldown_passed(record, &metrics.hostname, Instant::now())
            }
            None => None,
        }
    };
    let was_offline = recovery_msg.is_some();

    // Recovery alert: fan out to webhooks + alert_history write on a detached
    // task. `send_alert` can spend hundreds of ms on external HTTP, and the
    // caller is inside the scraper's `buffer_unordered(10)` stream — blocking
    // here steals a concurrency slot for the remainder of the cycle.
    if let Some(msg) = recovery_msg {
        let http = ctx.state.http_client.clone();
        let pool = ctx.state.db_pool.clone();
        let target_owned = ctx.target.clone();
        tokio::spawn(async move {
            alert_service::send_alert(&http, &pool, &msg).await;
            if let Err(e) = crate::repositories::alert_history_repo::insert_alert(
                &pool,
                &target_owned,
                "host_recovery",
                &msg,
            )
            .await
            {
                tracing::error!(err = ?e, "⚠️ [AlertHistory] Failed to log host_recovery");
            }
        });
    }

    // ── System info fetch (on reconnection or stale > 24h) ──
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

fn mark_recovery_if_cooldown_passed(
    record: &mut HostRecord,
    hostname: &str,
    now: Instant,
) -> Option<String> {
    if !record.alert_state.offline_alerted {
        return None;
    }

    let cooldown_passed = record
        .alert_state
        .last_recovery_alert
        .is_none_or(|t| now.duration_since(t) > Duration::from_secs(FLAP_COOLDOWN_SECS));

    if !cooldown_passed {
        return None;
    }

    record.alert_state.offline_alerted = false;
    record.alert_state.last_recovery_alert = Some(now);
    Some(format!(
        "✅ **[Host Recovery]** `{hostname}` — agent is back online."
    ))
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
        && let Some(arc) = lks.get_mut(target)
    {
        let status = Arc::make_mut(arc);
        status.os_info = Some(info.os.clone());
        status.cpu_model = Some(info.cpu_model.clone());
        status.memory_total_mb = Some(info.memory_total_mb as i64);
        status.boot_time = Some(info.boot_time as i64);
        status.ip_address = Some(info.ip_address.clone());
    }

    hosts_snapshot::apply_system_info(&state.hosts_snapshot, target, &info);

    tracing::info!(target = %target, "✅ [SystemInfo] Updated");
}

// ──────────────────────────────────────────────
// Failure path
// ──────────────────────────────────────────────

async fn handle_down(
    target: &str,
    display_name: &str,
    scrape_interval_secs: u64,
    state: &Arc<AppState>,
) {
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
            let last_offline = record.alert_state.last_offline_alert;
            let cooldown_passed =
                last_offline.is_none_or(|t| t.elapsed() > Duration::from_secs(FLAP_COOLDOWN_SECS));

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
            let arc = lks.entry(host_key.clone()).or_insert_with(|| {
                Arc::new(HostStatusPayload {
                    host_key: host_key.clone(),
                    display_name: hostname.clone(),
                    scrape_interval_secs,
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
                })
            });
            // `Arc::make_mut` is cheap (no-op) while the Arc is uniquely owned —
            // the common case when no SSE subscriber is currently holding the
            // previous broadcast. It clones only when a slow consumer still
            // references the prior payload, which is exactly when we need to
            // avoid mutating a value other tasks are reading.
            let status = Arc::make_mut(arc);
            status.scrape_interval_secs = scrape_interval_secs;
            status.is_online = false;
            status.last_seen = server_ts;
            status.processes = vec![];
            let broadcast_arc = Arc::clone(arc);

            let _ = state.sse_tx.send(SseBroadcast::Status(broadcast_arc));
        }
    }

    // ── Phase 3: alert delivery (async I/O, no locks held) ──
    // Fire-and-forget: same rationale as the recovery path — webhook latency
    // should not be charged against the scraper's concurrency budget.
    if let Some(msg) = alert_msg {
        let http = state.http_client.clone();
        let pool = state.db_pool.clone();
        let hk = host_key.clone();
        tokio::spawn(async move {
            alert_service::send_alert(&http, &pool, &msg).await;
            if let Err(e) =
                crate::repositories::alert_history_repo::insert_alert(&pool, &hk, "host_down", &msg)
                    .await
            {
                tracing::error!(err = ?e, "⚠️ [AlertHistory] Failed to log host_down");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_cooldown_uses_last_recovery_alert() {
        let now = Instant::now();
        let mut record = HostRecord::new("test-host".to_string());
        record.alert_state.offline_alerted = true;
        record.alert_state.last_offline_alert = Some(now);

        let first = mark_recovery_if_cooldown_passed(&mut record, "test-host", now);
        assert!(first.is_some(), "first recovery after host down must send");
        assert!(!record.alert_state.offline_alerted);
        assert_eq!(record.alert_state.last_recovery_alert, Some(now));

        record.alert_state.offline_alerted = true;
        let duplicate = mark_recovery_if_cooldown_passed(
            &mut record,
            "test-host",
            now + Duration::from_secs(30),
        );
        assert!(
            duplicate.is_none(),
            "second recovery inside cooldown must be suppressed"
        );
        assert!(record.alert_state.offline_alerted);

        let after_cooldown = mark_recovery_if_cooldown_passed(
            &mut record,
            "test-host",
            now + Duration::from_secs(FLAP_COOLDOWN_SECS + 1),
        );
        assert!(
            after_cooldown.is_some(),
            "recovery after cooldown must send again"
        );
        assert!(!record.alert_state.offline_alerted);
    }
}
