use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{SecondsFormat, Utc};
use futures::stream::{self, StreamExt};
use reqwest::Client;

use crate::models::agent_metrics::AgentMetrics;
use crate::models::app_state::{AlertConfig, AppState, HostRecord};
use crate::models::sse_payloads::{HostStatusPayload, SseBroadcast};
use crate::repositories::{alert_configs_repo, hosts_repo, metrics_repo};
use crate::services::alert_service;
use crate::services::metrics_service::{self, STATUS_PERIODIC_INTERVAL_SECS};

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
    // Reload the latest host list and alert configs from DB each cycle
    let hosts = match hosts_repo::list_hosts(&state.db_pool).await {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(err = ?e, "❌ [Scraper] Failed to load hosts from DB");
            return;
        }
    };

    let alert_map = match alert_configs_repo::load_all_as_map(&state.db_pool).await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(err = ?e, "⚠️ [Scraper] Failed to load alert configs, using defaults");
            std::collections::HashMap::new()
        }
    };

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
            let client = client.clone();
            let state = state.clone();
            let alert_config = alert_configs_repo::resolve_alert_config(
                &host.host_key,
                host.load_threshold,
                &alert_map,
            );
            let ports: Vec<u16> = host.ports.iter().map(|&p| p as u16).collect();
            let containers = host.containers.clone();
            let jwt_token = jwt_token.clone();

            async move {
                let url = host.host_key.clone();
                let dn = host.display_name.clone();
                let result = scrape_one(
                    &client,
                    &host.host_key,
                    &host.display_name,
                    &ports,
                    &containers,
                    &alert_config,
                    &state,
                    &jwt_token,
                )
                .await;
                (url, dn, result)
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

#[allow(clippy::too_many_arguments)]
async fn scrape_one(
    client: &Client,
    target: &str,
    display_name: &str,
    ports: &[u16],
    containers: &[String],
    alert_config: &AlertConfig,
    state: &Arc<AppState>,
    jwt_token: &str,
) -> ScrapeOutcome {
    let ports_str = ports
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let containers_str = containers.join(",");

    let mut url_str = format!("http://{}/metrics?", target);
    if !ports_str.is_empty() {
        url_str.push_str(&format!("ports={}&", ports_str));
    }
    if !containers_str.is_empty() {
        url_str.push_str(&format!("containers={}", containers_str));
    }

    match client
        .get(&url_str)
        .header("Authorization", format!("Bearer {}", jwt_token))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => match resp.bytes().await {
            Ok(bytes) if bytes.len() > 10 * 1024 * 1024 => {
                ScrapeOutcome::Failed(format!("Payload too large: {} bytes", bytes.len()))
            }
            Ok(bytes) => match bincode::deserialize::<AgentMetrics>(&bytes) {
                Ok(metrics) => {
                    if metrics.agent_version.is_empty() {
                        tracing::warn!(target = %target, "⚠️ [Scraper] Agent has no version field — consider upgrading");
                    } else if metrics.agent_version.as_str() < MIN_AGENT_VERSION {
                        tracing::warn!(
                            target = %target,
                            agent_version = %metrics.agent_version,
                            min_version = MIN_AGENT_VERSION,
                            server_version = SERVER_VERSION,
                            "⚠️ [Scraper] Agent version below minimum — consider upgrading"
                        );
                    }
                    handle_success(metrics, target, alert_config, state).await
                }
                Err(e) => ScrapeOutcome::Failed(format!("Bincode deserialization error: {}", e)),
            },
            Err(e) => ScrapeOutcome::Failed(format!("Failed to read response body: {}", e)),
        },
        Ok(_resp) => {
            handle_down(target, display_name, state).await;
            ScrapeOutcome::Offline
        }
        Err(_e) => {
            handle_down(target, display_name, state).await;
            ScrapeOutcome::Offline
        }
    }
}

// ──────────────────────────────────────────────
// Success path
// ──────────────────────────────────────────────

async fn handle_success(
    metrics: AgentMetrics,
    target: &str,
    alert_config: &AlertConfig,
    state: &Arc<AppState>,
) -> ScrapeOutcome {
    // Auto-register host and update display_name if needed
    if let Err(e) =
        hosts_repo::ensure_host_registered(&state.db_pool, target, &metrics.hostname).await
    {
        tracing::warn!(err = ?e, "⚠️ [Scraper] Failed to auto-register host");
    }

    match metrics_service::process_metrics(&metrics, target, state, alert_config).await {
        Ok(result) => {
            tracing::info!(target = %target, "✅ [Scraper] {}", result.log_msg);

            let _ = state
                .sse_tx
                .send(SseBroadcast::Metrics(result.metrics_payload));

            if let Some(status_payload) = result.status_payload {
                if let Ok(mut lks) = state.last_known_status.write() {
                    lks.insert(target.to_string(), status_payload.clone());
                }
                let _ = state.sse_tx.send(SseBroadcast::Status(status_payload));
            }
        }
        Err(e) => {
            tracing::error!(target = %target, err = ?e, "⚠️  [Scraper] process_metrics error");
            return ScrapeOutcome::Failed(format!("process_metrics error: {}", e));
        }
    }

    // Recovery (host back online) alert.
    //
    // Fast path: most cycles the host is already "online" (offline_alerted == false),
    // so we peek at the state under a read lock first and skip the write lock entirely.
    // Only when a transition is actually needed do we re-acquire as writer.
    let needs_recovery_check = {
        match state.store.read() {
            Ok(store) => store
                .hosts
                .get(target)
                .is_some_and(|r| r.alert_state.offline_alerted),
            Err(e) => {
                tracing::warn!(err = %e, "⚠️ [Scraper] Store read lock poisoned in recovery check");
                return ScrapeOutcome::Online(Box::new(metrics));
            }
        }
    };

    let recovery_msg = if needs_recovery_check {
        let mut store = match state.store.write() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(err = %e, "⚠️ [Scraper] Store write lock poisoned in recovery check");
                return ScrapeOutcome::Online(Box::new(metrics));
            }
        };
        let Some(record) = store.hosts.get_mut(target) else {
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
        alert_service::send_alert(&state.http_client, &state.db_pool, &msg).await;
        let _ = crate::repositories::alert_history_repo::insert_alert(
            &state.db_pool,
            target,
            "host_recovery",
            &msg,
        )
        .await;
    }

    ScrapeOutcome::Online(Box::new(metrics))
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
