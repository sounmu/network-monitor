use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use tokio::time::Instant;

use crate::models::app_state::AppState;
use crate::repositories::{http_monitors_repo, ping_monitors_repo};

/// Minimum scrape interval to prevent excessive polling
const MIN_INTERVAL_SECS: u64 = 10;

/// Start the HTTP and Ping monitor scraper as a background task.
/// Runs every 10 seconds — each monitor tracks its own interval via last_checked timestamps.
pub fn spawn_monitor_scraper(state: Arc<AppState>) {
    tokio::spawn(async move {
        // Track last check time per monitor
        let mut http_last_checked: std::collections::HashMap<i32, Instant> =
            std::collections::HashMap::new();
        let mut ping_last_checked: std::collections::HashMap<i32, Instant> =
            std::collections::HashMap::new();

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
                        check_http_endpoint(&state.db_pool, &state.http_client, monitor).await;
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
                        check_ping_host(&state.db_pool, monitor).await;
                    }
                }
            }
        }
    });
}

async fn check_http_endpoint(
    pool: &PgPool,
    client: &reqwest::Client,
    monitor: &http_monitors_repo::HttpMonitor,
) {
    let timeout = Duration::from_millis(monitor.timeout_ms.max(1000) as u64);
    let start = Instant::now();

    let request = match monitor.method.to_uppercase().as_str() {
        "POST" => client.post(&monitor.url),
        "HEAD" => client.head(&monitor.url),
        _ => client.get(&monitor.url),
    };

    match request.timeout(timeout).send().await {
        Ok(response) => {
            let elapsed = start.elapsed().as_millis() as i32;
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
        }
        Err(e) => {
            let elapsed = start.elapsed().as_millis() as i32;
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
        }
    }
}

async fn check_ping_host(pool: &PgPool, monitor: &ping_monitors_repo::PingMonitor) {
    let timeout = Duration::from_millis(monitor.timeout_ms.max(1000) as u64);

    // Use tokio TCP connect as a cross-platform "ping" alternative.
    // True ICMP ping requires raw sockets (root/CAP_NET_RAW) which is impractical in Docker.
    // TCP connect to port 80/443 achieves the same reachability check.
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
        }
        Ok(Err(e)) => {
            let _ = ping_monitors_repo::insert_result(
                pool,
                monitor.id,
                None,
                false,
                Some(&e.to_string()),
            )
            .await;
        }
        Err(_) => {
            let _ = ping_monitors_repo::insert_result(
                pool,
                monitor.id,
                None,
                false,
                Some(&format!("Timeout after {}ms", monitor.timeout_ms)),
            )
            .await;
        }
    }
}
