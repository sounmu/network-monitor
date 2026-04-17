use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};

use crate::errors::AppError;
use crate::models::app_state::AppState;
use crate::repositories::alert_configs_repo::{self, AlertConfigRow, UpsertAlertRequest};
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

fn validate_alert_request(req: &UpsertAlertRequest) -> Result<(), AppError> {
    // metric_type is validated at deserialization (MetricType enum) — no manual check needed.
    if !(0.0..=100.0).contains(&req.threshold) {
        return Err(AppError::BadRequest(format!(
            "threshold must be between 0 and 100, got {}",
            req.threshold
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::alert_configs_repo::MetricType;

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
        }
    }

    // Invalid metric_type values are now rejected at serde deserialization
    // (MetricType is a closed enum of cpu|memory|disk) — a dedicated runtime
    // test is no longer expressible from Rust and is not needed.

    #[test]
    fn test_valid_alert_config() {
        assert!(validate_alert_request(&make_request(MetricType::Cpu, 80.0, 300, 60)).is_ok());
        assert!(validate_alert_request(&make_request(MetricType::Memory, 90.0, 0, 0)).is_ok());
        assert!(validate_alert_request(&make_request(MetricType::Disk, 0.0, 3600, 86400)).is_ok());
        assert!(validate_alert_request(&make_request(MetricType::Cpu, 100.0, 0, 0)).is_ok());
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
}
