use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::errors::AppError;
use crate::models::app_state::{AppState, MetricsQueryCache};
use crate::repositories::metrics_repo::{self, MetricsRow, UptimeSummary};
use crate::services::auth::UserGuard;

/// Optional shared-secret bearer guard for `/metrics`.
///
/// Unauthenticated by default (backward-compatible for operators who
/// firewall the endpoint at the reverse proxy). If `METRICS_TOKEN` is set
/// at startup, every scrape must present `Authorization: Bearer <token>`.
///
/// `OnceLock` is seeded from env once at startup by `get_metrics_token()`;
/// rotating the token requires a server restart by design (same contract
/// as `JWT_SECRET`).
fn get_metrics_token() -> &'static Option<String> {
    use std::sync::OnceLock;
    static METRICS_TOKEN: OnceLock<Option<String>> = OnceLock::new();
    METRICS_TOKEN.get_or_init(|| {
        std::env::var("METRICS_TOKEN")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    })
}

fn check_metrics_auth(headers: &HeaderMap) -> Result<(), AppError> {
    let Some(expected) = get_metrics_token().as_deref() else {
        return Ok(()); // No token configured → open endpoint (legacy behavior).
    };
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    // Constant-time comparison avoids timing oracles on short-circuit equality.
    if presented.len() == expected.len()
        && presented
            .as_bytes()
            .iter()
            .zip(expected.as_bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    {
        Ok(())
    } else {
        Err(AppError::Unauthorized(
            "Prometheus endpoint requires a valid bearer token".into(),
        ))
    }
}

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
/// SP-04: Return `Arc<Vec>` directly — `Json<Arc<Vec<T>>>` serializes via deref
/// without cloning the Vec. Previously `Arc::unwrap_or_clone` always cloned
/// because the cache retains its own Arc reference (refcount >= 2).
pub async fn get_metrics_by_host_key(
    _auth: UserGuard,
    State(state): State<Arc<AppState>>,
    Path(host_key): Path<String>,
    Query(range): Query<TimeRangeQuery>,
) -> Result<Json<Arc<Vec<MetricsRow>>>, AppError> {
    let rows = match (range.start, range.end) {
        (Some(start), Some(end)) => {
            if start > end {
                return Err(AppError::BadRequest(
                    "start must not be later than end".to_string(),
                ));
            }

            let cache_key =
                MetricsQueryCache::make_key(&host_key, start.timestamp(), end.timestamp());
            if let Some(cached) = state.metrics_query_cache.get(&cache_key) {
                cached
            } else {
                let result =
                    metrics_repo::fetch_metrics_range(&state.db_pool, &host_key, start, end)
                        .await?;
                state.metrics_query_cache.insert(cache_key, result)
            }
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(AppError::BadRequest(
                "start and end must both be provided, or both omitted".to_string(),
            ));
        }
        _ => Arc::new(metrics_repo::fetch_recent_metrics(&state.db_pool, &host_key).await?),
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
    _auth: UserGuard,
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
/// SP-03: Uses `buffer_unordered(5)` instead of `join_all` to cap concurrent DB
/// queries at half the default pool size, preventing pool exhaustion.
/// SP-04: Returns `Arc<Vec>` to avoid cloning cached data.
pub async fn batch_metrics(
    _auth: UserGuard,
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchMetricsRequest>,
) -> Result<Json<HashMap<String, Arc<Vec<MetricsRow>>>>, AppError> {
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

    use futures::stream::{self, StreamExt};

    let pool = state.db_pool.clone();
    let cache = state.metrics_query_cache.clone();
    let start = req.start;
    let end = req.end;
    let host_keys = req.host_keys;
    let num_keys = host_keys.len();

    let results: Vec<_> = stream::iter(host_keys.into_iter().map(|hk| {
        let pool = pool.clone();
        let cache = cache.clone();
        async move {
            let cache_key = MetricsQueryCache::make_key(&hk, start.timestamp(), end.timestamp());
            let rows = if let Some(cached) = cache.get(&cache_key) {
                cached
            } else {
                let result = metrics_repo::fetch_metrics_range(&pool, &hk, start, end).await?;
                cache.insert(cache_key, result)
            };
            Ok::<_, AppError>((hk, rows))
        }
    }))
    .buffer_unordered(5)
    .collect()
    .await;

    let mut map = HashMap::with_capacity(num_keys);
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
/// Escape a string for use as a Prometheus label value.
/// Per the text exposition format: backslash, double-quote, and newline must be escaped.
fn escape_prom_label(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

pub async fn prometheus_metrics(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    check_metrics_auth(&headers)?;

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
                escape_prom_label(host_key),
                escape_prom_label(&status.display_name),
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
            escape_prom_label(host_key),
            escape_prom_label(&record.last_known_hostname),
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
