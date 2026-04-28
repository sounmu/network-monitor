use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::errors::AppError;
use crate::models::app_state::AppState;
use crate::repositories::{http_monitors_repo, ping_monitors_repo};
use crate::services::auth::{AdminGuard, UserGuard};
use crate::services::monitors_snapshot;

// ──────────────────────────────────────────────
// HTTP Monitors
// ──────────────────────────────────────────────

/// GET /api/http-monitors
pub async fn list_http_monitors(
    _auth: UserGuard,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<http_monitors_repo::HttpMonitor>>, AppError> {
    let monitors = http_monitors_repo::get_all(&state.db_pool).await?;
    Ok(Json(monitors))
}

/// POST /api/http-monitors (admin only)
pub async fn create_http_monitor(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Json(body): Json<http_monitors_repo::CreateHttpMonitorRequest>,
) -> Result<Json<http_monitors_repo::HttpMonitor>, AppError> {
    validate_http_monitor_request(
        &body.url,
        body.expected_status,
        body.interval_secs,
        body.timeout_ms,
    )?;
    validate_monitor_url_ssrf(&body.url).await?;
    let monitor = http_monitors_repo::create(&state.db_pool, &body).await?;
    monitors_snapshot::refresh(&state.db_pool, &state.monitors_snapshot).await;
    tracing::info!(id = monitor.id, url = %monitor.url, "🌐 [HTTP Monitor] Created");
    Ok(Json(monitor))
}

/// PUT /api/http-monitors/{id} (admin only)
pub async fn update_http_monitor(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
    Json(body): Json<http_monitors_repo::UpdateHttpMonitorRequest>,
) -> Result<Json<http_monitors_repo::HttpMonitor>, AppError> {
    validate_http_monitor_request(
        body.url.as_deref().unwrap_or("http://placeholder"),
        body.expected_status,
        body.interval_secs,
        body.timeout_ms,
    )?;
    if let Some(url) = &body.url {
        validate_monitor_url_ssrf(url).await?;
    }
    let monitor = http_monitors_repo::update(&state.db_pool, id, &body)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("HTTP monitor {} not found", id)))?;
    monitors_snapshot::refresh(&state.db_pool, &state.monitors_snapshot).await;
    Ok(Json(monitor))
}

/// DELETE /api/http-monitors/{id} (admin only)
pub async fn delete_http_monitor(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
) -> Result<Json<serde_json::Value>, AppError> {
    let deleted = http_monitors_repo::delete(&state.db_pool, id).await?;
    if !deleted {
        return Err(AppError::NotFound(format!("HTTP monitor {} not found", id)));
    }
    monitors_snapshot::refresh(&state.db_pool, &state.monitors_snapshot).await;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

#[derive(Deserialize)]
pub struct ResultsQuery {
    pub limit: Option<i64>,
}

/// GET /api/http-monitors/{id}/results
pub async fn get_http_results(
    _auth: UserGuard,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
    Query(query): Query<ResultsQuery>,
) -> Result<Json<Vec<http_monitors_repo::HttpMonitorResult>>, AppError> {
    let limit = query.limit.unwrap_or(50).min(200);
    let results = http_monitors_repo::get_results(&state.db_pool, id, limit).await?;
    Ok(Json(results))
}

/// GET /api/http-monitors/summaries
pub async fn get_http_summaries(
    _auth: UserGuard,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<http_monitors_repo::HttpMonitorSummary>>, AppError> {
    let summaries = http_monitors_repo::get_summaries(&state.db_pool).await?;
    Ok(Json(summaries))
}

// ──────────────────────────────────────────────
// Ping Monitors
// ──────────────────────────────────────────────

/// GET /api/ping-monitors
pub async fn list_ping_monitors(
    _auth: UserGuard,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ping_monitors_repo::PingMonitor>>, AppError> {
    let monitors = ping_monitors_repo::get_all(&state.db_pool).await?;
    Ok(Json(monitors))
}

/// POST /api/ping-monitors (admin only)
pub async fn create_ping_monitor(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Json(body): Json<ping_monitors_repo::CreatePingMonitorRequest>,
) -> Result<Json<ping_monitors_repo::PingMonitor>, AppError> {
    validate_ping_monitor_request(&body.host, body.interval_secs, body.timeout_ms)?;
    validate_monitor_host_ssrf(&body.host).await?;
    let monitor = ping_monitors_repo::create(&state.db_pool, &body).await?;
    monitors_snapshot::refresh(&state.db_pool, &state.monitors_snapshot).await;
    tracing::info!(id = monitor.id, host = %monitor.host, "🏓 [Ping Monitor] Created");
    Ok(Json(monitor))
}

/// PUT /api/ping-monitors/{id} (admin only)
pub async fn update_ping_monitor(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
    Json(body): Json<ping_monitors_repo::UpdatePingMonitorRequest>,
) -> Result<Json<ping_monitors_repo::PingMonitor>, AppError> {
    validate_ping_monitor_request(
        body.host.as_deref().unwrap_or("placeholder"),
        body.interval_secs,
        body.timeout_ms,
    )?;
    if let Some(host) = &body.host {
        validate_monitor_host_ssrf(host).await?;
    }
    let monitor = ping_monitors_repo::update(&state.db_pool, id, &body)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Ping monitor {} not found", id)))?;
    monitors_snapshot::refresh(&state.db_pool, &state.monitors_snapshot).await;
    Ok(Json(monitor))
}

/// DELETE /api/ping-monitors/{id} (admin only)
pub async fn delete_ping_monitor(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
) -> Result<Json<serde_json::Value>, AppError> {
    let deleted = ping_monitors_repo::delete(&state.db_pool, id).await?;
    if !deleted {
        return Err(AppError::NotFound(format!("Ping monitor {} not found", id)));
    }
    monitors_snapshot::refresh(&state.db_pool, &state.monitors_snapshot).await;
    Ok(Json(serde_json::json!({ "deleted": id })))
}

/// GET /api/ping-monitors/{id}/results
pub async fn get_ping_results(
    _auth: UserGuard,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
    Query(query): Query<ResultsQuery>,
) -> Result<Json<Vec<ping_monitors_repo::PingResult>>, AppError> {
    let limit = query.limit.unwrap_or(50).min(200);
    let results = ping_monitors_repo::get_results(&state.db_pool, id, limit).await?;
    Ok(Json(results))
}

/// GET /api/ping-monitors/summaries
pub async fn get_ping_summaries(
    _auth: UserGuard,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ping_monitors_repo::PingMonitorSummary>>, AppError> {
    let summaries = ping_monitors_repo::get_summaries(&state.db_pool).await?;
    Ok(Json(summaries))
}

// ──────────────────────────────────────────────
// Validation helpers
// ──────────────────────────────────────────────

/// SSRF protection: validate HTTP monitor URL resolves to public IPs only.
async fn validate_monitor_url_ssrf(url: &str) -> Result<(), AppError> {
    crate::services::url_validator::validate_url(url, &["http", "https"])
        .await
        .map_err(|e| AppError::BadRequest(format!("Monitor URL rejected: {e}")))
}

/// SSRF protection: validate ping monitor host resolves to public IPs only.
async fn validate_monitor_host_ssrf(host: &str) -> Result<(), AppError> {
    crate::services::url_validator::validate_host(host)
        .await
        .map_err(|e| AppError::BadRequest(format!("Monitor host rejected: {e}")))
}

fn validate_http_monitor_request(
    url: &str,
    expected_status: Option<i32>,
    interval_secs: Option<i32>,
    timeout_ms: Option<i32>,
) -> Result<(), AppError> {
    if url.is_empty() {
        return Err(AppError::BadRequest("URL is required".into()));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(AppError::BadRequest(
            "URL must start with http:// or https://".into(),
        ));
    }
    if let Some(status) = expected_status
        && !(100..=599).contains(&status)
    {
        return Err(AppError::BadRequest(format!(
            "expected_status must be between 100 and 599, got {status}"
        )));
    }
    validate_interval_and_timeout(interval_secs, timeout_ms)
}

fn validate_ping_monitor_request(
    host: &str,
    interval_secs: Option<i32>,
    timeout_ms: Option<i32>,
) -> Result<(), AppError> {
    if host.is_empty() {
        return Err(AppError::BadRequest("Host is required".into()));
    }
    if host.len() > 255 {
        return Err(AppError::BadRequest(
            "Host must be 255 characters or fewer".into(),
        ));
    }
    validate_interval_and_timeout(interval_secs, timeout_ms)
}

fn validate_interval_and_timeout(
    interval_secs: Option<i32>,
    timeout_ms: Option<i32>,
) -> Result<(), AppError> {
    if let Some(interval) = interval_secs
        && !(10..=3600).contains(&interval)
    {
        return Err(AppError::BadRequest(format!(
            "interval_secs must be between 10 and 3600, got {interval}"
        )));
    }
    if let Some(timeout) = timeout_ms
        && !(1000..=30000).contains(&timeout)
    {
        return Err(AppError::BadRequest(format!(
            "timeout_ms must be between 1000 and 30000, got {timeout}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_http_monitor() {
        assert!(
            validate_http_monitor_request("http://example.com", Some(200), Some(60), Some(5000))
                .is_ok()
        );
        assert!(validate_http_monitor_request("https://example.com", None, None, None).is_ok());
    }

    #[test]
    fn test_http_monitor_invalid_url() {
        assert!(validate_http_monitor_request("", None, None, None).is_err());
        assert!(validate_http_monitor_request("ftp://example.com", None, None, None).is_err());
        assert!(validate_http_monitor_request("example.com", None, None, None).is_err());
    }

    #[test]
    fn test_http_monitor_invalid_status() {
        assert!(validate_http_monitor_request("http://x.com", Some(99), None, None).is_err());
        assert!(validate_http_monitor_request("http://x.com", Some(600), None, None).is_err());
    }

    #[test]
    fn test_monitor_interval_range() {
        assert!(validate_interval_and_timeout(Some(9), None).is_err());
        assert!(validate_interval_and_timeout(Some(10), None).is_ok());
        assert!(validate_interval_and_timeout(Some(3600), None).is_ok());
        assert!(validate_interval_and_timeout(Some(3601), None).is_err());
    }

    #[test]
    fn test_monitor_timeout_range() {
        assert!(validate_interval_and_timeout(None, Some(999)).is_err());
        assert!(validate_interval_and_timeout(None, Some(1000)).is_ok());
        assert!(validate_interval_and_timeout(None, Some(30000)).is_ok());
        assert!(validate_interval_and_timeout(None, Some(30001)).is_err());
    }

    #[test]
    fn test_valid_ping_monitor() {
        assert!(validate_ping_monitor_request("192.168.1.1", Some(60), Some(5000)).is_ok());
    }

    #[test]
    fn test_ping_monitor_empty_host() {
        assert!(validate_ping_monitor_request("", None, None).is_err());
    }

    #[test]
    fn test_ping_monitor_host_too_long() {
        let long_host = "a".repeat(256);
        assert!(validate_ping_monitor_request(&long_host, None, None).is_err());
    }
}
