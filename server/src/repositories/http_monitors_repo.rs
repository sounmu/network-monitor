use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::db::DbPool;

// ──────────────────────────────────────────────
// Schema
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct HttpMonitor {
    pub id: i32,
    pub name: String,
    pub url: String,
    pub method: String,
    pub expected_status: i32,
    pub interval_secs: i32,
    pub timeout_ms: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct HttpMonitorResult {
    pub id: i64,
    pub monitor_id: i32,
    pub status_code: Option<i32>,
    pub response_time_ms: Option<i32>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateHttpMonitorRequest {
    pub name: String,
    pub url: String,
    pub method: Option<String>,
    pub expected_status: Option<i32>,
    pub interval_secs: Option<i32>,
    pub timeout_ms: Option<i32>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateHttpMonitorRequest {
    pub name: Option<String>,
    pub url: Option<String>,
    pub method: Option<String>,
    pub expected_status: Option<i32>,
    pub interval_secs: Option<i32>,
    pub timeout_ms: Option<i32>,
    pub enabled: Option<bool>,
}

/// Summary for the monitors list — latest result + uptime % (last 24h)
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct HttpMonitorSummary {
    pub monitor_id: i32,
    pub latest_status_code: Option<i32>,
    pub latest_response_time_ms: Option<i32>,
    pub latest_error: Option<String>,
    pub total_checks: i64,
    pub successful_checks: i64,
    pub uptime_pct: f64,
}

pub async fn get_all(pool: &DbPool) -> Result<Vec<HttpMonitor>, sqlx::Error> {
    sqlx::query_as::<_, HttpMonitor>(
        "SELECT id, name, url, method, expected_status, interval_secs, timeout_ms, \
         enabled, created_at, updated_at FROM http_monitors ORDER BY id",
    )
    .fetch_all(pool)
    .await
}

pub async fn get_enabled(pool: &DbPool) -> Result<Vec<HttpMonitor>, sqlx::Error> {
    sqlx::query_as::<_, HttpMonitor>(
        "SELECT id, name, url, method, expected_status, interval_secs, timeout_ms, \
         enabled, created_at, updated_at FROM http_monitors WHERE enabled = 1 ORDER BY id",
    )
    .fetch_all(pool)
    .await
}

pub async fn create(
    pool: &DbPool,
    req: &CreateHttpMonitorRequest,
) -> Result<HttpMonitor, sqlx::Error> {
    sqlx::query_as::<_, HttpMonitor>(
        r#"
        INSERT INTO http_monitors (name, url, method, expected_status, interval_secs, timeout_ms, enabled)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        RETURNING id, name, url, method, expected_status, interval_secs, timeout_ms,
                  enabled, created_at, updated_at
        "#,
    )
    .bind(&req.name)
    .bind(&req.url)
    .bind(req.method.as_deref().unwrap_or("GET"))
    .bind(req.expected_status.unwrap_or(200))
    .bind(req.interval_secs.unwrap_or(60))
    .bind(req.timeout_ms.unwrap_or(10000))
    .bind(req.enabled.unwrap_or(true))
    .fetch_one(pool)
    .await
}

pub async fn update(
    pool: &DbPool,
    id: i32,
    req: &UpdateHttpMonitorRequest,
) -> Result<Option<HttpMonitor>, sqlx::Error> {
    sqlx::query_as::<_, HttpMonitor>(
        r#"
        UPDATE http_monitors
        SET name            = COALESCE(?2, name),
            url             = COALESCE(?3, url),
            method          = COALESCE(?4, method),
            expected_status = COALESCE(?5, expected_status),
            interval_secs   = COALESCE(?6, interval_secs),
            timeout_ms      = COALESCE(?7, timeout_ms),
            enabled         = COALESCE(?8, enabled),
            updated_at      = strftime('%s','now')
        WHERE id = ?1
        RETURNING id, name, url, method, expected_status, interval_secs, timeout_ms,
                  enabled, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(&req.name)
    .bind(&req.url)
    .bind(&req.method)
    .bind(req.expected_status)
    .bind(req.interval_secs)
    .bind(req.timeout_ms)
    .bind(req.enabled)
    .fetch_optional(pool)
    .await
}

pub async fn delete(pool: &DbPool, id: i32) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM http_monitors WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn insert_result(
    pool: &DbPool,
    monitor_id: i32,
    status_code: Option<i32>,
    response_time_ms: Option<i32>,
    error: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO http_monitor_results (monitor_id, status_code, response_time_ms, error) \
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(monitor_id)
    .bind(status_code)
    .bind(response_time_ms)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_results(
    pool: &DbPool,
    monitor_id: i32,
    limit: i64,
) -> Result<Vec<HttpMonitorResult>, sqlx::Error> {
    sqlx::query_as::<_, HttpMonitorResult>(
        "SELECT id, monitor_id, status_code, response_time_ms, error, created_at \
         FROM http_monitor_results WHERE monitor_id = ?1 ORDER BY created_at DESC LIMIT ?2",
    )
    .bind(monitor_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Per-monitor latest-result + 24 h uptime summaries.
///
/// Previous shape issued **8 correlated subqueries per monitor** (3 for the
/// latest row columns, then 2×2 repeated COUNT/SUM blocks inside the CASE).
/// At 50 monitors that was 400 index lookups per request. This version
/// tags each recent result with a `ROW_NUMBER()` window and picks the
/// "latest" row via `MAX(CASE WHEN rn=1 THEN col END)`, so a single scan
/// of `http_monitor_results` feeds both the latest-row columns and the
/// aggregates. The `LEFT JOIN` preserves monitors that have no results yet.
pub async fn get_summaries(pool: &DbPool) -> Result<Vec<HttpMonitorSummary>, sqlx::Error> {
    sqlx::query_as::<_, HttpMonitorSummary>(
        r#"
        WITH ranked AS (
            SELECT
                monitor_id,
                status_code,
                response_time_ms,
                error,
                id,
                ROW_NUMBER() OVER (
                    PARTITION BY monitor_id
                    ORDER BY created_at DESC, id DESC
                ) AS rn
            FROM http_monitor_results
            WHERE created_at >= strftime('%s','now') - 86400
        )
        SELECT
            m.id AS monitor_id,
            MAX(CASE WHEN r.rn = 1 THEN r.status_code END)      AS latest_status_code,
            MAX(CASE WHEN r.rn = 1 THEN r.response_time_ms END) AS latest_response_time_ms,
            MAX(CASE WHEN r.rn = 1 THEN r.error END)            AS latest_error,
            COUNT(r.id)                                         AS total_checks,
            COALESCE(SUM(CASE
                WHEN r.error IS NULL AND r.status_code IS NOT NULL THEN 1 ELSE 0
            END), 0) AS successful_checks,
            CASE
                WHEN COUNT(r.id) > 0 THEN
                    100.0 * CAST(SUM(CASE
                        WHEN r.error IS NULL AND r.status_code IS NOT NULL THEN 1 ELSE 0
                    END) AS REAL) / CAST(COUNT(r.id) AS REAL)
                ELSE 0.0
            END AS uptime_pct
        FROM http_monitors m
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

    fn sample_req(name: &str, url: &str) -> CreateHttpMonitorRequest {
        CreateHttpMonitorRequest {
            name: name.into(),
            url: url.into(),
            method: None,
            expected_status: None,
            interval_secs: None,
            timeout_ms: None,
            enabled: None,
        }
    }

    #[tokio::test]
    async fn crud_cycle() {
        let pool = fresh_pool().await;
        let created = create(&pool, &sample_req("api", "https://example.com/api"))
            .await
            .unwrap();
        assert_eq!(created.method, "GET");
        assert_eq!(created.expected_status, 200);
        assert!(created.enabled);

        assert_eq!(get_all(&pool).await.unwrap().len(), 1);
        assert_eq!(get_enabled(&pool).await.unwrap().len(), 1);

        let updated = update(
            &pool,
            created.id,
            &UpdateHttpMonitorRequest {
                name: None,
                url: None,
                method: Some("HEAD".into()),
                expected_status: None,
                interval_secs: None,
                timeout_ms: None,
                enabled: Some(false),
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(updated.method, "HEAD");
        assert!(!updated.enabled);
        assert!(get_enabled(&pool).await.unwrap().is_empty());

        assert!(delete(&pool, created.id).await.unwrap());
        assert!(get_all(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn summaries_compute_uptime_from_recent_results() {
        let pool = fresh_pool().await;
        let m = create(&pool, &sample_req("home", "https://example.com"))
            .await
            .unwrap();

        // 3 successes (200, no error) + 1 failure.
        insert_result(&pool, m.id, Some(200), Some(120), None)
            .await
            .unwrap();
        insert_result(&pool, m.id, Some(200), Some(130), None)
            .await
            .unwrap();
        insert_result(&pool, m.id, Some(200), Some(140), None)
            .await
            .unwrap();
        insert_result(&pool, m.id, Some(503), Some(10), Some("bad gateway"))
            .await
            .unwrap();

        let summaries = get_summaries(&pool).await.unwrap();
        assert_eq!(summaries.len(), 1);
        let s = &summaries[0];
        assert_eq!(s.monitor_id, m.id);
        assert_eq!(s.total_checks, 4);
        assert_eq!(s.successful_checks, 3);
        assert!(
            (s.uptime_pct - 75.0).abs() < 0.01,
            "uptime={}",
            s.uptime_pct
        );
        // `latest_*` reflects the most recent insert (the 503 failure).
        assert_eq!(s.latest_status_code, Some(503));
        assert_eq!(s.latest_error.as_deref(), Some("bad gateway"));

        let results = get_results(&pool, m.id, 100).await.unwrap();
        assert_eq!(results.len(), 4);
    }

    #[tokio::test]
    async fn summaries_handle_monitor_with_no_results() {
        let pool = fresh_pool().await;
        create(&pool, &sample_req("empty", "https://nowhere.example"))
            .await
            .unwrap();
        let summaries = get_summaries(&pool).await.unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].total_checks, 0);
        assert_eq!(summaries[0].uptime_pct, 0.0);
        assert!(summaries[0].latest_status_code.is_none());
    }
}
