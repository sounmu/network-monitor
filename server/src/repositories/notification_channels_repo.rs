use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::db::DbPool;

/// Notification delivery channel type — compile-time exhaustive matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    Discord,
    Slack,
    Email,
}

/// Row from the `notification_channels` table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct NotificationChannelRow {
    pub id: i32,
    pub name: String,
    pub channel_type: ChannelType,
    pub enabled: bool,
    /// JSON config — varies by channel type:
    /// Discord/Slack: { "webhook_url": "https://..." }
    /// Email: { "smtp_host": "...", "smtp_port": 587, "smtp_user": "...", "smtp_pass": "...", "from": "...", "to": "..." }
    pub config: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateChannelRequest {
    pub name: String,
    pub channel_type: ChannelType,
    pub enabled: Option<bool>,
    pub config: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct UpdateChannelRequest {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub config: Option<serde_json::Value>,
}

// `config` is stored as JSON text; decoded into `serde_json::Value` at
// the boundary via `NotificationChannelRaw → TryFrom`.

#[derive(sqlx::FromRow)]
struct NotificationChannelRaw {
    id: i32,
    name: String,
    channel_type: ChannelType,
    enabled: bool,
    config: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<NotificationChannelRaw> for NotificationChannelRow {
    type Error = sqlx::Error;

    fn try_from(raw: NotificationChannelRaw) -> Result<Self, Self::Error> {
        let config: serde_json::Value =
            serde_json::from_str(&raw.config).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        Ok(Self {
            id: raw.id,
            name: raw.name,
            channel_type: raw.channel_type,
            enabled: raw.enabled,
            config,
            created_at: raw.created_at,
            updated_at: raw.updated_at,
        })
    }
}

pub async fn get_all(pool: &DbPool) -> Result<Vec<NotificationChannelRow>, sqlx::Error> {
    let raws = sqlx::query_as::<_, NotificationChannelRaw>(
        "SELECT id, name, channel_type, enabled, config, created_at, updated_at \
         FROM notification_channels ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    raws.into_iter()
        .map(NotificationChannelRow::try_from)
        .collect()
}

pub async fn get_by_id(
    pool: &DbPool,
    id: i32,
) -> Result<Option<NotificationChannelRow>, sqlx::Error> {
    let raw = sqlx::query_as::<_, NotificationChannelRaw>(
        "SELECT id, name, channel_type, enabled, config, created_at, updated_at \
         FROM notification_channels WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    raw.map(NotificationChannelRow::try_from).transpose()
}

pub async fn get_enabled(pool: &DbPool) -> Result<Vec<NotificationChannelRow>, sqlx::Error> {
    let raws = sqlx::query_as::<_, NotificationChannelRaw>(
        "SELECT id, name, channel_type, enabled, config, created_at, updated_at \
         FROM notification_channels WHERE enabled = 1 ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    raws.into_iter()
        .map(NotificationChannelRow::try_from)
        .collect()
}

pub async fn create_channel(
    pool: &DbPool,
    req: &CreateChannelRequest,
) -> Result<NotificationChannelRow, sqlx::Error> {
    let config_text =
        serde_json::to_string(&req.config).expect("serde_json::Value always serialises");
    let raw = sqlx::query_as::<_, NotificationChannelRaw>(
        r#"
        INSERT INTO notification_channels (name, channel_type, enabled, config)
        VALUES (?1, ?2, ?3, ?4)
        RETURNING id, name, channel_type, enabled, config, created_at, updated_at
        "#,
    )
    .bind(&req.name)
    .bind(req.channel_type)
    .bind(req.enabled.unwrap_or(true))
    .bind(&config_text)
    .fetch_one(pool)
    .await?;
    NotificationChannelRow::try_from(raw)
}

pub async fn update_channel(
    pool: &DbPool,
    id: i32,
    req: &UpdateChannelRequest,
) -> Result<Option<NotificationChannelRow>, sqlx::Error> {
    // SQLite does not let us bind a JSON Value directly — serialise to
    // text when present and pass NULL otherwise so the COALESCE short-
    // circuit in the UPDATE keeps the existing column value.
    let config_text = req
        .config
        .as_ref()
        .map(|v| serde_json::to_string(v).expect("serde_json::Value always serialises"));

    let raw = sqlx::query_as::<_, NotificationChannelRaw>(
        r#"
        UPDATE notification_channels
        SET name       = COALESCE(?2, name),
            enabled    = COALESCE(?3, enabled),
            config     = COALESCE(?4, config),
            updated_at = strftime('%s','now')
        WHERE id = ?1
        RETURNING id, name, channel_type, enabled, config, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(&req.name)
    .bind(req.enabled)
    .bind(config_text)
    .fetch_optional(pool)
    .await?;
    raw.map(NotificationChannelRow::try_from).transpose()
}

pub async fn delete_channel(pool: &DbPool, id: i32) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM notification_channels WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod sqlite_tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn fresh_pool() -> DbPool {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(false)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Memory);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn crud_cycle_roundtrips_enum_and_json_config() {
        let pool = fresh_pool().await;
        let cfg = serde_json::json!({ "webhook_url": "https://hooks.example.com/abc" });

        let created = create_channel(
            &pool,
            &CreateChannelRequest {
                name: "ops-discord".into(),
                channel_type: ChannelType::Discord,
                enabled: Some(true),
                config: cfg.clone(),
            },
        )
        .await
        .unwrap();
        assert_eq!(created.channel_type, ChannelType::Discord);
        assert_eq!(created.config, cfg);

        let all = get_all(&pool).await.unwrap();
        assert_eq!(all.len(), 1);

        let only_enabled = get_enabled(&pool).await.unwrap();
        assert_eq!(only_enabled.len(), 1);

        // Update — flip enabled off, swap config.
        let new_cfg = serde_json::json!({ "webhook_url": "https://hooks.example.com/xyz" });
        let updated = update_channel(
            &pool,
            created.id,
            &UpdateChannelRequest {
                name: None,
                enabled: Some(false),
                config: Some(new_cfg.clone()),
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert!(!updated.enabled);
        assert_eq!(updated.config, new_cfg);

        // `get_enabled` now returns nothing.
        assert!(get_enabled(&pool).await.unwrap().is_empty());

        // Delete.
        assert!(delete_channel(&pool, created.id).await.unwrap());
        assert!(get_by_id(&pool, created.id).await.unwrap().is_none());
    }
}
