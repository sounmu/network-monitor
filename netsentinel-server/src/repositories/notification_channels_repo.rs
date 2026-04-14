use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// Row from the `notification_channels` table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct NotificationChannelRow {
    pub id: i32,
    pub name: String,
    pub channel_type: String,
    pub enabled: bool,
    /// JSON config — varies by channel type:
    /// Discord/Slack: { "webhook_url": "https://..." }
    /// Email: { "smtp_host": "...", "smtp_port": 587, "smtp_user": "...", "smtp_pass": "...", "from": "...", "to": "..." }
    pub config: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Create the notification_channels table
pub async fn init_table(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS notification_channels (
            id           SERIAL PRIMARY KEY,
            name         TEXT NOT NULL,
            channel_type TEXT NOT NULL CHECK (channel_type IN ('discord', 'slack', 'email')),
            enabled      BOOLEAN NOT NULL DEFAULT true,
            config       JSONB NOT NULL DEFAULT '{}',
            created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Fetch all notification channels
pub async fn get_all(pool: &PgPool) -> Result<Vec<NotificationChannelRow>, sqlx::Error> {
    sqlx::query_as::<_, NotificationChannelRow>("SELECT * FROM notification_channels ORDER BY id")
        .fetch_all(pool)
        .await
}

/// Fetch a single notification channel by ID
pub async fn get_by_id(
    pool: &PgPool,
    id: i32,
) -> Result<Option<NotificationChannelRow>, sqlx::Error> {
    sqlx::query_as::<_, NotificationChannelRow>("SELECT * FROM notification_channels WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Fetch only enabled notification channels
pub async fn get_enabled(pool: &PgPool) -> Result<Vec<NotificationChannelRow>, sqlx::Error> {
    sqlx::query_as::<_, NotificationChannelRow>(
        "SELECT * FROM notification_channels WHERE enabled = true ORDER BY id",
    )
    .fetch_all(pool)
    .await
}

#[derive(Debug, Deserialize)]
pub struct CreateChannelRequest {
    pub name: String,
    pub channel_type: String,
    pub enabled: Option<bool>,
    pub config: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct UpdateChannelRequest {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub config: Option<serde_json::Value>,
}

/// Create a new notification channel
pub async fn create_channel(
    pool: &PgPool,
    req: &CreateChannelRequest,
) -> Result<NotificationChannelRow, sqlx::Error> {
    sqlx::query_as::<_, NotificationChannelRow>(
        r#"
        INSERT INTO notification_channels (name, channel_type, enabled, config)
        VALUES ($1, $2, $3, $4)
        RETURNING *
        "#,
    )
    .bind(&req.name)
    .bind(&req.channel_type)
    .bind(req.enabled.unwrap_or(true))
    .bind(&req.config)
    .fetch_one(pool)
    .await
}

/// Update an existing notification channel
pub async fn update_channel(
    pool: &PgPool,
    id: i32,
    req: &UpdateChannelRequest,
) -> Result<Option<NotificationChannelRow>, sqlx::Error> {
    sqlx::query_as::<_, NotificationChannelRow>(
        r#"
        UPDATE notification_channels
        SET name       = COALESCE($2, name),
            enabled    = COALESCE($3, enabled),
            config     = COALESCE($4, config),
            updated_at = NOW()
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(&req.name)
    .bind(req.enabled)
    .bind(&req.config)
    .fetch_optional(pool)
    .await
}

/// Delete a notification channel
pub async fn delete_channel(pool: &PgPool, id: i32) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM notification_channels WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
