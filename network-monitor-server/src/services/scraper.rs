use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{SecondsFormat, Utc};
use futures::stream::{self, StreamExt};
use reqwest::Client;

use crate::models::agent_metrics::AgentMetrics;
use crate::models::app_state::{AlertConfig, AppState, HostRecord};
use crate::models::sse_payloads::{HostStatusPayload, SseBroadcast};
use crate::repositories::{alert_configs_repo, hosts_repo};
use crate::services::{alert_service, metrics_service};

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

/// Per-host failure tracking for exponential backoff
struct HostBackoff {
    consecutive_failures: u32,
    last_attempt: Instant,
}

/// Starts the pull-model scraper as a background task.
/// Reads target list from the `hosts` DB table and alert rules from `alert_configs` each cycle.
pub fn start_scraper(state: Arc<AppState>) {
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

        loop {
            interval.tick().await;
            scrape_all(&client, &state, &mut backoff_map).await;
        }
    });
}

async fn scrape_all(
    client: &Client,
    state: &Arc<AppState>,
    backoff_map: &mut HashMap<String, HostBackoff>,
) {
    // Reload the latest host list and alert configs from DB each cycle
    let hosts = match hosts_repo::list_hosts(&state.db_pool).await {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(err = ?e, "❌ [Scraper] Failed to load hosts from DB");
            return;
        }
    };

    let alert_map = alert_configs_repo::load_all_as_map(&state.db_pool)
        .await
        .unwrap_or_default();

    // Pre-register any newly added hosts in last_known_status
    state.pre_populate_status(&hosts);

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

            async move {
                let url = host.host_key.clone();
                let result = scrape_one(
                    &client,
                    &host.host_key,
                    &host.display_name,
                    &ports,
                    &containers,
                    &alert_config,
                    &state,
                )
                .await;
                (url, result)
            }
        });

    let results = stream::iter(futures)
        .buffer_unordered(10)
        .collect::<Vec<_>>()
        .await;

    let mut success_count = 0;
    let mut fail_count = 0;

    for (url, res) in &results {
        match res {
            Ok(_) => {
                success_count += 1;
                backoff_map.remove(url);
            }
            Err(e) => {
                tracing::warn!(url = %url, error = %e, "🔴 [Scraper] Target failed");
                fail_count += 1;
                let entry = backoff_map.entry(url.clone()).or_insert(HostBackoff {
                    consecutive_failures: 0,
                    last_attempt: Instant::now(),
                });
                entry.consecutive_failures += 1;
                entry.last_attempt = Instant::now();
            }
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

async fn scrape_one(
    client: &Client,
    target: &str,
    display_name: &str,
    ports: &[u16],
    containers: &[String],
    alert_config: &AlertConfig,
    state: &Arc<AppState>,
) -> Result<(), String> {
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

    let jwt_token = match crate::services::auth::generate_jwt() {
        Ok(t) => t,
        Err(e) => return Err(format!("JWT Generation Error: {}", e)),
    };

    match client
        .get(&url_str)
        .header("Authorization", format!("Bearer {}", jwt_token))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => match resp.bytes().await {
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
                    handle_success(metrics, target, alert_config, state).await;
                    Ok(())
                }
                Err(e) => Err(format!("Bincode deserialization error: {}", e)),
            },
            Err(e) => Err(format!("Failed to read response body: {}", e)),
        },
        Ok(resp) => {
            handle_down(target, display_name, state).await;
            Err(format!("Bad HTTP status: {}", resp.status()))
        }
        Err(e) => {
            handle_down(target, display_name, state).await;
            Err(format!("Unreachable: {}", e))
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
) {
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
            tracing::error!(target = %target, err = ?e, "⚠️  [Scraper] process_metrics error")
        }
    }

    // Recovery (host back online) alert
    let recovery_msg = {
        let mut store = match state.store.write() {
            Ok(s) => s,
            Err(_) => return,
        };
        let Some(record) = store.hosts.get_mut(target) else {
            return;
        };

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
}

// ──────────────────────────────────────────────
// Failure path
// ──────────────────────────────────────────────

async fn handle_down(target: &str, display_name: &str, state: &Arc<AppState>) {
    let now = Instant::now();
    let host_key = target.to_string();

    let alert_msg = {
        let mut store = match state.store.write() {
            Ok(s) => s,
            Err(_) => return,
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

        let server_ts = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let offline_status = HostStatusPayload {
            host_key: host_key.clone(),
            display_name: hostname.clone(),
            is_online: false,
            last_seen: server_ts,
            docker_containers: state
                .last_known_status
                .read()
                .ok()
                .and_then(|lks| lks.get(&host_key).map(|s| s.docker_containers.clone()))
                .unwrap_or_default(),
            ports: state
                .last_known_status
                .read()
                .ok()
                .and_then(|lks| lks.get(&host_key).map(|s| s.ports.clone()))
                .unwrap_or_default(),
            disks: state
                .last_known_status
                .read()
                .ok()
                .and_then(|lks| lks.get(&host_key).map(|s| s.disks.clone()))
                .unwrap_or_default(),
            processes: vec![],
            temperatures: state
                .last_known_status
                .read()
                .ok()
                .and_then(|lks| lks.get(&host_key).map(|s| s.temperatures.clone()))
                .unwrap_or_default(),
            gpus: state
                .last_known_status
                .read()
                .ok()
                .and_then(|lks| lks.get(&host_key).map(|s| s.gpus.clone()))
                .unwrap_or_default(),
        };

        if let Ok(mut lks) = state.last_known_status.write() {
            lks.insert(host_key.clone(), offline_status.clone());
        }
        let _ = state.sse_tx.send(SseBroadcast::Status(offline_status));

        if record.alert_state.offline_alerted {
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
        }
    };

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
