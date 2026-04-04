use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};

use crate::errors::AppError;
use crate::models::app_state::AppState;
use crate::repositories::alert_configs_repo::{self, AlertConfigRow, UpsertAlertRequest};
use crate::services::auth::AuthGuard;

/// GET /api/alert-configs — get global default alert configs
pub async fn get_global_configs(
    _auth: AuthGuard,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<AlertConfigRow>>, AppError> {
    let configs = alert_configs_repo::get_global_configs(&state.db_pool).await?;
    Ok(Json(configs))
}

/// PUT /api/alert-configs — update global default alert configs
pub async fn update_global_configs(
    _auth: AuthGuard,
    State(state): State<Arc<AppState>>,
    Json(body): Json<Vec<UpsertAlertRequest>>,
) -> Result<Json<Vec<AlertConfigRow>>, AppError> {
    let mut results = Vec::new();
    for req in &body {
        if !matches!(req.metric_type.as_str(), "cpu" | "memory" | "disk") {
            return Err(AppError::BadRequest(format!(
                "Unsupported metric_type: {} (must be 'cpu', 'memory', or 'disk')",
                req.metric_type
            )));
        }
        let row = alert_configs_repo::upsert_alert_config(&state.db_pool, None, req).await?;
        results.push(row);
    }
    tracing::info!("🔔 [AlertConfig] Global alert configs updated");
    Ok(Json(results))
}

/// GET /api/alert-configs/{host_key} — get per-host alert config overrides
pub async fn get_host_configs(
    _auth: AuthGuard,
    State(state): State<Arc<AppState>>,
    Path(host_key): Path<String>,
) -> Result<Json<Vec<AlertConfigRow>>, AppError> {
    let configs = alert_configs_repo::get_host_configs(&state.db_pool, &host_key).await?;
    Ok(Json(configs))
}

/// PUT /api/alert-configs/{host_key} — upsert per-host alert config overrides
pub async fn update_host_configs(
    _auth: AuthGuard,
    State(state): State<Arc<AppState>>,
    Path(host_key): Path<String>,
    Json(body): Json<Vec<UpsertAlertRequest>>,
) -> Result<Json<Vec<AlertConfigRow>>, AppError> {
    let mut results = Vec::new();
    for req in &body {
        if !matches!(req.metric_type.as_str(), "cpu" | "memory" | "disk") {
            return Err(AppError::BadRequest(format!(
                "Unsupported metric_type: {}",
                req.metric_type
            )));
        }
        let row =
            alert_configs_repo::upsert_alert_config(&state.db_pool, Some(&host_key), req).await?;
        results.push(row);
    }
    tracing::info!(host_key = %host_key, "🔔 [AlertConfig] Per-host alert configs updated");
    Ok(Json(results))
}

/// DELETE /api/alert-configs/{host_key} — delete per-host overrides (reverts to global)
pub async fn delete_host_configs(
    _auth: AuthGuard,
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
    tracing::info!(host_key = %host_key, "🔔 [AlertConfig] Per-host overrides deleted, reverted to global");
    Ok(Json(serde_json::json!({ "deleted": host_key })))
}
