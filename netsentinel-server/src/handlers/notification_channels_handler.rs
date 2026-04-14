use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};

use crate::errors::AppError;
use crate::models::app_state::AppState;
use crate::repositories::notification_channels_repo::{
    self, CreateChannelRequest, NotificationChannelRow, UpdateChannelRequest,
};
use crate::services::auth::{AdminGuard, AuthGuard};

/// GET /api/notification-channels — list all notification channels
pub async fn list_channels(
    _auth: AuthGuard,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<NotificationChannelRow>>, AppError> {
    let channels = notification_channels_repo::get_all(&state.db_pool).await?;
    Ok(Json(channels))
}

/// POST /api/notification-channels — create a new notification channel
pub async fn create_channel(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateChannelRequest>,
) -> Result<Json<NotificationChannelRow>, AppError> {
    validate_channel(&body.channel_type, &body.config)?;
    validate_webhook_ssrf(&body.channel_type, &body.config).await?;
    let channel = notification_channels_repo::create_channel(&state.db_pool, &body).await?;
    tracing::info!(id = channel.id, channel_type = %body.channel_type, "🔔 [Notification] Channel created");
    Ok(Json(channel))
}

/// PUT /api/notification-channels/{id} — update a notification channel
pub async fn update_channel(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
    Json(body): Json<UpdateChannelRequest>,
) -> Result<Json<NotificationChannelRow>, AppError> {
    // If config is being updated, validate it against existing channel type
    if let Some(config) = &body.config {
        let existing = notification_channels_repo::get_by_id(&state.db_pool, id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Notification channel {} not found", id)))?;
        validate_channel(&existing.channel_type, config)?;
        validate_webhook_ssrf(&existing.channel_type, config).await?;
    }
    let channel = notification_channels_repo::update_channel(&state.db_pool, id, &body)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Notification channel {} not found", id)))?;
    tracing::info!(id = id, "🔔 [Notification] Channel updated");
    Ok(Json(channel))
}

/// DELETE /api/notification-channels/{id} — delete a notification channel
pub async fn delete_channel(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
) -> Result<Json<serde_json::Value>, AppError> {
    let deleted = notification_channels_repo::delete_channel(&state.db_pool, id).await?;
    if !deleted {
        return Err(AppError::NotFound(format!(
            "Notification channel {} not found",
            id
        )));
    }
    tracing::info!(id = id, "🔔 [Notification] Channel deleted");
    Ok(Json(serde_json::json!({ "deleted": id })))
}

/// POST /api/notification-channels/{id}/test — send a test message
pub async fn test_channel(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
) -> Result<Json<serde_json::Value>, AppError> {
    let channel = notification_channels_repo::get_by_id(&state.db_pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Notification channel {} not found", id)))?;

    crate::services::alert_service::test_channel(&state.http_client, &channel)
        .await
        .map_err(AppError::BadRequest)?;

    Ok(Json(serde_json::json!({ "success": true })))
}

/// SSRF protection: validate webhook URLs resolve to public IPs only.
async fn validate_webhook_ssrf(
    channel_type: &str,
    config: &serde_json::Value,
) -> Result<(), AppError> {
    if matches!(channel_type, "discord" | "slack")
        && let Some(url) = config.get("webhook_url").and_then(|v| v.as_str())
    {
        crate::services::url_validator::validate_url(url, &["https"])
            .await
            .map_err(|e| AppError::BadRequest(format!("Webhook URL rejected: {e}")))?;
    }
    Ok(())
}

fn validate_channel(channel_type: &str, config: &serde_json::Value) -> Result<(), AppError> {
    if !matches!(channel_type, "discord" | "slack" | "email") {
        return Err(AppError::BadRequest(format!(
            "Unsupported channel_type: {channel_type} (must be 'discord', 'slack', or 'email')"
        )));
    }

    match channel_type {
        "discord" | "slack" => {
            let webhook_url = config
                .get("webhook_url")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if webhook_url.is_empty() {
                return Err(AppError::BadRequest(format!(
                    "{channel_type} channel requires a non-empty 'webhook_url' in config"
                )));
            }
        }
        "email" => {
            for field in ["smtp_host", "from", "to"] {
                let val = config.get(field).and_then(|v| v.as_str()).unwrap_or("");
                if val.is_empty() {
                    return Err(AppError::BadRequest(format!(
                        "Email channel requires a non-empty '{field}' in config"
                    )));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_valid_discord_channel() {
        let config = json!({ "webhook_url": "https://discord.com/api/webhooks/123/abc" });
        assert!(validate_channel("discord", &config).is_ok());
    }

    #[test]
    fn test_discord_missing_webhook() {
        assert!(validate_channel("discord", &json!({})).is_err());
        assert!(validate_channel("discord", &json!({ "webhook_url": "" })).is_err());
    }

    #[test]
    fn test_valid_email_channel() {
        let config = json!({
            "smtp_host": "smtp.example.com",
            "from": "noreply@example.com",
            "to": "admin@example.com"
        });
        assert!(validate_channel("email", &config).is_ok());
    }

    #[test]
    fn test_email_missing_fields() {
        assert!(validate_channel("email", &json!({ "smtp_host": "x", "from": "x" })).is_err());
        assert!(validate_channel("email", &json!({ "smtp_host": "x", "to": "x" })).is_err());
        assert!(validate_channel("email", &json!({ "from": "x", "to": "x" })).is_err());
    }

    #[test]
    fn test_unsupported_channel_type() {
        assert!(validate_channel("telegram", &json!({})).is_err());
    }
}
