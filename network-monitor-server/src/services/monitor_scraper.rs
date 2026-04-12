use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;

use crate::models::app_state::AppState;
use crate::repositories::{alert_history_repo, http_monitors_repo, ping_monitors_repo};
use crate::services::alert_service;

/// Minimum scrape interval to prevent excessive polling
const MIN_INTERVAL_SECS: u64 = 10;
/// Alert cooldown per monitor to prevent spam (seconds)
const MONITOR_ALERT_COOLDOWN_SECS: u64 = 300;

/// Per-monitor alert state tracking
struct MonitorAlertState {
    is_failing: bool,
    last_alert: Option<Instant>,
}

/// Start the HTTP and Ping monitor scraper as a background task.
/// Runs every 10 seconds — each monitor tracks its own interval via last_checked timestamps.
pub fn spawn_monitor_scraper(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Track last check time per monitor
        let mut http_last_checked: HashMap<i32, Instant> = HashMap::new();
        let mut ping_last_checked: HashMap<i32, Instant> = HashMap::new();
        // Track alert state per monitor (keyed by "http:{id}" or "ping:{id}")
        let mut alert_state: HashMap<String, MonitorAlertState> = HashMap::new();

        loop {
            tokio::time::sleep(Duration::from_secs(MIN_INTERVAL_SECS)).await;

            // HTTP monitors
            if let Ok(monitors) = http_monitors_repo::get_enabled(&state.db_pool).await {
                for monitor in &monitors {
                    let interval = Duration::from_secs(monitor.interval_secs.max(10) as u64);
                    let should_check = http_last_checked
                        .get(&monitor.id)
                        .is_none_or(|t| t.elapsed() >= interval);

                    if should_check {
                        http_last_checked.insert(monitor.id, Instant::now());
                        let error =
                            check_http_endpoint(&state.db_pool, &state.http_client, monitor).await;
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
            }

            // Ping monitors
            if let Ok(monitors) = ping_monitors_repo::get_enabled(&state.db_pool).await {
                for monitor in &monitors {
                    let interval = Duration::from_secs(monitor.interval_secs.max(10) as u64);
                    let should_check = ping_last_checked
                        .get(&monitor.id)
                        .is_none_or(|t| t.elapsed() >= interval);

                    if should_check {
                        ping_last_checked.insert(monitor.id, Instant::now());
                        let error = check_ping_host(&state.db_pool, monitor).await;
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
            alert_service::send_alert(&state.http_client, &state.db_pool, &msg).await;
            let _ =
                alert_history_repo::insert_alert(&state.db_pool, key, "monitor_down", &msg).await;
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
        alert_service::send_alert(&state.http_client, &state.db_pool, &msg).await;
        let _ =
            alert_history_repo::insert_alert(&state.db_pool, key, "monitor_recovery", &msg).await;
        entry.is_failing = false;
        entry.last_alert = None;
    }
}

/// Check an HTTP endpoint. Returns Some(error_message) on failure, None on success.
async fn check_http_endpoint(
    pool: &sqlx::PgPool,
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
    pool: &sqlx::PgPool,
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
