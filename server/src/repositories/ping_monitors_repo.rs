use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::db::DbPool;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PingMonitor {
    pub id: i32,
    pub name: String,
    pub host: String,
    pub interval_secs: i32,
    pub timeout_ms: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PingResult {
    pub id: i64,
    pub monitor_id: i32,
    pub rtt_ms: Option<f64>,
    pub success: bool,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePingMonitorRequest {
    pub name: String,
    pub host: String,
    pub interval_secs: Option<i32>,
    pub timeout_ms: Option<i32>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePingMonitorRequest {
    pub name: Option<String>,
    pub host: Option<String>,
    pub interval_secs: Option<i32>,
    pub timeout_ms: Option<i32>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PingMonitorSummary {
    pub monitor_id: i32,
    pub latest_rtt_ms: Option<f64>,
    pub latest_success: Option<bool>,
    pub latest_error: Option<String>,
    pub total_checks: i64,
    pub successful_checks: i64,
    pub uptime_pct: f64,
}

pub async fn get_all(pool: &DbPool) -> Result<Vec<PingMonitor>, sqlx::Error> {
    sqlx::query_as::<_, PingMonitor>(
        "SELECT id, name, host, interval_secs, timeout_ms, enabled, created_at, updated_at \
         FROM ping_monitors ORDER BY id",
    )
    .fetch_all(pool)
    .await
}

pub async fn get_enabled(pool: &DbPool) -> Result<Vec<PingMonitor>, sqlx::Error> {
    sqlx::query_as::<_, PingMonitor>(
        "SELECT id, name, host, interval_secs, timeout_ms, enabled, created_at, updated_at \
         FROM ping_monitors WHERE enabled = 1 ORDER BY id",
    )
    .fetch_all(pool)
    .await
}

pub async fn create(
    pool: &DbPool,
    req: &CreatePingMonitorRequest,
) -> Result<PingMonitor, sqlx::Error> {
    sqlx::query_as::<_, PingMonitor>(
        r#"
        INSERT INTO ping_monitors (name, host, interval_secs, timeout_ms, enabled)
        VALUES (?1, ?2, ?3, ?4, ?5)
        RETURNING id, name, host, interval_secs, timeout_ms, enabled, created_at, updated_at
        "#,
    )
    .bind(&req.name)
    .bind(&req.host)
    .bind(req.interval_secs.unwrap_or(60))
    .bind(req.timeout_ms.unwrap_or(5000))
    .bind(req.enabled.unwrap_or(true))
    .fetch_one(pool)
    .await
}

pub async fn update(
    pool: &DbPool,
    id: i32,
    req: &UpdatePingMonitorRequest,
) -> Result<Option<PingMonitor>, sqlx::Error> {
    sqlx::query_as::<_, PingMonitor>(
        r#"
        UPDATE ping_monitors
        SET name          = COALESCE(?2, name),
            host          = COALESCE(?3, host),
            interval_secs = COALESCE(?4, interval_secs),
            timeout_ms    = COALESCE(?5, timeout_ms),
            enabled       = COALESCE(?6, enabled),
            updated_at    = strftime('%s','now')
        WHERE id = ?1
        RETURNING id, name, host, interval_secs, timeout_ms, enabled, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(&req.name)
    .bind(&req.host)
    .bind(req.interval_secs)
    .bind(req.timeout_ms)
    .bind(req.enabled)
    .fetch_optional(pool)
    .await
}

pub async fn delete(pool: &DbPool, id: i32) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM ping_monitors WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn insert_result(
    pool: &DbPool,
    monitor_id: i32,
    rtt_ms: Option<f64>,
    success: bool,
    error: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ping_results (monitor_id, rtt_ms, success, error) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(monitor_id)
    .bind(rtt_ms)
    .bind(success)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_results(
    pool: &DbPool,
    monitor_id: i32,
    limit: i64,
) -> Result<Vec<PingResult>, sqlx::Error> {
    sqlx::query_as::<_, PingResult>(
        "SELECT id, monitor_id, rtt_ms, success, error, created_at \
         FROM ping_results WHERE monitor_id = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2",
    )
    .bind(monitor_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Per-monitor latest-result + 24 h uptime summaries.
///
/// Same transformation as `http_monitors_repo::get_summaries` — see that
/// function's comment for the full rationale. Window + LEFT JOIN + single
/// GROUP BY replaces the previous 8 correlated subqueries per monitor.
pub async fn get_summaries(pool: &DbPool) -> Result<Vec<PingMonitorSummary>, sqlx::Error> {
    sqlx::query_as::<_, PingMonitorSummary>(
        r#"
        WITH ranked AS (
            SELECT
                monitor_id,
                rtt_ms,
                success,
                error,
                id,
                ROW_NUMBER() OVER (
                    PARTITION BY monitor_id
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM ping_results
            WHERE created_at >= strftime('%s','now') - 86400
        )
        SELECT
            m.id AS monitor_id,
            MAX(CASE WHEN r.rn = 1 THEN r.rtt_ms END)   AS latest_rtt_ms,
            MAX(CASE WHEN r.rn = 1 THEN r.success END)  AS latest_success,
            MAX(CASE WHEN r.rn = 1 THEN r.error END)    AS latest_error,
            COUNT(r.id)                                 AS total_checks,
            COALESCE(SUM(CASE WHEN r.success = 1 THEN 1 ELSE 0 END), 0) AS successful_checks,
            CASE
                WHEN COUNT(r.id) > 0 THEN
                    100.0 * CAST(SUM(CASE WHEN r.success = 1 THEN 1 ELSE 0 END) AS REAL)
                          / CAST(COUNT(r.id) AS REAL)
                ELSE 0.0
            END AS uptime_pct
        FROM ping_monitors m
        LEFT JOIN ranked r ON r.monitor_id = m.id
        GROUP BY m.id
        ORDER BY m.id
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
    async fn crud_cycle() {
        let pool = fresh_pool().await;
        let created = create(
            &pool,
            &CreatePingMonitorRequest {
                name: "gw".into(),
                host: "192.168.1.1".into(),
                interval_secs: None,
                timeout_ms: None,
                enabled: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(created.host, "192.168.1.1");
        assert!(created.enabled);

        assert!(delete(&pool, created.id).await.unwrap());
        assert!(get_all(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn summaries_uptime_from_success_flag() {
        let pool = fresh_pool().await;
        let m = create(
            &pool,
            &CreatePingMonitorRequest {
                name: "gw".into(),
                host: "192.168.1.1".into(),
                interval_secs: None,
                timeout_ms: None,
                enabled: None,
            },
        )
        .await
        .unwrap();

        insert_result(&pool, m.id, Some(1.2), true, None)
            .await
            .unwrap();
        insert_result(&pool, m.id, Some(1.5), true, None)
            .await
            .unwrap();
        insert_result(&pool, m.id, None, false, Some("timeout"))
            .await
            .unwrap();

        let s = &get_summaries(&pool).await.unwrap()[0];
        assert_eq!(s.total_checks, 3);
        assert_eq!(s.successful_checks, 2);
        assert!((s.uptime_pct - (2.0 / 3.0) * 100.0).abs() < 0.01);
        // `latest_success` reflects the most recent insert (the failure).
        assert_eq!(s.latest_success, Some(false));
        assert_eq!(s.latest_error.as_deref(), Some("timeout"));
    }
}
