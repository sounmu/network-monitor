use reqwest::Client;
use sqlx::PgPool;

use crate::repositories::notification_channels_repo::{self, NotificationChannelRow};

// ──────────────────────────────────────────────
// Multi-channel alert delivery
// ──────────────────────────────────────────────

/// Send an alert message to all enabled notification channels from the DB.
pub async fn send_alert(client: &Client, pool: &PgPool, message: &str) {
    let channels = notification_channels_repo::get_enabled(pool)
        .await
        .unwrap_or_default();
    for channel in &channels {
        match channel.channel_type.as_str() {
            "discord" => {
                if let Some(url) = channel.config.get("webhook_url").and_then(|v| v.as_str()) {
                    send_discord(client, url, message).await;
                }
            }
            "slack" => {
                if let Some(url) = channel.config.get("webhook_url").and_then(|v| v.as_str()) {
                    send_slack(client, url, message).await;
                }
            }
            "email" => {
                send_email(&channel.config, message).await;
            }
            _ => {
                tracing::warn!(channel_type = %channel.channel_type, "Unknown notification channel type");
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

    match channel.channel_type.as_str() {
        "discord" => {
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
        "slack" => {
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
        "email" => {
            send_email(&channel.config, &test_msg).await;
            Ok(())
        }
        other => Err(format!("Unknown channel type: {}", other)),
    }
}

// ──────────────────────────────────────────────
// Channel implementations
// ──────────────────────────────────────────────

async fn send_discord(client: &Client, webhook_url: &str, message: &str) -> bool {
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

    // Strip markdown formatting for plain-text email
    let plain_text = message.replace("**", "").replace('`', "");

    let email = match Message::builder()
        .from(
            from.parse()
                .unwrap_or_else(|_| "alert@monitor.local".parse().unwrap()),
        )
        .to(to
            .parse()
            .unwrap_or_else(|_| "admin@monitor.local".parse().unwrap()))
        .subject("Network Monitor Alert")
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
