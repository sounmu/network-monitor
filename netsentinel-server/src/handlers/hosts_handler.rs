use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};

use crate::errors::AppError;
use crate::models::app_state::AppState;
use crate::repositories::hosts_repo::{self, CreateHostRequest, HostRow, UpdateHostRequest};
use crate::services::auth::{AdminGuard, UserGuard};
use crate::services::hosts_snapshot;
use url::Host;

// ── Validation limits ────────────────────────
const MAX_KEY_LEN: usize = 255;
const MAX_NAME_LEN: usize = 255;

/// Validate that `host_key` is a safe `host:port` string — no path, query, or fragment.
/// Prevents SSRF via path/query injection when the scraper builds `http://{host_key}/metrics`.
fn validate_host_key_format(host_key: &str) -> Result<(), AppError> {
    // Must not contain path separators, query, or fragment characters
    if host_key.contains('/')
        || host_key.contains('?')
        || host_key.contains('#')
        || host_key.contains('@')
    {
        return Err(AppError::BadRequest(
            "host_key must be host:port format (no path, query, fragment, or @)".to_string(),
        ));
    }
    // Must parse as a valid socket address (ip:port or hostname:port)
    // Try as SocketAddr first (handles IP:port), then as host:port string
    if host_key.parse::<std::net::SocketAddr>().is_err() {
        // Not a raw IP:port — check it's at least hostname:port
        let Some((host, port_str)) = host_key.rsplit_once(':') else {
            return Err(AppError::BadRequest(
                "host_key must include a port (host:port)".to_string(),
            ));
        };
        if host.is_empty() {
            return Err(AppError::BadRequest(
                "host_key host part must not be empty".to_string(),
            ));
        }
        if port_str.parse::<u16>().is_err() {
            return Err(AppError::BadRequest(
                "host_key port must be a valid number (1-65535)".to_string(),
            ));
        }
        if host.contains(':') {
            return Err(AppError::BadRequest(
                "host_key host must be a hostname or bracketed IPv6 literal".to_string(),
            ));
        }
        Host::parse(host).map_err(|_| {
            AppError::BadRequest("host_key host part is not a valid hostname".to_string())
        })?;
    }
    Ok(())
}

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

fn validate_scrape_interval(scrape_interval_secs: i32) -> Result<(), AppError> {
    if scrape_interval_secs <= 0 {
        return Err(AppError::BadRequest(
            "scrape_interval_secs must be greater than 0".to_string(),
        ));
    }
    Ok(())
}

/// GET /api/hosts — list all hosts (includes is_online status)
pub async fn list_hosts(
    _auth: UserGuard,
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
    validate_host_key_format(&body.host_key)?;
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
    validate_scrape_interval(body.scrape_interval_secs)?;
    validate_ports(&body.ports)?;

    let host = hosts_repo::create_host(&state.db_pool, &body).await?;

    // Pre-register in last_known_status as offline
    state.pre_populate_status(std::slice::from_ref(&host));
    // Refresh the snapshot so the scraper picks up the new host on its
    // next cycle rather than waiting up to 60 s for the background tick.
    hosts_snapshot::refresh(&state.db_pool, &state.hosts_snapshot).await;

    tracing::info!(host_key = %host.host_key, "🆕 [Hosts] New host registered");
    Ok(Json(host))
}

/// GET /api/hosts/{host_key} — get a specific host's full config
pub async fn get_host(
    _auth: UserGuard,
    State(state): State<Arc<AppState>>,
    Path(host_key): Path<String>,
) -> Result<Json<HostRow>, AppError> {
    let host = hosts_repo::get_host(&state.db_pool, &host_key)
        .await?
        .ok_or_else(|| AppError::NotFound("Host not found".to_string()))?;
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
    if let Some(scrape_interval_secs) = body.scrape_interval_secs {
        validate_scrape_interval(scrape_interval_secs)?;
    }
    if let Some(ref ports) = body.ports {
        validate_ports(ports)?;
    }

    let host = hosts_repo::update_host(&state.db_pool, &host_key, &body)
        .await?
        .ok_or_else(|| AppError::NotFound("Host not found".to_string()))?;

    if let Ok(mut lks) = state.last_known_status.write()
        && let Some(arc) = lks.get_mut(&host.host_key)
    {
        let status = std::sync::Arc::make_mut(arc);
        status.display_name = host.display_name.clone();
        status.scrape_interval_secs = u64::try_from(host.scrape_interval_secs)
            .ok()
            .filter(|secs| *secs > 0)
            .unwrap_or(state.scrape_interval_secs);
        status.os_info = host.os_info.clone();
        status.cpu_model = host.cpu_model.clone();
        status.memory_total_mb = host.memory_total_mb;
        status.boot_time = host.boot_time;
        status.ip_address = host.ip_address.clone();
    }

    hosts_snapshot::refresh(&state.db_pool, &state.hosts_snapshot).await;

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
        return Err(AppError::NotFound("Host not found".to_string()));
    }

    // Clean up in-memory caches for the deleted host
    if let Ok(mut lks) = state.last_known_status.write() {
        lks.remove(&host_key);
    }
    if let Ok(mut store) = state.store.write() {
        store.hosts.remove(&host_key);
    }
    hosts_snapshot::refresh(&state.db_pool, &state.hosts_snapshot).await;

    // `?host_key` (Debug) escapes control chars so a path param containing
    // `\r\n` cannot forge additional log lines (CRLF log injection).
    // `host.host_key` / `host.display_name` fields sourced from DB rows pass
    // through `validate_host_key_format` on write, so `%` is fine for them;
    // the bare `host_key` path extractor has not been re-validated here.
    tracing::info!(host_key = ?host_key, "🗑️ [Hosts] Host deleted");
    Ok(Json(serde_json::json!({ "deleted": host_key })))
}
