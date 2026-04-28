use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::stream::{self, StreamExt};
use tokio::time::Instant;

use crate::models::app_state::AppState;
use crate::repositories::{alert_history_repo, http_monitors_repo, ping_monitors_repo};
use crate::services::alert_service;
use crate::services::monitors_snapshot;

/// Minimum scrape interval to prevent excessive polling
const MIN_INTERVAL_SECS: u64 = 10;
/// Alert cooldown per monitor to prevent spam (seconds)
const MONITOR_ALERT_COOLDOWN_SECS: u64 = 300;
/// Upper bound on concurrent monitor executions per sweep.
const MONITOR_CONCURRENCY: usize = 16;

/// Per-monitor alert state tracking
struct MonitorAlertState {
    is_failing: bool,
    last_alert: Option<Instant>,
}

/// Start the HTTP and Ping monitor scraper as a background task.
/// Runs every 10 seconds — each monitor tracks its own interval via last_checked timestamps.
///
/// Uses `tokio::time::interval` (not `sleep`) with `MissedTickBehavior::Delay`
/// so a slow sweep does not push the next sweep further out — a sweep that
/// exceeds the period is just absorbed and the loop catches up on the next
/// tick. Matches the pattern used by `scraper`, `rollup_worker`, and
/// `retention_worker`.
///
/// Reads enabled monitors from `monitors_snapshot` (Top-10 review #9) instead
/// of issuing two `SELECT … WHERE enabled = 1` queries per sweep. Mutation
/// handlers refresh the snapshot synchronously, and a 60 s background tick
/// catches any races.
pub fn spawn_monitor_scraper(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Track last check time per monitor
        let mut http_last_checked: HashMap<i32, Instant> = HashMap::new();
        let mut ping_last_checked: HashMap<i32, Instant> = HashMap::new();
        // Track alert state per monitor (keyed by "http:{id}" or "ping:{id}")
        let mut alert_state: HashMap<String, MonitorAlertState> = HashMap::new();

        let mut ticker = tokio::time::interval(Duration::from_secs(MIN_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // First tick fires immediately; skip it so callers that already
        // seeded snapshots / state at startup do not see a redundant sweep
        // before the snapshot's synchronous DB load completes.
        ticker.tick().await;

        loop {
            ticker.tick().await;

            let snapshot = monitors_snapshot::load(&state.db_pool, &state.monitors_snapshot);

            // HTTP monitors
            {
                let monitors = snapshot.http.clone();
                let live_ids: std::collections::HashSet<i32> =
                    monitors.iter().map(|monitor| monitor.id).collect();
                http_last_checked.retain(|id, _| live_ids.contains(id));
                alert_state.retain(|key, _| {
                    key.strip_prefix("http:")
                        .and_then(|id| id.parse::<i32>().ok())
                        .is_none_or(|id| live_ids.contains(&id))
                });

                let due_monitors: Vec<_> = monitors
                    .into_iter()
                    .filter(|monitor| {
                        let interval = Duration::from_secs(monitor.interval_secs.max(10) as u64);
                        http_last_checked
                            .get(&monitor.id)
                            .is_none_or(|t| t.elapsed() >= interval)
                    })
                    .collect();

                for monitor in &due_monitors {
                    http_last_checked.insert(monitor.id, Instant::now());
                }

                let results: Vec<_> = stream::iter(due_monitors.into_iter().map(|monitor| {
                    let pool = state.db_pool.clone();
                    let client = state.http_client.clone();
                    async move {
                        let error = check_http_endpoint(&pool, &client, &monitor).await;
                        (monitor, error)
                    }
                }))
                .buffer_unordered(MONITOR_CONCURRENCY)
                .collect()
                .await;

                for (monitor, error) in results {
                    handle_monitor_alert(
                        &state,
                        &mut alert_state,
                        &format!("http:{}", monitor.id),
                        &monitor.name,
                        error,
                    )
                    .await;
                }
            }

            // Ping monitors
            {
                let monitors = snapshot.ping.clone();
                let live_ids: std::collections::HashSet<i32> =
                    monitors.iter().map(|monitor| monitor.id).collect();
                ping_last_checked.retain(|id, _| live_ids.contains(id));
                alert_state.retain(|key, _| {
                    key.strip_prefix("ping:")
                        .and_then(|id| id.parse::<i32>().ok())
                        .is_none_or(|id| live_ids.contains(&id))
                });

                let due_monitors: Vec<_> = monitors
                    .into_iter()
                    .filter(|monitor| {
                        let interval = Duration::from_secs(monitor.interval_secs.max(10) as u64);
                        ping_last_checked
                            .get(&monitor.id)
                            .is_none_or(|t| t.elapsed() >= interval)
                    })
                    .collect();

                for monitor in &due_monitors {
                    ping_last_checked.insert(monitor.id, Instant::now());
                }

                let results: Vec<_> = stream::iter(due_monitors.into_iter().map(|monitor| {
                    let pool = state.db_pool.clone();
                    async move {
                        let error = check_ping_host(&pool, &monitor).await;
                        (monitor, error)
                    }
                }))
                .buffer_unordered(MONITOR_CONCURRENCY)
                .collect()
                .await;

                for (monitor, error) in results {
                    handle_monitor_alert(
                        &state,
                        &mut alert_state,
                        &format!("ping:{}", monitor.id),
                        &monitor.name,
                        error,
                    )
                    .await;
                }
            }
        }
    })
}

/// Handle alert state transitions for a monitor check result.
/// Sends failure alerts (with cooldown) and recovery alerts.
async fn handle_monitor_alert(
    state: &AppState,
    alert_state: &mut HashMap<String, MonitorAlertState>,
    key: &str,
    name: &str,
    error: Option<String>,
) {
    let entry = alert_state
        .entry(key.to_string())
        .or_insert(MonitorAlertState {
            is_failing: false,
            last_alert: None,
        });

    let cooldown = Duration::from_secs(MONITOR_ALERT_COOLDOWN_SECS);

    if let Some(err_msg) = error {
        // Failure: send alert if cooldown has passed
        let should_alert = entry.last_alert.is_none_or(|t| t.elapsed() >= cooldown);

        if should_alert {
            let monitor_type = if key.starts_with("http:") {
                "HTTP"
            } else {
                "Ping"
            };
            let msg = format!(
                "🚨 **[{} Monitor Down]** `{}` — {}",
                monitor_type, name, err_msg
            );
            spawn_monitor_alert(state, key.to_string(), "monitor_down", msg);
            entry.last_alert = Some(Instant::now());
        }
        entry.is_failing = true;
    } else if entry.is_failing {
        // Recovery: was failing, now succeeds
        let monitor_type = if key.starts_with("http:") {
            "HTTP"
        } else {
            "Ping"
        };
        let msg = format!(
            "✅ **[{} Monitor Recovery]** `{}` — back online",
            monitor_type, name
        );
        spawn_monitor_alert(state, key.to_string(), "monitor_recovery", msg);
        entry.is_failing = false;
        entry.last_alert = None;
    }
}

/// Fan out a monitor alert: fire the outbound webhook + log to alert_history
/// on a detached task so the monitor scheduler loop is never blocked by
/// external HTTP latency.
fn spawn_monitor_alert(state: &AppState, key: String, alert_type: &'static str, msg: String) {
    let http = state.http_client.clone();
    let pool = state.db_pool.clone();
    tokio::spawn(async move {
        alert_service::send_alert(&http, &pool, &msg).await;
        if let Err(e) = alert_history_repo::insert_alert(&pool, &key, alert_type, &msg).await {
            tracing::error!(err = ?e, %alert_type, "⚠️ [AlertHistory] Failed to log monitor alert");
        }
    });
}

/// Check an HTTP endpoint. Returns Some(error_message) on failure, None on success.
async fn check_http_endpoint(
    pool: &crate::db::DbPool,
    client: &reqwest::Client,
    monitor: &http_monitors_repo::HttpMonitor,
) -> Option<String> {
    // Defense-in-depth: re-validate URL at runtime (catches pre-existing DB entries)
    if let Err(e) = super::url_validator::validate_url(&monitor.url, &["http", "https"]).await {
        tracing::warn!(monitor_id = monitor.id, url = %monitor.url, "⚠️ [HTTP Monitor] SSRF blocked: {e}");
        return Some(format!("SSRF blocked: {e}"));
    }

    let timeout = Duration::from_millis(monitor.timeout_ms.max(1000) as u64);
    let start = Instant::now();

    let request = match monitor.method.to_uppercase().as_str() {
        "POST" => client.post(&monitor.url),
        "HEAD" => client.head(&monitor.url),
        _ => client.get(&monitor.url),
    };

    match request.timeout(timeout).send().await {
        Ok(response) => {
            let elapsed = i32::try_from(start.elapsed().as_millis()).unwrap_or(i32::MAX);
            let status = response.status().as_u16() as i32;
            let error = if status != monitor.expected_status {
                Some(format!(
                    "Expected status {}, got {}",
                    monitor.expected_status, status
                ))
            } else {
                None
            };

            if let Err(e) = http_monitors_repo::insert_result(
                pool,
                monitor.id,
                Some(status),
                Some(elapsed),
                error.as_deref(),
            )
            .await
            {
                tracing::error!(monitor_id = monitor.id, err = ?e, "⚠️ [HTTP Monitor] Failed to store result");
            }

            error
        }
        Err(e) => {
            let elapsed = i32::try_from(start.elapsed().as_millis()).unwrap_or(i32::MAX);
            let error_msg = if e.is_timeout() {
                format!("Timeout after {}ms", monitor.timeout_ms)
            } else {
                e.to_string()
            };

            if let Err(e) = http_monitors_repo::insert_result(
                pool,
                monitor.id,
                None,
                Some(elapsed),
                Some(&error_msg),
            )
            .await
            {
                tracing::error!(monitor_id = monitor.id, err = ?e, "⚠️ [HTTP Monitor] Failed to store result");
            }

            Some(error_msg)
        }
    }
}

/// Check a ping (TCP connect) host. Returns Some(error_message) on failure, None on success.
async fn check_ping_host(
    pool: &crate::db::DbPool,
    monitor: &ping_monitors_repo::PingMonitor,
) -> Option<String> {
    // Defense-in-depth: re-validate host at runtime (catches pre-existing DB entries)
    if let Err(e) = super::url_validator::validate_host(&monitor.host).await {
        tracing::warn!(monitor_id = monitor.id, host = %monitor.host, "⚠️ [Ping Monitor] SSRF blocked: {e}");
        return Some(format!("SSRF blocked: {e}"));
    }

    let timeout = Duration::from_millis(monitor.timeout_ms.max(1000) as u64);

    // Use tokio TCP connect as a cross-platform "ping" alternative.
    // True ICMP ping requires raw sockets (root/CAP_NET_RAW) which is impractical in Docker.
    let target = if monitor.host.contains(':') {
        monitor.host.clone()
    } else {
        format!("{}:80", monitor.host)
    };

    let start = Instant::now();
    match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(&target)).await {
        Ok(Ok(_)) => {
            let rtt = start.elapsed().as_secs_f64() * 1000.0;
            let _ =
                ping_monitors_repo::insert_result(pool, monitor.id, Some(rtt), true, None).await;
            None
        }
        Ok(Err(e)) => {
            let error_msg = e.to_string();
            let _ =
                ping_monitors_repo::insert_result(pool, monitor.id, None, false, Some(&error_msg))
                    .await;
            Some(error_msg)
        }
        Err(_) => {
            let error_msg = format!("Timeout after {}ms", monitor.timeout_ms);
            let _ =
                ping_monitors_repo::insert_result(pool, monitor.id, None, false, Some(&error_msg))
                    .await;
            Some(error_msg)
        }
    }
}
