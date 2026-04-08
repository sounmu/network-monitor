use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};

use crate::errors::AppError;
use crate::models::app_state::AppState;
use crate::repositories::hosts_repo::{self, CreateHostRequest, HostRow, UpdateHostRequest};
use crate::services::auth::{AdminGuard, AuthGuard};

// ── Validation limits ────────────────────────
const MAX_KEY_LEN: usize = 255;
const MAX_NAME_LEN: usize = 255;

fn validate_ports(ports: &[i32]) -> Result<(), AppError> {
    for &p in ports {
        if !(1..=65535).contains(&p) {
            return Err(AppError::BadRequest(format!(
                "Port {} is out of range (1–65535)",
                p
            )));
        }
    }
    Ok(())
}

/// GET /api/hosts — list all hosts (includes is_online status)
pub async fn list_hosts(
    _auth: AuthGuard,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<crate::repositories::metrics_repo::HostSummary>>, AppError> {
    let hosts = crate::repositories::metrics_repo::fetch_host_summaries(&state.db_pool).await?;
    Ok(Json(hosts))
}

/// POST /api/hosts — register a new host (admin only)
pub async fn create_host(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateHostRequest>,
) -> Result<Json<HostRow>, AppError> {
    if body.host_key.trim().is_empty() {
        return Err(AppError::BadRequest(
            "host_key must not be empty".to_string(),
        ));
    }
    if body.host_key.len() > MAX_KEY_LEN {
        return Err(AppError::BadRequest(format!(
            "host_key must not exceed {} characters",
            MAX_KEY_LEN
        )));
    }
    if body.display_name.len() > MAX_NAME_LEN {
        return Err(AppError::BadRequest(format!(
            "display_name must not exceed {} characters",
            MAX_NAME_LEN
        )));
    }
    validate_ports(&body.ports)?;

    let host = hosts_repo::create_host(&state.db_pool, &body)
        .await
        .map_err(|e| {
            if e.to_string().contains("duplicate key") {
                AppError::Conflict(format!("host_key already exists: {}", body.host_key))
            } else {
                AppError::Internal(format!("Failed to create host: {}", e))
            }
        })?;

    // Pre-register in last_known_status as offline
    state.pre_populate_status(std::slice::from_ref(&host));

    tracing::info!(host_key = %host.host_key, "🆕 [Hosts] New host registered");
    Ok(Json(host))
}

/// GET /api/hosts/{host_key} — get a specific host's full config
pub async fn get_host(
    _auth: AuthGuard,
    State(state): State<Arc<AppState>>,
    Path(host_key): Path<String>,
) -> Result<Json<HostRow>, AppError> {
    let host = hosts_repo::get_host(&state.db_pool, &host_key)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Host not found: {}", host_key)))?;
    Ok(Json(host))
}

/// PUT /api/hosts/{host_key} — update host config (admin only)
pub async fn update_host(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Path(host_key): Path<String>,
    Json(body): Json<UpdateHostRequest>,
) -> Result<Json<HostRow>, AppError> {
    if let Some(ref name) = body.display_name
        && name.len() > MAX_NAME_LEN
    {
        return Err(AppError::BadRequest(format!(
            "display_name must not exceed {} characters",
            MAX_NAME_LEN
        )));
    }
    if let Some(ref ports) = body.ports {
        validate_ports(ports)?;
    }

    let host = hosts_repo::update_host(&state.db_pool, &host_key, &body)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Host not found: {}", host_key)))?;

    tracing::info!(host_key = %host.host_key, "✏️ [Hosts] Host config updated");
    Ok(Json(host))
}

/// DELETE /api/hosts/{host_key} — delete a host (admin only)
pub async fn delete_host(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Path(host_key): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let deleted = hosts_repo::delete_host(&state.db_pool, &host_key).await?;
    if !deleted {
        return Err(AppError::NotFound(format!("Host not found: {}", host_key)));
    }

    // Clean up in-memory caches for the deleted host
    if let Ok(mut lks) = state.last_known_status.write() {
        lks.remove(&host_key);
    }
    if let Ok(mut store) = state.store.write() {
        store.hosts.remove(&host_key);
    }

    tracing::info!(host_key = %host_key, "🗑️ [Hosts] Host deleted");
    Ok(Json(serde_json::json!({ "deleted": host_key })))
}
