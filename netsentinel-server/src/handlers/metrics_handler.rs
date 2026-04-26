use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::errors::AppError;
use crate::models::app_state::{
    AppState, CacheWeight, MetricsQueryCache, metrics_cache_key, should_cache_metrics_range,
};
use crate::repositories::metrics_repo::{self, ChartMetricsRow, MetricsRow, UptimeSummary};
use crate::repositories::{http_monitors_repo, ping_monitors_repo};
use crate::services::auth::UserGuard;

/// Default raw boundary used by the full-metrics endpoints (≤ 6 h is never
/// cached). The lightweight chart endpoint passes its own 1 h boundary via
/// `metrics_repo::CHART_RAW_BOUNDARY_SECS`.
const FULL_METRICS_RAW_BOUNDARY_SECS: i64 = 6 * 3600;

/// Resolve a single (host_key, time-range) query against a metrics cache,
/// falling back to a one-shot fetch when the range is outside the cacheable
/// window. The `raw_boundary_secs` argument lets the chart and full-metrics
/// callers share this helper while keeping their own raw thresholds.
async fn cached_or_fetch<T, F, Fut>(
    cache: &MetricsQueryCache<T>,
    raw_boundary_secs: i64,
    host_key: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    fetch: F,
) -> Result<Arc<Vec<T>>, AppError>
where
    T: CacheWeight,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Vec<T>, sqlx::Error>>,
{
    let start_ts = start.timestamp();
    let end_ts = end.timestamp();
    if !should_cache_metrics_range(start_ts, end_ts, raw_boundary_secs) {
        let rows = fetch().await?;
        return Ok(Arc::new(rows));
    }

    let key = metrics_cache_key(host_key, start_ts, end_ts, raw_boundary_secs);
    if let Some(cached) = cache.get(&key) {
        return Ok(cached);
    }
    let rows = fetch().await?;
    Ok(cache.insert(key, rows))
}

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

fn allow_unauthenticated_metrics() -> bool {
    use std::sync::OnceLock;
    static ALLOW_UNAUTHENTICATED: OnceLock<bool> = OnceLock::new();
    *ALLOW_UNAUTHENTICATED.get_or_init(|| {
        matches!(
            std::env::var("ALLOW_UNAUTHENTICATED_METRICS")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn check_metrics_auth(headers: &HeaderMap) -> Result<(), AppError> {
    let Some(expected) = get_metrics_token().as_deref() else {
        if allow_unauthenticated_metrics() {
            return Ok(());
        }
        return Err(AppError::Unauthorized(
            "Prometheus endpoint requires METRICS_TOKEN unless ALLOW_UNAUTHENTICATED_METRICS=true"
                .into(),
        ));
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

            cached_or_fetch(
                &state.metrics_query_cache,
                FULL_METRICS_RAW_BOUNDARY_SECS,
                &host_key,
                start,
                end,
                || metrics_repo::fetch_metrics_range(&state.db_pool, &host_key, start, end),
            )
            .await?
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

/// GET /api/metrics/:host_key/chart — chart-only metrics for a specific host.
///
/// Returns a narrower row shape than `/api/metrics/:host_key`, omitting large
/// detail snapshots that the host charts never render. Windows up to 1h use raw
/// 10s rows; longer chart windows use 5-minute rollups and the bounded chart
/// cache.
pub async fn get_chart_metrics_by_host_key(
    _auth: UserGuard,
    State(state): State<Arc<AppState>>,
    Path(host_key): Path<String>,
    Query(range): Query<TimeRangeQuery>,
) -> Result<Json<Arc<Vec<ChartMetricsRow>>>, AppError> {
    let (Some(start), Some(end)) = (range.start, range.end) else {
        return Err(AppError::BadRequest(
            "start and end must both be provided".to_string(),
        ));
    };
    if start > end {
        return Err(AppError::BadRequest(
            "start must not be later than end".to_string(),
        ));
    }

    let rows = cached_or_fetch(
        &state.chart_metrics_query_cache,
        metrics_repo::CHART_RAW_BOUNDARY_SECS,
        &host_key,
        start,
        end,
        || metrics_repo::fetch_chart_metrics_range(&state.db_pool, &host_key, start, end),
    )
    .await?;

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
/// SP-03: Uses `buffer_unordered(pool_size - 1)` instead of `join_all` to cap
/// concurrent DB queries and leave room for scrape-cycle writes.
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
    // SQLite has exactly one writer; keep one pool slot out of this read
    // fan-out so scrape-cycle batch INSERTs are less likely to starve.
    // Floor at 1 so tiny pools still make forward progress.
    let fanout = state.max_db_connections.saturating_sub(1).max(1) as usize;

    let results: Vec<_> = stream::iter(host_keys.into_iter().map(|hk| {
        let pool = pool.clone();
        let cache = cache.clone();
        async move {
            let rows = cached_or_fetch(
                &cache,
                FULL_METRICS_RAW_BOUNDARY_SECS,
                &hk,
                start,
                end,
                || metrics_repo::fetch_metrics_range(&pool, &hk, start, end),
            )
            .await?;
            Ok::<_, AppError>((hk, rows))
        }
    }))
    .buffer_unordered(fanout)
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

#[derive(serde::Serialize)]
pub struct PublicMonitorStatus {
    pub monitor_id: i32,
    // `"http"` | `"ping"` — kept as a string in the wire contract so the frontend
    // can treat both kinds uniformly without enum discriminator bookkeeping.
    pub kind: &'static str,
    pub name: String,
    // For HTTP monitors this is the configured URL; for Ping monitors it is the
    // `host[:port]` target. Renamed from kind-specific fields so the frontend's
    // list view can render a single `target` column.
    pub target: String,
    pub is_online: bool,
    pub uptime_24h: f64,
}

#[derive(serde::Serialize)]
pub struct PublicStatusResponse {
    pub hosts: Vec<PublicHostStatus>,
    pub monitors: Vec<PublicMonitorStatus>,
}

/// GET /api/public/status — public status page data (no auth)
///
/// **Opt-in**: returns 404 unless `PUBLIC_STATUS_ENABLED=true` is set.
/// When enabled, emits agent `host_key` (often an internal IP:port),
/// `display_name` (OS hostname), 7-day uptime, **and** the list of external
/// HTTP / Ping monitors with 24-hour uptime. That full surface is exactly what
/// a Zero-Trust homelab setup wants to keep private, so the safe default is
/// off — the endpoint still exists so self-hosters can flip it on when they
/// explicitly want a public status page behind their Cloudflare tunnel.
///
/// Fetches hosts + 7-day uptime + HTTP monitors (list + summaries) + Ping
/// monitors (list + summaries) in parallel (`tokio::try_join!`), so total
/// latency is dominated by the slowest single query rather than summed.
pub async fn public_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<PublicStatusResponse>, AppError> {
    if !public_status_enabled() {
        return Err(AppError::NotFound(
            "public status is disabled — set PUBLIC_STATUS_ENABLED=true to enable".into(),
        ));
    }

    let (hosts, uptime_map, http_list, http_summaries, ping_list, ping_summaries) = tokio::try_join!(
        async {
            metrics_repo::fetch_host_summaries(&state.db_pool)
                .await
                .map_err(AppError::from)
        },
        async {
            metrics_repo::fetch_batch_uptime_pct(&state.db_pool, 7)
                .await
                .map_err(AppError::from)
        },
        async {
            http_monitors_repo::get_all(&state.db_pool)
                .await
                .map_err(AppError::from)
        },
        async {
            http_monitors_repo::get_summaries(&state.db_pool)
                .await
                .map_err(AppError::from)
        },
        async {
            ping_monitors_repo::get_all(&state.db_pool)
                .await
                .map_err(AppError::from)
        },
        async {
            ping_monitors_repo::get_summaries(&state.db_pool)
                .await
                .map_err(AppError::from)
        },
    )?;

    let host_rows = hosts
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

    let http_summary_map: HashMap<i32, &http_monitors_repo::HttpMonitorSummary> =
        http_summaries.iter().map(|s| (s.monitor_id, s)).collect();
    let ping_summary_map: HashMap<i32, &ping_monitors_repo::PingMonitorSummary> =
        ping_summaries.iter().map(|s| (s.monitor_id, s)).collect();

    let mut monitor_rows: Vec<PublicMonitorStatus> =
        Vec::with_capacity(http_list.len() + ping_list.len());

    for m in http_list.into_iter().filter(|m| m.enabled) {
        let summary = http_summary_map.get(&m.id);
        // `latest_error.is_none() && latest_status_code.is_some()` mirrors the
        // `successful_checks` criterion in `get_summaries` so the "is_online"
        // projection stays consistent with the uptime % in the same row.
        let is_online =
            summary.is_some_and(|s| s.latest_error.is_none() && s.latest_status_code.is_some());
        let uptime_24h = summary.map(|s| s.uptime_pct).unwrap_or(0.0);
        monitor_rows.push(PublicMonitorStatus {
            monitor_id: m.id,
            kind: "http",
            name: m.name,
            target: m.url,
            is_online,
            uptime_24h,
        });
    }

    for m in ping_list.into_iter().filter(|m| m.enabled) {
        let summary = ping_summary_map.get(&m.id);
        let is_online = summary.is_some_and(|s| s.latest_success == Some(true));
        let uptime_24h = summary.map(|s| s.uptime_pct).unwrap_or(0.0);
        monitor_rows.push(PublicMonitorStatus {
            monitor_id: m.id,
            kind: "ping",
            name: m.name,
            target: m.host,
            is_online,
            uptime_24h,
        });
    }

    Ok(Json(PublicStatusResponse {
        hosts: host_rows,
        monitors: monitor_rows,
    }))
}

fn public_status_enabled() -> bool {
    std::env::var("PUBLIC_STATUS_ENABLED")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
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

    // Snapshot both maps into plain Vecs under separate, minimal lock scopes —
    // then drop every lock before any formatting happens. Holding `store.read()`
    // while building O(hosts) format!() strings blocks the scraper (which takes
    // `store.write()` every cycle), and the old code nested `last_known_status`
    // inside `store` which also established a lock-ordering trap the scraper
    // doesn't follow.
    struct OnlineRow {
        host_key: String,
        display_name: String,
        is_online: bool,
    }
    struct MetricRow {
        host_key: String,
        display_name: String,
        cpu_usage_percent: f32,
        memory_usage_percent: f32,
    }

    let status_snapshot: Vec<OnlineRow> = state
        .last_known_status
        .read()
        .map(|lks| {
            lks.iter()
                .map(|(k, s)| OnlineRow {
                    host_key: k.clone(),
                    display_name: s.display_name.clone(),
                    is_online: s.is_online,
                })
                .collect()
        })
        .unwrap_or_default();

    let metric_snapshot: Vec<MetricRow> = state
        .store
        .read()
        .map(|store| {
            store
                .hosts
                .iter()
                .filter_map(|(k, record)| {
                    record.alert_history.back().map(|latest| MetricRow {
                        host_key: k.clone(),
                        display_name: record.last_known_hostname.clone(),
                        cpu_usage_percent: latest.cpu_usage_percent,
                        memory_usage_percent: latest.memory_usage_percent,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Build the exposition text without any lock held. Pre-size the output
    // to avoid repeated allocations for large host counts.
    //
    // Formatting uses `write!` against the pre-sized `String` directly, so
    // every interpolation writes into the same heap buffer. The old
    // `output.push_str(&format!(...))` pattern allocated a throwaway
    // `String` per metric line — negligible for a handful of hosts but
    // O(hosts × metrics) allocations on large fleets. The `std::fmt::Write`
    // calls into `String` cannot fail (grow() handles capacity), so we
    // swallow the `Result` with `let _ =`.
    use std::fmt::Write as _;

    let mut output =
        String::with_capacity(256 + status_snapshot.len() * 96 + metric_snapshot.len() * 192);
    output
        .push_str("# HELP netmonitor_host_online Whether the host is online (1) or offline (0).\n");
    output.push_str("# TYPE netmonitor_host_online gauge\n");
    output.push_str("# HELP netmonitor_cpu_usage_percent CPU usage percentage.\n");
    output.push_str("# TYPE netmonitor_cpu_usage_percent gauge\n");
    output.push_str("# HELP netmonitor_memory_usage_percent Memory usage percentage.\n");
    output.push_str("# TYPE netmonitor_memory_usage_percent gauge\n");
    output.push_str("# HELP netmonitor_legacy_fallback_total Agent bincode payloads decoded via the legacy compatibility path.\n");
    output.push_str("# TYPE netmonitor_legacy_fallback_total counter\n");
    let _ = writeln!(
        output,
        "netmonitor_legacy_fallback_total {}",
        crate::services::metrics_service::legacy_fallback_total()
    );

    for row in &status_snapshot {
        let host_key = escape_prom_label(&row.host_key);
        let display_name = escape_prom_label(&row.display_name);
        let online = if row.is_online { 1 } else { 0 };
        let _ = writeln!(
            output,
            "netmonitor_host_online{{host_key=\"{host_key}\",display_name=\"{display_name}\"}} {online}"
        );
    }

    for row in &metric_snapshot {
        let host_key = escape_prom_label(&row.host_key);
        let display_name = escape_prom_label(&row.display_name);
        let cpu = row.cpu_usage_percent;
        let mem = row.memory_usage_percent;
        let _ = writeln!(
            output,
            "netmonitor_cpu_usage_percent{{host_key=\"{host_key}\",display_name=\"{display_name}\"}} {cpu:.2}"
        );
        let _ = writeln!(
            output,
            "netmonitor_memory_usage_percent{{host_key=\"{host_key}\",display_name=\"{display_name}\"}} {mem:.2}"
        );
    }

    Ok((
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        output,
    ))
}
