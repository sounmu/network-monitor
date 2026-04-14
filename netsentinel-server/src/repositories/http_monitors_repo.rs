use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

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

pub async fn init_tables(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS http_monitors (
            id              SERIAL PRIMARY KEY,
            name            TEXT NOT NULL,
            url             TEXT NOT NULL,
            method          TEXT NOT NULL DEFAULT 'GET',
            expected_status INT NOT NULL DEFAULT 200,
            interval_secs   INT NOT NULL DEFAULT 60,
            timeout_ms      INT NOT NULL DEFAULT 10000,
            enabled         BOOLEAN NOT NULL DEFAULT true,
            created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS http_monitor_results (
            id              BIGSERIAL PRIMARY KEY,
            monitor_id      INT NOT NULL REFERENCES http_monitors(id) ON DELETE CASCADE,
            status_code     INT,
            response_time_ms INT,
            error           TEXT,
            created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_http_results_monitor_time ON http_monitor_results (monitor_id, created_at DESC)",
    )
    .execute(pool)
    .await?;

    Ok(())
}

// ──────────────────────────────────────────────
// CRUD
// ──────────────────────────────────────────────

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

pub async fn get_all(pool: &PgPool) -> Result<Vec<HttpMonitor>, sqlx::Error> {
    sqlx::query_as::<_, HttpMonitor>("SELECT * FROM http_monitors ORDER BY id")
        .fetch_all(pool)
        .await
}

pub async fn get_enabled(pool: &PgPool) -> Result<Vec<HttpMonitor>, sqlx::Error> {
    sqlx::query_as::<_, HttpMonitor>("SELECT * FROM http_monitors WHERE enabled = true ORDER BY id")
        .fetch_all(pool)
        .await
}

pub async fn create(
    pool: &PgPool,
    req: &CreateHttpMonitorRequest,
) -> Result<HttpMonitor, sqlx::Error> {
    sqlx::query_as::<_, HttpMonitor>(
        r#"
        INSERT INTO http_monitors (name, url, method, expected_status, interval_secs, timeout_ms, enabled)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING *
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
    pool: &PgPool,
    id: i32,
    req: &UpdateHttpMonitorRequest,
) -> Result<Option<HttpMonitor>, sqlx::Error> {
    sqlx::query_as::<_, HttpMonitor>(
        r#"
        UPDATE http_monitors
        SET name            = COALESCE($2, name),
            url             = COALESCE($3, url),
            method          = COALESCE($4, method),
            expected_status = COALESCE($5, expected_status),
            interval_secs   = COALESCE($6, interval_secs),
            timeout_ms      = COALESCE($7, timeout_ms),
            enabled         = COALESCE($8, enabled),
            updated_at      = NOW()
        WHERE id = $1
        RETURNING *
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

pub async fn delete(pool: &PgPool, id: i32) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM http_monitors WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

// ──────────────────────────────────────────────
// Results
// ──────────────────────────────────────────────

pub async fn insert_result(
    pool: &PgPool,
    monitor_id: i32,
    status_code: Option<i32>,
    response_time_ms: Option<i32>,
    error: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO http_monitor_results (monitor_id, status_code, response_time_ms, error) VALUES ($1, $2, $3, $4)",
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
    pool: &PgPool,
    monitor_id: i32,
    limit: i64,
) -> Result<Vec<HttpMonitorResult>, sqlx::Error> {
    sqlx::query_as::<_, HttpMonitorResult>(
        "SELECT * FROM http_monitor_results WHERE monitor_id = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(monitor_id)
    .bind(limit)
    .fetch_all(pool)
    .await
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

pub async fn get_summaries(pool: &PgPool) -> Result<Vec<HttpMonitorSummary>, sqlx::Error> {
    sqlx::query_as::<_, HttpMonitorSummary>(
        r#"
        SELECT
            m.id AS monitor_id,
            latest.status_code AS latest_status_code,
            latest.response_time_ms AS latest_response_time_ms,
            latest.error AS latest_error,
            COALESCE(stats.total_checks, 0) AS total_checks,
            COALESCE(stats.successful_checks, 0) AS successful_checks,
            CASE WHEN COALESCE(stats.total_checks, 0) > 0
                THEN (stats.successful_checks::FLOAT / stats.total_checks::FLOAT * 100.0)
                ELSE 0.0
            END AS uptime_pct
        FROM http_monitors m
        LEFT JOIN LATERAL (
            SELECT status_code, response_time_ms, error
            FROM http_monitor_results
            WHERE monitor_id = m.id
            ORDER BY created_at DESC
            LIMIT 1
        ) latest ON true
        LEFT JOIN LATERAL (
            SELECT
                COUNT(*)::BIGINT AS total_checks,
                COUNT(*) FILTER (WHERE error IS NULL AND status_code IS NOT NULL)::BIGINT AS successful_checks
            FROM http_monitor_results
            WHERE monitor_id = m.id
              AND created_at >= NOW() - INTERVAL '24 hours'
        ) stats ON true
        ORDER BY m.id
        "#,
    )
    .fetch_all(pool)
    .await
}
