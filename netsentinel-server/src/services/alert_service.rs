use reqwest::Client;
use sqlx::PgPool;

use crate::repositories::notification_channels_repo::{self, ChannelType, NotificationChannelRow};
use crate::services::url_validator;

// ──────────────────────────────────────────────
// Multi-channel alert delivery
// ──────────────────────────────────────────────

/// Send an alert message to all enabled notification channels from the DB.
pub async fn send_alert(client: &Client, pool: &PgPool, message: &str) {
    let channels = notification_channels_repo::get_enabled(pool)
        .await
        .unwrap_or_default();
    for channel in &channels {
        match channel.channel_type {
            ChannelType::Discord => {
                if let Some(url) = channel.config.get("webhook_url").and_then(|v| v.as_str()) {
                    send_discord(client, url, message).await;
                }
            }
            ChannelType::Slack => {
                if let Some(url) = channel.config.get("webhook_url").and_then(|v| v.as_str()) {
                    send_slack(client, url, message).await;
                }
            }
            ChannelType::Email => {
                send_email(&channel.config, message).await;
            }
        }
    }
}

/// Test a specific notification channel by sending a test message
pub async fn test_channel(client: &Client, channel: &NotificationChannelRow) -> Result<(), String> {
    let test_msg = format!(
        "🔔 **[Test]** Notification channel `{}` is working!",
        channel.name
    );

    match channel.channel_type {
        ChannelType::Discord => {
            let url = channel
                .config
                .get("webhook_url")
                .and_then(|v| v.as_str())
                .ok_or("Missing webhook_url in config")?;
            if send_discord(client, url, &test_msg).await {
                Ok(())
            } else {
                Err("Discord webhook request failed".to_string())
            }
        }
        ChannelType::Slack => {
            let url = channel
                .config
                .get("webhook_url")
                .and_then(|v| v.as_str())
                .ok_or("Missing webhook_url in config")?;
            if send_slack(client, url, &test_msg).await {
                Ok(())
            } else {
                Err("Slack webhook request failed".to_string())
            }
        }
        ChannelType::Email => {
            send_email(&channel.config, &test_msg).await;
            Ok(())
        }
    }
}

// ──────────────────────────────────────────────
// Channel implementations
// ──────────────────────────────────────────────

async fn send_discord(client: &Client, webhook_url: &str, message: &str) -> bool {
    // Defense-in-depth: re-validate URL at runtime to prevent SSRF via DNS rebinding
    if let Err(e) = url_validator::validate_url(webhook_url, &["https"]).await {
        tracing::error!(channel = "discord", err = %e, "⚠️ [Alert] Webhook URL failed SSRF validation");
        return false;
    }

    let body = serde_json::json!({ "content": message });

    match client.post(webhook_url).json(&body).send().await {
        Ok(response) => {
            if response.status().is_success() {
                tracing::info!(channel = "discord", "🔔 [Alert Sent]");
                true
            } else {
                tracing::error!(channel = "discord", status = %response.status(), "⚠️ [Alert Failed]");
                false
            }
        }
        Err(e) => {
            tracing::error!(channel = "discord", err = ?e, "⚠️ [Alert Error]");
            false
        }
    }
}

async fn send_slack(client: &Client, webhook_url: &str, message: &str) -> bool {
    // Defense-in-depth: re-validate URL at runtime to prevent SSRF via DNS rebinding
    if let Err(e) = url_validator::validate_url(webhook_url, &["https"]).await {
        tracing::error!(channel = "slack", err = %e, "⚠️ [Alert] Webhook URL failed SSRF validation");
        return false;
    }

    // Slack uses markdown-like formatting (mrkdwn), convert Discord markdown
    let slack_text = message.replace("**", "*"); // Discord bold (**) → Slack bold (*)

    let body = serde_json::json!({
        "text": slack_text,
        "mrkdwn": true,
    });

    match client.post(webhook_url).json(&body).send().await {
        Ok(response) => {
            if response.status().is_success() {
                tracing::info!(channel = "slack", "🔔 [Alert Sent]");
                true
            } else {
                tracing::error!(channel = "slack", status = %response.status(), "⚠️ [Alert Failed]");
                false
            }
        }
        Err(e) => {
            tracing::error!(channel = "slack", err = ?e, "⚠️ [Alert Error]");
            false
        }
    }
}

async fn send_email(config: &serde_json::Value, message: &str) {
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

    let smtp_host = config
        .get("smtp_host")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let smtp_port = config
        .get("smtp_port")
        .and_then(|v| v.as_u64())
        .unwrap_or(587) as u16;
    let smtp_user = config
        .get("smtp_user")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let smtp_pass = config
        .get("smtp_pass")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let from = config.get("from").and_then(|v| v.as_str()).unwrap_or("");
    let to = config.get("to").and_then(|v| v.as_str()).unwrap_or("");

    if smtp_host.is_empty() || from.is_empty() || to.is_empty() {
        tracing::warn!(
            channel = "email",
            "⚠️ [Email] Missing required config (smtp_host, from, to)"
        );
        return;
    }

    // Runtime SSRF re-check. Handler-time validation is not enough: if the
    // blocklist grew (new private-IP range, new disallowed port) after a
    // channel was saved, or the DNS record for `smtp_host` changed to point
    // at an internal IP, the previously-valid config would otherwise be
    // honored. Also guards against raw DB writes that skip the handler.
    if let Err(e) =
        crate::services::url_validator::validate_host(&format!("{smtp_host}:{smtp_port}")).await
    {
        tracing::error!(
            channel = "email",
            smtp_host,
            smtp_port,
            err = %e,
            "🚫 [Email] SSRF block — refusing to connect"
        );
        return;
    }

    if smtp_user.is_empty() || smtp_pass.is_empty() {
        tracing::warn!(
            channel = "email",
            "⚠️ [Email] SMTP credentials are empty — authentication will likely fail"
        );
    }

    // Strip markdown formatting for plain-text email
    let plain_text = message.replace("**", "").replace('`', "");

    let Ok(from_addr) = from.parse() else {
        tracing::error!(channel = "email", from, "⚠️ [Email] Invalid 'from' address");
        return;
    };
    let Ok(to_addr) = to.parse() else {
        tracing::error!(channel = "email", to, "⚠️ [Email] Invalid 'to' address");
        return;
    };

    let email = match Message::builder()
        .from(from_addr)
        .to(to_addr)
        .subject("NetSentinel Alert")
        .body(plain_text)
    {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(channel = "email", err = ?e, "⚠️ [Email] Failed to build message");
            return;
        }
    };

    let creds = Credentials::new(smtp_user.to_string(), smtp_pass.to_string());

    let mailer = match AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(smtp_host) {
        Ok(builder) => builder.port(smtp_port).credentials(creds).build(),
        Err(e) => {
            tracing::error!(channel = "email", err = ?e, "⚠️ [Email] Failed to create SMTP transport");
            return;
        }
    };

    match mailer.send(email).await {
        Ok(_) => tracing::info!(channel = "email", to = %to, "🔔 [Alert Sent]"),
        Err(e) => tracing::error!(channel = "email", err = ?e, "⚠️ [Alert Failed]"),
    }
}
