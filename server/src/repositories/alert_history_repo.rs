use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::db::DbPool;

/// Row from the `alert_history` table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AlertHistoryRow {
    pub id: i64,
    pub host_key: String,
    pub alert_type: String,
    pub message: String,
    pub created_at: DateTime<Utc>,
}

/// Query parameters for alert history listing.
#[derive(Debug, Default, Deserialize)]
pub struct AlertHistoryQuery {
    pub host_key: Option<String>,
    /// Filter by a specific alert_type (e.g. `cpu_overload`).
    #[serde(rename = "type")]
    pub alert_type: Option<String>,
    /// Inclusive lower bound on `created_at`, RFC3339.
    pub from: Option<DateTime<Utc>>,
    /// Exclusive upper bound on `created_at`, RFC3339.
    pub to: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Page of alert history rows together with the total number of rows that
/// match the same filter (used to drive client pagination).
#[derive(Debug, Serialize)]
pub struct AlertHistoryPage {
    pub rows: Vec<AlertHistoryRow>,
    pub total: i64,
}

pub async fn insert_alert(
    pool: &DbPool,
    host_key: &str,
    alert_type: &str,
    message: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO alert_history (host_key, alert_type, message) VALUES (?1, ?2, ?3)")
        .bind(host_key)
        .bind(alert_type)
        .bind(message)
        .execute(pool)
        .await?;
    Ok(())
}

/// Batch-insert several alert_history rows in a single writer-lock acquisition.
///
/// SQLite has exactly one writer at a time, so N separate `insert_alert` calls
/// cost N round-trips *and* N chances to contend with the concurrent scrape-
/// cycle batch INSERTs. Callers that already know the set of alerts fired in
/// one cycle should prefer this variant.
///
/// Rows are `(host_key, alert_type, message)`. Empty input is a no-op.
pub async fn insert_alerts_batch(
    pool: &DbPool,
    rows: &[(&str, &str, &str)],
) -> Result<(), sqlx::Error> {
    if rows.is_empty() {
        return Ok(());
    }
    use sqlx::{QueryBuilder, Sqlite};
    let mut qb =
        QueryBuilder::<Sqlite>::new("INSERT INTO alert_history (host_key, alert_type, message) ");
    qb.push_values(rows.iter(), |mut b, (hk, ty, msg)| {
        b.push_bind(*hk).push_bind(*ty).push_bind(*msg);
    });
    qb.build().execute(pool).await?;
    Ok(())
}

pub async fn get_alert_history_page(
    pool: &DbPool,
    query: &AlertHistoryQuery,
) -> Result<AlertHistoryPage, sqlx::Error> {
    use sqlx::{QueryBuilder, Sqlite};

    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).clamp(0, 10_000);

    let mut list = QueryBuilder::<Sqlite>::new(
        "SELECT id, host_key, alert_type, message, created_at FROM alert_history WHERE 1=1",
    );
    let mut count = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM alert_history WHERE 1=1");

    if let Some(ref hk) = query.host_key {
        list.push(" AND host_key = ").push_bind(hk.clone());
        count.push(" AND host_key = ").push_bind(hk.clone());
    }
    if let Some(ref at) = query.alert_type {
        list.push(" AND alert_type = ").push_bind(at.clone());
        count.push(" AND alert_type = ").push_bind(at.clone());
    }
    if let Some(from) = query.from {
        list.push(" AND created_at >= ").push_bind(from.timestamp());
        count
            .push(" AND created_at >= ")
            .push_bind(from.timestamp());
    }
    if let Some(to) = query.to {
        list.push(" AND created_at < ").push_bind(to.timestamp());
        count.push(" AND created_at < ").push_bind(to.timestamp());
    }

    list.push(" ORDER BY created_at DESC LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);

    let rows = list
        .build_query_as::<AlertHistoryRow>()
        .fetch_all(pool)
        .await?;
    let total: (i64,) = count.build_query_as().fetch_one(pool).await?;

    Ok(AlertHistoryPage {
        rows,
        total: total.0,
    })
}

/// Compute currently-firing alerts by picking the latest event per
/// (host_key, base_alert_kind) and returning only those that end in
/// `_overload` / `_down`. SQLite has no regex, so the base-kind is
/// derived with a `CASE` over the three suffixes, and `DISTINCT ON` is
/// emulated with a `ROW_NUMBER()` window.
pub async fn get_active_alerts(pool: &DbPool) -> Result<Vec<AlertHistoryRow>, sqlx::Error> {
    sqlx::query_as::<_, AlertHistoryRow>(
        r#"
        WITH paired AS (
            SELECT
                id,
                host_key,
                alert_type,
                message,
                created_at,
                -- base_kind: strip the final `_overload` / `_recovery`
                -- / `_down` suffix so opposing events cancel. SQLite
                -- has no regex — enumerate the three possibilities.
                CASE
                    WHEN alert_type LIKE '%\_overload' ESCAPE '\'
                        THEN substr(alert_type, 1, length(alert_type) - 9)
                    WHEN alert_type LIKE '%\_recovery' ESCAPE '\'
                        THEN substr(alert_type, 1, length(alert_type) - 9)
                    WHEN alert_type LIKE '%\_down' ESCAPE '\'
                        THEN substr(alert_type, 1, length(alert_type) - 5)
                    ELSE alert_type
                END AS base_kind
            FROM alert_history
            WHERE created_at > strftime('%s','now') - 14 * 86400
        ),
        ranked AS (
            SELECT
                id, host_key, alert_type, message, created_at, base_kind,
                ROW_NUMBER() OVER (
                    PARTITION BY host_key, base_kind
                    ORDER BY created_at DESC
                ) AS rn
            FROM paired
        )
        SELECT id, host_key, alert_type, message, created_at
        FROM ranked
        WHERE rn = 1
          AND (alert_type LIKE '%\_overload' ESCAPE '\'
            OR alert_type LIKE '%\_down' ESCAPE '\')
        ORDER BY created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await
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
    async fn insert_and_paginate() {
        let pool = fresh_pool().await;
        for i in 0..10 {
            insert_alert(
                &pool,
                "a:9101",
                "cpu_overload",
                &format!("sample message {i}"),
            )
            .await
            .unwrap();
        }
        insert_alert(&pool, "b:9101", "memory_overload", "ram hot")
            .await
            .unwrap();

        let page = get_alert_history_page(
            &pool,
            &AlertHistoryQuery {
                limit: Some(5),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(page.rows.len(), 5);
        assert_eq!(page.total, 11);

        let by_host = get_alert_history_page(
            &pool,
            &AlertHistoryQuery {
                host_key: Some("b:9101".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(by_host.total, 1);
        assert_eq!(by_host.rows[0].alert_type, "memory_overload");
    }

    #[tokio::test]
    async fn active_alerts_keeps_only_unresolved_latest() {
        let pool = fresh_pool().await;
        // Host A: CPU overload, then CPU recovery → resolved.
        insert_alert(&pool, "a:9101", "cpu_overload", "cpu=92%")
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        insert_alert(&pool, "a:9101", "cpu_recovery", "cpu back to 50%")
            .await
            .unwrap();

        // Host A: memory overload → still firing.
        insert_alert(&pool, "a:9101", "memory_overload", "mem=95%")
            .await
            .unwrap();

        // Host B: host_down → active.
        insert_alert(&pool, "b:9101", "host_down", "unreachable")
            .await
            .unwrap();

        let active = get_active_alerts(&pool).await.unwrap();
        let active_types: std::collections::HashSet<_> =
            active.iter().map(|r| r.alert_type.as_str()).collect();
        assert!(active_types.contains("memory_overload"));
        assert!(active_types.contains("host_down"));
        assert!(!active_types.contains("cpu_overload"));
        assert!(!active_types.contains("cpu_recovery"));
        assert_eq!(active.len(), 2);
    }
}
