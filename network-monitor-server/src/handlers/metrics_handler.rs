use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::errors::AppError;
use crate::models::app_state::{AppState, MetricsQueryCache};
use crate::repositories::metrics_repo::{self, MetricsRow, UptimeSummary};
use crate::services::auth::AuthGuard;

/// GET / — server health check
pub async fn root_handler() -> &'static str {
    "Monitoring Hub is running! (Pull mode)"
}

/// GET /api/health — deep health check (verifies DB connectivity)
pub async fn health_check(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    sqlx::query("SELECT 1")
        .execute(&state.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Database health check failed: {e}")))?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    })))
}

/// Time range query parameters
#[derive(Deserialize)]
pub struct TimeRangeQuery {
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
}

/// GET /api/metrics/:host_key — fetch metrics for a specific host
///
/// host_key is a target-URL-based unique identifier — avoids hostname collisions.
/// Optional `start` and `end` query params return data within that time range.
/// If omitted, returns the most recent 50 records.
pub async fn get_metrics_by_host_key(
    _auth: AuthGuard,
    State(state): State<Arc<AppState>>,
    Path(host_key): Path<String>,
    Query(range): Query<TimeRangeQuery>,
) -> Result<Json<Vec<MetricsRow>>, AppError> {
    let rows = match (range.start, range.end) {
        (Some(start), Some(end)) => {
            if start > end {
                return Err(AppError::BadRequest(
                    "start must not be later than end".to_string(),
                ));
            }

            let duration_hours = (end - start).num_hours();

            // For long ranges (>6h), use the in-memory TTL cache to avoid repeated DB scans
            if duration_hours > 6 {
                let cache_key =
                    MetricsQueryCache::make_key(&host_key, start.timestamp(), end.timestamp());
                if let Some(cached) = state.metrics_query_cache.get(&cache_key) {
                    cached
                } else {
                    let result =
                        metrics_repo::fetch_metrics_range(&state.db_pool, &host_key, start, end)
                            .await?;
                    state.metrics_query_cache.insert(cache_key, result.clone());
                    result
                }
            } else {
                metrics_repo::fetch_metrics_range(&state.db_pool, &host_key, start, end).await?
            }
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(AppError::BadRequest(
                "start and end must both be provided, or both omitted".to_string(),
            ));
        }
        _ => metrics_repo::fetch_recent_metrics(&state.db_pool, &host_key).await?,
    };

    Ok(Json(rows))
}

/// Query parameter for uptime endpoint
#[derive(Deserialize)]
pub struct UptimeQuery {
    pub days: Option<i32>,
}

/// GET /api/uptime/:host_key — compute daily uptime for a host
pub async fn get_uptime(
    _auth: AuthGuard,
    State(state): State<Arc<AppState>>,
    Path(host_key): Path<String>,
    Query(query): Query<UptimeQuery>,
) -> Result<Json<UptimeSummary>, AppError> {
    let days = query.days.unwrap_or(30).min(90);
    let summary = metrics_repo::fetch_uptime(&state.db_pool, &host_key, days).await?;
    Ok(Json(summary))
}

/// Public status response (no auth required)
#[derive(serde::Serialize)]
pub struct PublicHostStatus {
    pub host_key: String,
    pub display_name: String,
    pub is_online: bool,
    pub uptime_7d: f64,
}

/// GET /api/public/status — public status page data (no auth)
///
/// Fetches host summaries + 7-day uptime for each host concurrently.
pub async fn public_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<PublicHostStatus>>, AppError> {
    let hosts = metrics_repo::fetch_host_summaries(&state.db_pool).await?;

    // Run all uptime queries concurrently instead of sequentially
    let futures: Vec<_> = hosts
        .iter()
        .map(|host| {
            let pool = state.db_pool.clone();
            let host_key = host.host_key.clone();
            async move { metrics_repo::fetch_uptime(&pool, &host_key, 7).await }
        })
        .collect();
    let uptimes = futures::future::join_all(futures).await;

    let results = hosts
        .iter()
        .zip(uptimes)
        .map(|(host, uptime_result)| PublicHostStatus {
            host_key: host.host_key.clone(),
            display_name: host.display_name.clone(),
            is_online: host.is_online,
            uptime_7d: uptime_result.map(|u| u.overall_pct).unwrap_or(0.0),
        })
        .collect();

    Ok(Json(results))
}

/// GET /metrics — Prometheus-compatible metrics export (text format)
///
/// Exports the latest metrics for all hosts as Prometheus gauges.
/// This endpoint is designed to be scraped by a Prometheus server.
pub async fn prometheus_metrics(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let store = state
        .store
        .read()
        .map_err(|e| AppError::Internal(format!("Failed to acquire store read lock: {}", e)))?;

    let mut output = String::new();

    // HELP and TYPE declarations
    output
        .push_str("# HELP netmonitor_host_online Whether the host is online (1) or offline (0).\n");
    output.push_str("# TYPE netmonitor_host_online gauge\n");
    output.push_str("# HELP netmonitor_cpu_usage_percent CPU usage percentage.\n");
    output.push_str("# TYPE netmonitor_cpu_usage_percent gauge\n");
    output.push_str("# HELP netmonitor_memory_usage_percent Memory usage percentage.\n");
    output.push_str("# TYPE netmonitor_memory_usage_percent gauge\n");
    output.push_str("# HELP netmonitor_load_1min Load average (1 minute).\n");
    output.push_str("# TYPE netmonitor_load_1min gauge\n");
    output.push_str("# HELP netmonitor_load_5min Load average (5 minutes).\n");
    output.push_str("# TYPE netmonitor_load_5min gauge\n");
    output.push_str("# HELP netmonitor_load_15min Load average (15 minutes).\n");
    output.push_str("# TYPE netmonitor_load_15min gauge\n");

    // Read latest metrics from the SSE status + metrics maps
    if let Ok(lks) = state.last_known_status.read() {
        for (host_key, status) in lks.iter() {
            let labels = format!(
                "host_key=\"{}\",display_name=\"{}\"",
                host_key.replace('"', "\\\""),
                status.display_name.replace('"', "\\\""),
            );
            let online = if status.is_online { 1 } else { 0 };
            output.push_str(&format!(
                "netmonitor_host_online{{{}}} {}\n",
                labels, online
            ));
        }
    }

    // Per-host metric values from in-memory store
    for (host_key, record) in &store.hosts {
        let labels = format!(
            "host_key=\"{}\",display_name=\"{}\"",
            host_key.replace('"', "\\\""),
            record.last_known_hostname.replace('"', "\\\""),
        );

        if let Some(latest) = record.alert_history.back() {
            output.push_str(&format!(
                "netmonitor_cpu_usage_percent{{{}}} {:.2}\n",
                labels, latest.cpu_usage_percent
            ));
            output.push_str(&format!(
                "netmonitor_memory_usage_percent{{{}}} {:.2}\n",
                labels, latest.memory_usage_percent
            ));
        }
    }

    Ok((
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        output,
    ))
}
