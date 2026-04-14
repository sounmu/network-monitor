use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

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

pub async fn init_tables(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS ping_monitors (
            id              SERIAL PRIMARY KEY,
            name            TEXT NOT NULL,
            host            TEXT NOT NULL,
            interval_secs   INT NOT NULL DEFAULT 60,
            timeout_ms      INT NOT NULL DEFAULT 5000,
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
        CREATE TABLE IF NOT EXISTS ping_results (
            id              BIGSERIAL PRIMARY KEY,
            monitor_id      INT NOT NULL REFERENCES ping_monitors(id) ON DELETE CASCADE,
            rtt_ms          DOUBLE PRECISION,
            success         BOOLEAN NOT NULL,
            error           TEXT,
            created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_ping_results_monitor_time ON ping_results (monitor_id, created_at DESC)",
    )
    .execute(pool)
    .await?;

    Ok(())
}

// ── CRUD ──

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

pub async fn get_all(pool: &PgPool) -> Result<Vec<PingMonitor>, sqlx::Error> {
    sqlx::query_as::<_, PingMonitor>("SELECT * FROM ping_monitors ORDER BY id")
        .fetch_all(pool)
        .await
}

pub async fn get_enabled(pool: &PgPool) -> Result<Vec<PingMonitor>, sqlx::Error> {
    sqlx::query_as::<_, PingMonitor>("SELECT * FROM ping_monitors WHERE enabled = true ORDER BY id")
        .fetch_all(pool)
        .await
}

pub async fn create(
    pool: &PgPool,
    req: &CreatePingMonitorRequest,
) -> Result<PingMonitor, sqlx::Error> {
    sqlx::query_as::<_, PingMonitor>(
        r#"
        INSERT INTO ping_monitors (name, host, interval_secs, timeout_ms, enabled)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING *
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
    pool: &PgPool,
    id: i32,
    req: &UpdatePingMonitorRequest,
) -> Result<Option<PingMonitor>, sqlx::Error> {
    sqlx::query_as::<_, PingMonitor>(
        r#"
        UPDATE ping_monitors
        SET name          = COALESCE($2, name),
            host          = COALESCE($3, host),
            interval_secs = COALESCE($4, interval_secs),
            timeout_ms    = COALESCE($5, timeout_ms),
            enabled       = COALESCE($6, enabled),
            updated_at    = NOW()
        WHERE id = $1
        RETURNING *
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

pub async fn delete(pool: &PgPool, id: i32) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM ping_monitors WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

// ── Results ──

pub async fn insert_result(
    pool: &PgPool,
    monitor_id: i32,
    rtt_ms: Option<f64>,
    success: bool,
    error: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ping_results (monitor_id, rtt_ms, success, error) VALUES ($1, $2, $3, $4)",
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
    pool: &PgPool,
    monitor_id: i32,
    limit: i64,
) -> Result<Vec<PingResult>, sqlx::Error> {
    sqlx::query_as::<_, PingResult>(
        "SELECT * FROM ping_results WHERE monitor_id = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(monitor_id)
    .bind(limit)
    .fetch_all(pool)
    .await
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

pub async fn get_summaries(pool: &PgPool) -> Result<Vec<PingMonitorSummary>, sqlx::Error> {
    sqlx::query_as::<_, PingMonitorSummary>(
        r#"
        SELECT
            m.id AS monitor_id,
            latest.rtt_ms AS latest_rtt_ms,
            latest.success AS latest_success,
            latest.error AS latest_error,
            COALESCE(stats.total_checks, 0) AS total_checks,
            COALESCE(stats.successful_checks, 0) AS successful_checks,
            CASE WHEN COALESCE(stats.total_checks, 0) > 0
                THEN (stats.successful_checks::FLOAT / stats.total_checks::FLOAT * 100.0)
                ELSE 0.0
            END AS uptime_pct
        FROM ping_monitors m
        LEFT JOIN LATERAL (
            SELECT rtt_ms, success, error
            FROM ping_results
            WHERE monitor_id = m.id
            ORDER BY created_at DESC
            LIMIT 1
        ) latest ON true
        LEFT JOIN LATERAL (
            SELECT
                COUNT(*)::BIGINT AS total_checks,
                COUNT(*) FILTER (WHERE success = true)::BIGINT AS successful_checks
            FROM ping_results
            WHERE monitor_id = m.id
              AND created_at >= NOW() - INTERVAL '24 hours'
        ) stats ON true
        ORDER BY m.id
        "#,
    )
    .fetch_all(pool)
    .await
}
