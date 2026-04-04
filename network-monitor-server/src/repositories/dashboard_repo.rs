use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// Dashboard layout stored per user.
/// `widgets` is a JSON array of widget configurations (type, position, host_key, etc.)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DashboardLayout {
    pub id: i32,
    pub user_id: i32,
    pub widgets: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

pub async fn init_table(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS dashboard_layouts (
            id         SERIAL PRIMARY KEY,
            user_id    INT NOT NULL UNIQUE,
            widgets    JSONB NOT NULL DEFAULT '[]',
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Get dashboard layout for a user (returns None if not customized yet)
pub async fn get_layout(
    pool: &PgPool,
    user_id: i32,
) -> Result<Option<DashboardLayout>, sqlx::Error> {
    sqlx::query_as::<_, DashboardLayout>("SELECT * FROM dashboard_layouts WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await
}

/// Save or update dashboard layout for a user
pub async fn upsert_layout(
    pool: &PgPool,
    user_id: i32,
    widgets: &serde_json::Value,
) -> Result<DashboardLayout, sqlx::Error> {
    sqlx::query_as::<_, DashboardLayout>(
        r#"
        INSERT INTO dashboard_layouts (user_id, widgets, updated_at)
        VALUES ($1, $2, NOW())
        ON CONFLICT (user_id)
        DO UPDATE SET widgets = EXCLUDED.widgets, updated_at = NOW()
        RETURNING *
        "#,
    )
    .bind(user_id)
    .bind(widgets)
    .fetch_one(pool)
    .await
}
