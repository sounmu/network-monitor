use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// Row from the `alert_history` table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AlertHistoryRow {
    pub id: i64,
    pub host_key: String,
    pub alert_type: String,
    pub message: String,
    pub created_at: DateTime<Utc>,
}

/// Create the alert_history table
pub async fn init_table(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS alert_history (
            id           BIGSERIAL PRIMARY KEY,
            host_key     VARCHAR(255) NOT NULL,
            alert_type   VARCHAR(100) NOT NULL,
            message      TEXT NOT NULL,
            created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_alert_history_host_time ON alert_history (host_key, created_at DESC)",
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Insert an alert history record
pub async fn insert_alert(
    pool: &PgPool,
    host_key: &str,
    alert_type: &str,
    message: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO alert_history (host_key, alert_type, message) VALUES ($1, $2, $3)")
        .bind(host_key)
        .bind(alert_type)
        .bind(message)
        .execute(pool)
        .await?;
    Ok(())
}

/// Query parameters for alert history listing
#[derive(Debug, Deserialize)]
pub struct AlertHistoryQuery {
    pub host_key: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Fetch alert history with optional host_key filter and pagination
pub async fn get_alert_history(
    pool: &PgPool,
    query: &AlertHistoryQuery,
) -> Result<Vec<AlertHistoryRow>, sqlx::Error> {
    let limit = query.limit.unwrap_or(50).min(200);
    let offset = query.offset.unwrap_or(0);

    if let Some(ref hk) = query.host_key {
        sqlx::query_as::<_, AlertHistoryRow>(
            "SELECT * FROM alert_history WHERE host_key = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
        )
        .bind(hk)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, AlertHistoryRow>(
            "SELECT * FROM alert_history ORDER BY created_at DESC LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
    }
}
