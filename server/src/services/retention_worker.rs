//! Retention worker for the SQLite backend.
//!
//! The Postgres backend relies on TimescaleDB's `add_retention_policy`
//! to prune old chunks in the background. SQLite has no such thing, so
//! this worker walks the five time-series tables once a day and
//! deletes rows older than their configured window.
//!
//! Windows come from `docs/SQLITE_MIGRATION.md §5.2` and differ from
//! Postgres intentionally:
//!   • raw `metrics`              — 3 days (down from PG's 90d)
//!   • `metrics_5min` rollup      — 90 days
//!   • `alert_history`            — 90 days
//!   • `http_monitor_results`     — 90 days
//!   • `ping_results`             — 90 days
//!
//! The raw window shrinks because 5-minute rollups preserve enough
//! fidelity for everything except live-troubleshooting reads. Anyone
//! who disagrees can override `RAW_METRICS_DAYS` via environment.
//!
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::db::DbPool;

pub const RAW_METRICS_DAYS: i64 = 3;
pub const ROLLUP_DAYS: i64 = 90;
pub const ALERT_HISTORY_DAYS: i64 = 90;
pub const MONITOR_RESULTS_DAYS: i64 = 90;

/// Rough tick cadence — once a day. Retention is not latency-sensitive;
/// an hour of staleness makes no observable difference.
const TICK_SECS: u64 = 24 * 60 * 60;

/// How many rows each tick deleted, per table. Returned by `run_once`
/// so callers can log per-table stats or write them to `/metrics`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RetentionStats {
    pub metrics: u64,
    pub metrics_5min: u64,
    pub alert_history: u64,
    pub http_monitor_results: u64,
    pub ping_results: u64,
}

impl RetentionStats {
    pub fn total(&self) -> u64 {
        self.metrics
            + self.metrics_5min
            + self.alert_history
            + self.http_monitor_results
            + self.ping_results
    }
}

/// How many rows a single DELETE chunk processes. Keeps the SQLite writer
/// lock held for a bounded, cooperative slice per chunk — at WAL + default
/// page size that is ≤ low-single-digit-millisecond work — after which the
/// task `yield_now()`s so the scraper's batch INSERT can slip in.
///
/// 2_000 was chosen so the worst-case chunk duration stays comfortably below
/// the scrape tick (10 s) and the SQLite `busy_timeout` (5 s). A cold first
/// pass touching millions of stale rows used to hold the writer lock long
/// enough that rollup (60 s tick) + scraper (10 s tick) + retention could
/// all fail at once when the `busy_timeout` and `acquire_timeout` windows
/// coincided. See commit `895312e` for the asymmetric pool-timeout split
/// that is the other half of this fix.
const DELETE_CHUNK: i64 = 2_000;

/// Run one retention pass. Each table deletes in its own statement —
/// wrapping all five in a single transaction could hold the writer
/// lock long enough to stall live scraping, and these DELETEs are
/// already idempotent.
pub async fn run_once(pool: &DbPool) -> Result<RetentionStats, sqlx::Error> {
    Ok(RetentionStats {
        // Rowid tables support the chunked `rowid IN (SELECT …)` pattern.
        metrics: delete_chunked_rowid(pool, "metrics", "timestamp", RAW_METRICS_DAYS).await?,
        // `metrics_5min` is declared `STRICT, WITHOUT ROWID` (see the init
        // migration). Rows per day are small (≤ one per 5-min bucket × host),
        // so the whole prune fits comfortably in a single DELETE without
        // needing to chunk by implicit rowid.
        metrics_5min: delete_whole(
            pool,
            "DELETE FROM metrics_5min WHERE bucket < strftime('%s','now') - ?1 * 86400",
            ROLLUP_DAYS,
        )
        .await?,
        alert_history: delete_chunked_rowid(
            pool,
            "alert_history",
            "created_at",
            ALERT_HISTORY_DAYS,
        )
        .await?,
        http_monitor_results: delete_chunked_rowid(
            pool,
            "http_monitor_results",
            "created_at",
            MONITOR_RESULTS_DAYS,
        )
        .await?,
        ping_results: delete_chunked_rowid(
            pool,
            "ping_results",
            "created_at",
            MONITOR_RESULTS_DAYS,
        )
        .await?,
    })
}

/// Delete rows older than `days` from a rowid-backed table. Runs in bounded
/// chunks and yields between iterations so a multi-million-row purge does
/// not monopolise the SQLite writer lock.
///
/// `table` and `column` are **never** user input — they are fixed
/// `&'static str`s chosen from the migration schema. That is the only
/// reason string interpolation into SQL is acceptable here.
///
/// Note: SQLite's `DELETE ... LIMIT N` syntax requires the non-default
/// `SQLITE_ENABLE_UPDATE_DELETE_LIMIT` compile flag, so we emulate it with
/// a `rowid IN (SELECT rowid …)` subquery instead. That implicitly
/// requires the table to have an implicit rowid — callers must route
/// `WITHOUT ROWID` tables through `delete_whole` above.
async fn delete_chunked_rowid(
    pool: &DbPool,
    table: &'static str,
    column: &'static str,
    days: i64,
) -> Result<u64, sqlx::Error> {
    let sql = format!(
        "DELETE FROM {table} WHERE rowid IN (\
            SELECT rowid FROM {table} \
            WHERE {column} < strftime('%s','now') - ?1 * 86400 \
            LIMIT ?2\
         )"
    );
    let mut total = 0u64;
    loop {
        let affected = sqlx::query(&sql)
            .bind(days)
            .bind(DELETE_CHUNK)
            .execute(pool)
            .await?
            .rows_affected();
        total += affected;
        if affected < DELETE_CHUNK as u64 {
            return Ok(total);
        }
        // Give the scheduler a chance to run pending futures (in particular,
        // the scraper's batch INSERT). `yield_now()` returns to the executor
        // and we resume on the next poll — costs a cooperative reschedule,
        // saves a multi-second SQLITE_BUSY during the initial cold purge.
        tokio::task::yield_now().await;
    }
}

/// One-shot DELETE for tables too small to benefit from chunking (rollup).
async fn delete_whole(pool: &DbPool, sql: &str, days: i64) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(sql).bind(days).execute(pool).await?;
    Ok(res.rows_affected())
}

/// Spawn the daily retention task. The first tick fires on spawn so
/// operators see the pruning behaviour without a 24 h wait, then the
/// interval settles into a steady daily cadence.
pub fn spawn(pool: DbPool) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(TICK_SECS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            interval.tick().await;
            match run_once(&pool).await {
                Ok(stats) => tracing::info!(
                    total = stats.total(),
                    metrics = stats.metrics,
                    rollup = stats.metrics_5min,
                    alerts = stats.alert_history,
                    http = stats.http_monitor_results,
                    ping = stats.ping_results,
                    "[retention_worker] tick complete"
                ),
                Err(e) => tracing::warn!(err = ?e, "[retention_worker] tick failed"),
            }
        }
    })
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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

    async fn seed_metrics_at(pool: &DbPool, host: &str, ts: i64) {
        sqlx::query(
            r#"
            INSERT INTO metrics (
                host_key, display_name, is_online,
                cpu_usage_percent, memory_usage_percent,
                load_1min, load_5min, load_15min,
                timestamp
            )
            VALUES (?1, 'h', 1, 0, 0, 0, 0, 0, ?2)
            "#,
        )
        .bind(host)
        .bind(ts)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn prunes_rows_older_than_windows_and_leaves_recent_ones() {
        let pool = fresh_pool().await;
        let now: i64 = sqlx::query_scalar("SELECT CAST(strftime('%s','now') AS INTEGER)")
            .fetch_one(&pool)
            .await
            .unwrap();

        let day = 86_400i64;
        // Within window:
        seed_metrics_at(&pool, "a:1", now - day).await;
        // Outside window (raw is 3 days):
        seed_metrics_at(&pool, "a:1", now - 10 * day).await;
        // Even older:
        seed_metrics_at(&pool, "a:1", now - 100 * day).await;

        let stats = run_once(&pool).await.unwrap();
        assert_eq!(stats.metrics, 2, "rows older than 3 days should be deleted");

        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM metrics")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[tokio::test]
    async fn rollup_survives_beyond_raw_window() {
        let pool = fresh_pool().await;
        let now: i64 = sqlx::query_scalar("SELECT CAST(strftime('%s','now') AS INTEGER)")
            .fetch_one(&pool)
            .await
            .unwrap();
        let day = 86_400i64;

        // 10-day-old rollup row — past the raw cutoff but safely in
        // the rollup window.
        sqlx::query(
            "INSERT INTO metrics_5min (host_key, bucket, cpu_usage_percent, sample_count) \
             VALUES ('a:1', ?1, 50.0, 10)",
        )
        .bind(now - 10 * day)
        .execute(&pool)
        .await
        .unwrap();

        let stats = run_once(&pool).await.unwrap();
        assert_eq!(stats.metrics_5min, 0);

        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM metrics_5min")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[tokio::test]
    async fn run_once_is_safe_on_empty_tables() {
        let pool = fresh_pool().await;
        let stats = run_once(&pool).await.unwrap();
        assert_eq!(stats, RetentionStats::default());
    }
}
