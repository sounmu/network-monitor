use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::db::DbPool;

/// Dashboard layout stored per user.
/// `widgets` is a JSON array of widget configurations (type, position, host_key, etc.)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DashboardLayout {
    pub user_id: i32,
    pub widgets: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

/// Intermediate row holding the raw SQLite column types before we
/// parse the TEXT JSON payload into `serde_json::Value`. Kept private
/// so callers only see the canonical `DashboardLayout`.
#[derive(sqlx::FromRow)]
struct DashboardLayoutRaw {
    user_id: i32,
    widgets: String,
    updated_at: DateTime<Utc>,
}

impl TryFrom<DashboardLayoutRaw> for DashboardLayout {
    type Error = sqlx::Error;

    fn try_from(raw: DashboardLayoutRaw) -> Result<Self, Self::Error> {
        let widgets: serde_json::Value =
            serde_json::from_str(&raw.widgets).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        Ok(Self {
            user_id: raw.user_id,
            widgets,
            updated_at: raw.updated_at,
        })
    }
}

pub async fn get_layout(
    pool: &DbPool,
    user_id: i32,
) -> Result<Option<DashboardLayout>, sqlx::Error> {
    let raw = sqlx::query_as::<_, DashboardLayoutRaw>(
        "SELECT user_id, widgets, updated_at FROM dashboard_layouts WHERE user_id = ?1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    raw.map(DashboardLayout::try_from).transpose()
}

pub async fn upsert_layout(
    pool: &DbPool,
    user_id: i32,
    widgets: &serde_json::Value,
) -> Result<DashboardLayout, sqlx::Error> {
    // `serde_json::to_string` cannot fail for a `serde_json::Value` —
    // only for types with custom `Serialize` impls that propagate
    // errors, which we don't have here — so `.expect` documents the
    // invariant rather than masking a real error path.
    let widgets_text = serde_json::to_string(widgets).expect("serde_json::Value always serialises");

    let raw = sqlx::query_as::<_, DashboardLayoutRaw>(
        r#"
        INSERT INTO dashboard_layouts (user_id, widgets, updated_at)
        VALUES (?1, ?2, strftime('%s','now'))
        ON CONFLICT(user_id) DO UPDATE SET
            widgets    = excluded.widgets,
            updated_at = strftime('%s','now')
        RETURNING user_id, widgets, updated_at
        "#,
    )
    .bind(user_id)
    .bind(&widgets_text)
    .fetch_one(pool)
    .await?;

    DashboardLayout::try_from(raw)
}

// ── Round-trip smoke test ────────────────────────────────────────

#[cfg(test)]
mod sqlite_tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn fresh_pool() -> DbPool {
        // `foreign_keys(false)` so the dashboard row can stand alone
        // without a matching users row — production still enforces
        // FKs via `db::connect`.
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
    async fn upsert_then_get_roundtrips_widgets_and_timestamp() {
        let pool = fresh_pool().await;

        let widgets = serde_json::json!([
            { "type": "cpu", "host_key": "192.168.1.10:9101" },
            { "type": "mem", "host_key": "192.168.1.11:9101" },
        ]);

        let saved = upsert_layout(&pool, 42, &widgets).await.unwrap();
        assert_eq!(saved.user_id, 42);
        assert_eq!(saved.widgets, widgets);
        let drift = (Utc::now() - saved.updated_at).num_seconds().abs();
        assert!(drift < 5, "updated_at drift = {drift}s");

        let read_back = get_layout(&pool, 42).await.unwrap().unwrap();
        assert_eq!(read_back.widgets, widgets);
        assert_eq!(read_back.updated_at, saved.updated_at);

        let missing = get_layout(&pool, 99).await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn upsert_overwrites_prior_widgets() {
        let pool = fresh_pool().await;

        let first = serde_json::json!([{ "type": "cpu" }]);
        let second = serde_json::json!([{ "type": "mem" }, { "type": "disk" }]);

        upsert_layout(&pool, 7, &first).await.unwrap();
        let after = upsert_layout(&pool, 7, &second).await.unwrap();

        assert_eq!(after.widgets, second);
    }
}
