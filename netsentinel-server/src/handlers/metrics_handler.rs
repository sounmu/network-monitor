use std::collections::HashMap;
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

            // All time-range queries use the in-memory TTL cache.
            // Cache keys are rounded to 5-minute boundaries so near-identical
            // requests share a single entry.
            let cache_key =
                MetricsQueryCache::make_key(&host_key, start.timestamp(), end.timestamp());
            if let Some(cached) = state.metrics_query_cache.get(&cache_key) {
                Arc::unwrap_or_clone(cached)
            } else {
                let result =
                    metrics_repo::fetch_metrics_range(&state.db_pool, &host_key, start, end)
                        .await?;
                // insert() returns Arc — no clone needed
                let arc = state.metrics_query_cache.insert(cache_key, result);
                Arc::unwrap_or_clone(arc)
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

/// Request body for batch metrics query
#[derive(Deserialize)]
pub struct BatchMetricsRequest {
    pub host_keys: Vec<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

/// POST /api/metrics/batch — fetch metrics for multiple hosts in a single request.
///
/// Reduces HTTP overhead when the dashboard renders charts for many hosts simultaneously.
/// Each host_key is queried concurrently, with cache applied for long-range queries.
pub async fn batch_metrics(
    _auth: AuthGuard,
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchMetricsRequest>,
) -> Result<Json<HashMap<String, Vec<MetricsRow>>>, AppError> {
    if req.host_keys.is_empty() {
        return Ok(Json(HashMap::new()));
    }
    if req.host_keys.len() > 50 {
        return Err(AppError::BadRequest(
            "Too many host_keys (max 50)".to_string(),
        ));
    }
    if req.start > req.end {
        return Err(AppError::BadRequest(
            "start must not be later than end".to_string(),
        ));
    }

    let futs = req.host_keys.iter().map(|host_key| {
        let pool = &state.db_pool;
        let cache = &state.metrics_query_cache;
        let start = req.start;
        let end = req.end;
        let hk = host_key.clone();

        async move {
            let cache_key = MetricsQueryCache::make_key(&hk, start.timestamp(), end.timestamp());
            let rows = if let Some(cached) = cache.get(&cache_key) {
                Arc::unwrap_or_clone(cached)
            } else {
                let result = metrics_repo::fetch_metrics_range(pool, &hk, start, end).await?;
                let arc = cache.insert(cache_key, result);
                Arc::unwrap_or_clone(arc)
            };
            Ok::<(String, Vec<MetricsRow>), AppError>((hk, rows))
        }
    });

    let results = futures::future::join_all(futs).await;
    let mut map = HashMap::with_capacity(req.host_keys.len());
    for result in results {
        let (hk, rows) = result?;
        map.insert(hk, rows);
    }

    Ok(Json(map))
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
/// Fetches host summaries + 7-day uptime in two queries (not N+1).
pub async fn public_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<PublicHostStatus>>, AppError> {
    // Two parallel queries instead of 1 + N sequential queries
    let (hosts, uptime_map) = tokio::try_join!(
        async {
            metrics_repo::fetch_host_summaries(&state.db_pool)
                .await
                .map_err(|e| AppError::Internal(e.to_string()))
        },
        async {
            metrics_repo::fetch_batch_uptime_pct(&state.db_pool, 7)
                .await
                .map_err(|e| AppError::Internal(e.to_string()))
        },
    )?;

    let results = hosts
        .into_iter()
        .map(|host| {
            let uptime_7d = uptime_map.get(&host.host_key).copied().unwrap_or(0.0);
            PublicHostStatus {
                host_key: host.host_key,
                display_name: host.display_name,
                is_online: host.is_online,
                uptime_7d,
            }
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
