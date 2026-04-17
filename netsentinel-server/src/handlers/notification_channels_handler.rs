use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};

use crate::errors::AppError;
use crate::models::app_state::AppState;
use crate::repositories::notification_channels_repo::{
    self, ChannelType, CreateChannelRequest, NotificationChannelRow, UpdateChannelRequest,
};
use crate::services::auth::AdminGuard;

/// GET /api/notification-channels — list all notification channels
/// Requires AdminGuard to prevent agent JWTs from reading SMTP credentials.
pub async fn list_channels(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<NotificationChannelRow>>, AppError> {
    let mut channels = notification_channels_repo::get_all(&state.db_pool).await?;
    // Redact SMTP password from email channels before returning
    for channel in &mut channels {
        if channel.channel_type == ChannelType::Email
            && let Some(obj) = channel.config.as_object_mut()
            && obj.contains_key("smtp_pass")
        {
            obj.insert("smtp_pass".into(), serde_json::json!("********"));
        }
    }
    Ok(Json(channels))
}

/// POST /api/notification-channels — create a new notification channel
pub async fn create_channel(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateChannelRequest>,
) -> Result<Json<NotificationChannelRow>, AppError> {
    validate_channel(body.channel_type, &body.config)?;
    validate_webhook_ssrf(body.channel_type, &body.config).await?;
    let channel = notification_channels_repo::create_channel(&state.db_pool, &body).await?;
    tracing::info!(id = channel.id, channel_type = ?body.channel_type, "🔔 [Notification] Channel created");
    Ok(Json(channel))
}

/// PUT /api/notification-channels/{id} — update a notification channel
pub async fn update_channel(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
    Json(mut body): Json<UpdateChannelRequest>,
) -> Result<Json<NotificationChannelRow>, AppError> {
    // If config is being updated, validate it against existing channel type.
    // Also merge redacted placeholders back to the stored value — the GET
    // handler masks `smtp_pass` as "********"; without this merge, a naive
    // "edit channel name" round-trip (load → submit) would overwrite the
    // real SMTP password with the literal mask and break all future mails.
    if let Some(config) = &mut body.config {
        let existing = notification_channels_repo::get_by_id(&state.db_pool, id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Notification channel {} not found", id)))?;
        preserve_redacted_secrets(existing.channel_type, &existing.config, config);
        validate_channel(existing.channel_type, config)?;
        validate_webhook_ssrf(existing.channel_type, config).await?;
    }
    let channel = notification_channels_repo::update_channel(&state.db_pool, id, &body)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Notification channel {} not found", id)))?;
    tracing::info!(id = id, "🔔 [Notification] Channel updated");
    Ok(Json(channel))
}

/// Placeholder value the GET handler substitutes for sensitive fields.
/// Keep in sync with `notification_channels_repo::redact_secrets`.
const REDACTED_PLACEHOLDER: &str = "********";

/// Replace the redacted placeholder with the stored secret for fields that the
/// server masks on read. Currently only `smtp_pass` on Email channels, but the
/// loop shape tolerates future additions (e.g. Discord bot tokens) without
/// touching the call site.
fn preserve_redacted_secrets(
    channel_type: ChannelType,
    stored: &serde_json::Value,
    incoming: &mut serde_json::Value,
) {
    let redacted_fields: &[&str] = match channel_type {
        ChannelType::Email => &["smtp_pass"],
        ChannelType::Discord | ChannelType::Slack => &[],
    };
    let Some(incoming_obj) = incoming.as_object_mut() else {
        return;
    };
    for field in redacted_fields {
        if incoming_obj.get(*field).and_then(|v| v.as_str()) == Some(REDACTED_PLACEHOLDER)
            && let Some(original) = stored.get(*field).cloned()
        {
            incoming_obj.insert((*field).to_string(), original);
        }
    }
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

/// Ports that must never be an SMTP target. SMTP standard ports (25, 465,
/// 587, 2525) are permitted; any admin pointing at a database/cache/SSH
/// port is almost certainly probing internal services rather than sending
/// mail. Paired with `url_validator::validate_host` (private-IP block) at
/// both handler + runtime entry points for defense in depth.
const DISALLOWED_SMTP_PORTS: &[u16] = &[22, 80, 443, 3306, 5432, 6379, 11211, 27017];

/// SSRF protection: validate webhook URLs and SMTP endpoints resolve to
/// public IPs only. Applied at handler time (blocks bad configs from being
/// saved) and mirrored in `alert_service::send_email` at send time
/// (defense in depth — config may have been saved before a new blocklist
/// entry was added).
async fn validate_webhook_ssrf(
    channel_type: ChannelType,
    config: &serde_json::Value,
) -> Result<(), AppError> {
    match channel_type {
        ChannelType::Discord | ChannelType::Slack => {
            if let Some(url) = config.get("webhook_url").and_then(|v| v.as_str()) {
                crate::services::url_validator::validate_url(url, &["https"])
                    .await
                    .map_err(|e| AppError::BadRequest(format!("Webhook URL rejected: {e}")))?;
            }
        }
        ChannelType::Email => {
            let smtp_host = config
                .get("smtp_host")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let smtp_port = config
                .get("smtp_port")
                .and_then(|v| v.as_u64())
                .unwrap_or(587) as u16;
            if smtp_host.is_empty() {
                return Ok(()); // `validate_channel` handles the "missing required" error.
            }
            if DISALLOWED_SMTP_PORTS.contains(&smtp_port) {
                return Err(AppError::BadRequest(format!(
                    "SMTP port {smtp_port} is not allowed (reserved for non-SMTP services)"
                )));
            }
            crate::services::url_validator::validate_host(&format!("{smtp_host}:{smtp_port}"))
                .await
                .map_err(|e| AppError::BadRequest(format!("SMTP host rejected: {e}")))?;
        }
    }
    Ok(())
}

/// Validate channel config based on channel type.
/// With `ChannelType` as an enum, the type-level check replaces the old
/// `matches!(channel_type, "discord" | "slack" | "email")` guard.
fn validate_channel(channel_type: ChannelType, config: &serde_json::Value) -> Result<(), AppError> {
    match channel_type {
        ChannelType::Discord | ChannelType::Slack => {
            let webhook_url = config
                .get("webhook_url")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if webhook_url.is_empty() {
                return Err(AppError::BadRequest(format!(
                    "{channel_type:?} channel requires a non-empty 'webhook_url' in config"
                )));
            }
        }
        ChannelType::Email => {
            for field in ["smtp_host", "from", "to"] {
                let val = config.get(field).and_then(|v| v.as_str()).unwrap_or("");
                if val.is_empty() {
                    return Err(AppError::BadRequest(format!(
                        "Email channel requires a non-empty '{field}' in config"
                    )));
                }
            }
        }
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
        assert!(validate_channel(ChannelType::Discord, &config).is_ok());
    }

    #[test]
    fn test_discord_missing_webhook() {
        assert!(validate_channel(ChannelType::Discord, &json!({})).is_err());
        assert!(validate_channel(ChannelType::Discord, &json!({ "webhook_url": "" })).is_err());
    }

    #[test]
    fn test_valid_email_channel() {
        let config = json!({
            "smtp_host": "smtp.example.com",
            "from": "noreply@example.com",
            "to": "admin@example.com"
        });
        assert!(validate_channel(ChannelType::Email, &config).is_ok());
    }

    #[test]
    fn test_email_missing_fields() {
        assert!(
            validate_channel(
                ChannelType::Email,
                &json!({ "smtp_host": "x", "from": "x" })
            )
            .is_err()
        );
        assert!(
            validate_channel(ChannelType::Email, &json!({ "smtp_host": "x", "to": "x" })).is_err()
        );
        assert!(validate_channel(ChannelType::Email, &json!({ "from": "x", "to": "x" })).is_err());
    }
}
