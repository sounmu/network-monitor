use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;

use crate::errors::AppError;
use crate::models::app_state::AppState;
use crate::repositories::alert_configs_repo::{
    self, AlertConfigRow, MetricType, UpsertAlertRequest,
};
use crate::repositories::hosts_repo;
use crate::services::auth::{AdminGuard, UserGuard};
use crate::services::hosts_snapshot;

/// GET /api/alert-configs — get global default alert configs
pub async fn get_global_configs(
    _auth: UserGuard,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<AlertConfigRow>>, AppError> {
    let configs = alert_configs_repo::get_global_configs(&state.db_pool).await?;
    Ok(Json(configs))
}

/// PUT /api/alert-configs — update global default alert configs
pub async fn update_global_configs(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Json(body): Json<Vec<UpsertAlertRequest>>,
) -> Result<Json<Vec<AlertConfigRow>>, AppError> {
    let mut results = Vec::new();
    for req in &body {
        validate_alert_request(req)?;
        let row = alert_configs_repo::upsert_alert_config(&state.db_pool, None, req).await?;
        results.push(row);
    }
    hosts_snapshot::refresh(&state.db_pool, &state.hosts_snapshot).await;
    tracing::info!("🔔 [AlertConfig] Global alert configs updated");
    Ok(Json(results))
}

/// GET /api/alert-configs/{host_key} — get per-host alert config overrides
pub async fn get_host_configs(
    _auth: UserGuard,
    State(state): State<Arc<AppState>>,
    Path(host_key): Path<String>,
) -> Result<Json<Vec<AlertConfigRow>>, AppError> {
    let configs = alert_configs_repo::get_host_configs(&state.db_pool, &host_key).await?;
    Ok(Json(configs))
}

/// PUT /api/alert-configs/{host_key} — upsert per-host alert config overrides
pub async fn update_host_configs(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Path(host_key): Path<String>,
    Json(body): Json<Vec<UpsertAlertRequest>>,
) -> Result<Json<Vec<AlertConfigRow>>, AppError> {
    let mut results = Vec::new();
    for req in &body {
        validate_alert_request(req)?;
        let row =
            alert_configs_repo::upsert_alert_config(&state.db_pool, Some(&host_key), req).await?;
        results.push(row);
    }
    hosts_snapshot::refresh(&state.db_pool, &state.hosts_snapshot).await;
    tracing::info!(host_key = %host_key, "🔔 [AlertConfig] Per-host alert configs updated");
    Ok(Json(results))
}

/// DELETE /api/alert-configs/{host_key} — delete per-host overrides (reverts to global)
pub async fn delete_host_configs(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Path(host_key): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let deleted = alert_configs_repo::delete_host_configs(&state.db_pool, &host_key).await?;
    if !deleted {
        return Err(AppError::NotFound(format!(
            "No alert config overrides found for host: {}",
            host_key
        )));
    }
    hosts_snapshot::refresh(&state.db_pool, &state.hosts_snapshot).await;
    tracing::info!(host_key = %host_key, "🔔 [AlertConfig] Per-host overrides deleted, reverted to global");
    Ok(Json(serde_json::json!({ "deleted": host_key })))
}

/// Request body for bulk host-config updates.
#[derive(Debug, Deserialize)]
pub struct BulkAlertConfigRequest {
    pub host_keys: Vec<String>,
    pub configs: Vec<UpsertAlertRequest>,
}

/// POST /api/alert-configs/bulk — apply the same overrides to every selected host
/// in a single transaction.
pub async fn bulk_update_host_configs(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Json(body): Json<BulkAlertConfigRequest>,
) -> Result<Json<Vec<AlertConfigRow>>, AppError> {
    if body.host_keys.is_empty() {
        return Err(AppError::BadRequest(
            "host_keys must contain at least one entry".into(),
        ));
    }
    if body.host_keys.len() > 500 {
        return Err(AppError::BadRequest(
            "bulk apply is capped at 500 hosts per request".into(),
        ));
    }
    if body.configs.is_empty() {
        return Err(AppError::BadRequest(
            "configs must contain at least one entry".into(),
        ));
    }

    // Validate every rule up-front so we fail before mutating anything.
    for req in &body.configs {
        validate_alert_request(req)?;
    }

    // Make sure every targeted host actually exists; this gives a cleaner 4xx
    // than letting a FK violation surface from Postgres.
    for hk in &body.host_keys {
        if hosts_repo::get_host(&state.db_pool, hk).await?.is_none() {
            return Err(AppError::BadRequest(format!("unknown host_key: {hk}")));
        }
    }

    let rows = alert_configs_repo::bulk_upsert_host_configs(
        &state.db_pool,
        &body.host_keys,
        &body.configs,
    )
    .await?;

    hosts_snapshot::refresh(&state.db_pool, &state.hosts_snapshot).await;
    tracing::info!(
        hosts = body.host_keys.len(),
        rules = body.configs.len(),
        "🔔 [AlertConfig] Bulk overrides applied"
    );
    Ok(Json(rows))
}

fn validate_alert_request(req: &UpsertAlertRequest) -> Result<(), AppError> {
    let (min, max, unit) = threshold_bounds(req.metric_type);
    if !(min..=max).contains(&req.threshold) {
        return Err(AppError::BadRequest(format!(
            "threshold for {metric} must be between {min} and {max}{unit}, got {got}",
            metric = req.metric_type,
            got = req.threshold,
        )));
    }
    if !(0..=3600).contains(&req.sustained_secs) {
        return Err(AppError::BadRequest(format!(
            "sustained_secs must be between 0 and 3600, got {}",
            req.sustained_secs
        )));
    }
    if !(0..=86400).contains(&req.cooldown_secs) {
        return Err(AppError::BadRequest(format!(
            "cooldown_secs must be between 0 and 86400, got {}",
            req.cooldown_secs
        )));
    }
    if let Some(sub) = req.sub_key.as_deref()
        && (sub.is_empty() || sub.len() > 128)
    {
        return Err(AppError::BadRequest(
            "sub_key must be 1-128 characters when provided".into(),
        ));
    }
    Ok(())
}

/// Per-metric threshold range + a human-readable unit label for error messages.
fn threshold_bounds(mt: MetricType) -> (f64, f64, &'static str) {
    match mt {
        MetricType::Cpu | MetricType::Memory | MetricType::Disk | MetricType::Gpu => {
            (0.0, 100.0, "%")
        }
        // Load avg — cap at 64 so an operator with a 32-core host can still
        // set a "10x" alarm without hitting the ceiling.
        MetricType::Load => (0.0, 64.0, ""),
        // Network in bytes/sec — 10 Gbps upper bound.
        MetricType::Network => (0.0, 10_000_000_000.0, " B/s"),
        MetricType::Temperature => (-20.0, 120.0, "°C"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_request(
        metric_type: MetricType,
        threshold: f64,
        sustained_secs: i32,
        cooldown_secs: i32,
    ) -> UpsertAlertRequest {
        UpsertAlertRequest {
            metric_type,
            enabled: true,
            threshold,
            sustained_secs,
            cooldown_secs,
            sub_key: None,
        }
    }

    #[test]
    fn test_valid_alert_config() {
        assert!(validate_alert_request(&make_request(MetricType::Cpu, 80.0, 300, 60)).is_ok());
        assert!(validate_alert_request(&make_request(MetricType::Memory, 90.0, 0, 0)).is_ok());
        assert!(validate_alert_request(&make_request(MetricType::Disk, 0.0, 3600, 86400)).is_ok());
        assert!(validate_alert_request(&make_request(MetricType::Cpu, 100.0, 0, 0)).is_ok());
    }

    #[test]
    fn test_load_threshold_range() {
        assert!(validate_alert_request(&make_request(MetricType::Load, 0.0, 300, 60)).is_ok());
        assert!(validate_alert_request(&make_request(MetricType::Load, 64.0, 300, 60)).is_ok());
        assert!(validate_alert_request(&make_request(MetricType::Load, -1.0, 300, 60)).is_err());
        assert!(validate_alert_request(&make_request(MetricType::Load, 65.0, 300, 60)).is_err());
    }

    #[test]
    fn test_temperature_threshold_range() {
        assert!(
            validate_alert_request(&make_request(MetricType::Temperature, -20.0, 300, 60)).is_ok()
        );
        assert!(
            validate_alert_request(&make_request(MetricType::Temperature, 120.0, 300, 60)).is_ok()
        );
        assert!(
            validate_alert_request(&make_request(MetricType::Temperature, -21.0, 300, 60)).is_err()
        );
        assert!(
            validate_alert_request(&make_request(MetricType::Temperature, 121.0, 300, 60)).is_err()
        );
    }

    #[test]
    fn test_network_threshold_range() {
        assert!(validate_alert_request(&make_request(MetricType::Network, 0.0, 300, 60)).is_ok());
        assert!(
            validate_alert_request(&make_request(
                MetricType::Network,
                10_000_000_000.0,
                300,
                60
            ))
            .is_ok()
        );
        assert!(
            validate_alert_request(&make_request(
                MetricType::Network,
                10_000_000_001.0,
                300,
                60
            ))
            .is_err()
        );
    }

    #[test]
    fn test_threshold_out_of_range() {
        assert!(validate_alert_request(&make_request(MetricType::Cpu, -1.0, 300, 60)).is_err());
        assert!(validate_alert_request(&make_request(MetricType::Cpu, 101.0, 300, 60)).is_err());
    }

    #[test]
    fn test_sustained_secs_out_of_range() {
        assert!(validate_alert_request(&make_request(MetricType::Cpu, 80.0, -1, 60)).is_err());
        assert!(validate_alert_request(&make_request(MetricType::Cpu, 80.0, 3601, 60)).is_err());
    }

    #[test]
    fn test_cooldown_secs_out_of_range() {
        assert!(validate_alert_request(&make_request(MetricType::Cpu, 80.0, 300, -1)).is_err());
        assert!(validate_alert_request(&make_request(MetricType::Cpu, 80.0, 300, 86401)).is_err());
    }

    #[test]
    fn test_sub_key_bounds() {
        let mut req = make_request(MetricType::Temperature, 85.0, 120, 600);
        req.sub_key = Some(String::new());
        assert!(validate_alert_request(&req).is_err());
        req.sub_key = Some("a".repeat(129));
        assert!(validate_alert_request(&req).is_err());
        req.sub_key = Some("cpu0".into());
        assert!(validate_alert_request(&req).is_ok());
    }
}
