//! 5-minute rollup worker for the SQLite backend.
//!
//! On Postgres the `metrics_5min` view is a TimescaleDB continuous
//! aggregate that the database maintains on our behalf. SQLite has
//! no equivalent, so this worker owns the same contract: every
//! 60 seconds it upserts the *previous* and *current* 5-minute
//! buckets into `metrics_5min` from `metrics`.
//!
//! Why both buckets, every tick:
//!   • The current bucket is still filling — we want dashboards to
//!     see partial data.
//!   • The previous bucket may still receive late inserts from slow
//!     scrapes. Re-aggregating it once more is cheap and idempotent
//!     (UPSERT overwrites).
//!
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::db::DbPool;

/// Width of a rollup bucket, in seconds.
const BUCKET_SECS: i64 = 300;

/// How often the worker re-aggregates. Two adjacent buckets every
/// tick means staleness is bounded by `TICK_SECS` for data already in
/// the current bucket, and by `2 * TICK_SECS` for very-late arrivals
/// that only land *after* the bucket they belong to has closed.
const TICK_SECS: u64 = 60;

/// Run exactly one rollup pass, aggregating the current and previous
/// buckets. Exposed for tests and for a future `/internal/rollup`
/// hand-crank endpoint; the production path calls it via `spawn`.
///
/// Returns the number of rows written (each host × each of the two
/// buckets counts as one row). Errors propagate so the caller can
/// decide whether to log-and-continue or abort.
pub async fn run_once(pool: &DbPool) -> Result<u64, sqlx::Error> {
    let now: i64 = sqlx::query_scalar("SELECT CAST(strftime('%s','now') AS INTEGER)")
        .fetch_one(pool)
        .await?;
    let current_bucket = (now / BUCKET_SECS) * BUCKET_SECS;
    let previous_bucket = current_bucket - BUCKET_SECS;

    // The current bucket is re-aggregated every tick so dashboards see partial
    // data with at most `TICK_SECS` staleness.
    //
    // The previous bucket only needs a refresh to pick up late arrivals — a
    // scrape that started before the bucket closed but landed after it did.
    // Before this optimization, every 60 s tick re-aggregated the previous
    // bucket regardless of how long ago it closed, which doubled the rollup
    // workload (and the writer-lock contention with the scraper batch INSERT)
    // for data that had not changed in minutes. Run the previous bucket only
    // on the tick immediately after it closed — any later straggler is a
    // pathological case we intentionally accept as not-rolled.
    let previous_rows = if (now - current_bucket) < (TICK_SECS as i64) * 2 {
        rollup_bucket(pool, previous_bucket).await?
    } else {
        0
    };
    let current_rows = rollup_bucket(pool, current_bucket).await?;
    Ok(previous_rows + current_rows)
}

/// Upsert a single 5-minute bucket from raw `metrics` into
/// `metrics_5min`. The query is idempotent: re-running it for the
/// same bucket overwrites scalar aggregates and refreshes the
/// "last-in-bucket" JSON snapshots to reflect any late inserts.
async fn rollup_bucket(pool: &DbPool, bucket_start: i64) -> Result<u64, sqlx::Error> {
    let bucket_end = bucket_start + BUCKET_SECS;

    // Previous shape issued **four correlated subqueries per host row**
    // (disks / temperatures / gpus / docker_stats), each a fresh index
    // probe on `metrics`. At 100 hosts × 30 samples/bucket × 4 snapshots
    // that was ~12 000 json-column fetches per 60 s tick, each holding
    // the SQLite writer lock against the scraper batch INSERT.
    //
    // New shape: a single `tagged` CTE scans the bucket once, stamping
    // every row with `ROW_NUMBER() OVER (PARTITION BY host_key
    // ORDER BY timestamp DESC, id DESC)`. The outer GROUP BY then picks
    // the "last-in-bucket" snapshot with `MAX(CASE WHEN rn = 1 THEN col END)`
    // — SQLite's idiomatic equivalent of PostgreSQL's `last(col, timestamp)`
    // — without re-entering the table. Scalar aggregates (AVG/MIN/COUNT)
    // piggyback on the same scan.
    //
    // `networks` is stored as TEXT JSON on the raw side; the rollup table
    // splits it into scalar columns so uptime / bandwidth queries don't
    // have to re-parse JSON on every read.
    // Bandwidth aggregation (`avg_*_bytes_per_sec`) uses AVG because each
    // raw sample already carries a rate — the agent differentiates the
    // kernel counter across its own 200 ms window before sending. Taking
    // the bucket average mirrors how `cpu_usage_percent` is rolled up and
    // yields a real bandwidth that long-range dashboard queries can read
    // without re-deriving it from `MAX(total_*_bytes)` deltas.
    let res = sqlx::query(
        r#"
        WITH tagged AS (
            SELECT
                host_key,
                timestamp,
                id,
                is_online,
                cpu_usage_percent,
                memory_usage_percent,
                load_1min, load_5min, load_15min,
                rx_bytes_per_sec, tx_bytes_per_sec,
                CAST(json_extract(networks, '$.total_rx_bytes') AS INTEGER) AS rx_total,
                CAST(json_extract(networks, '$.total_tx_bytes') AS INTEGER) AS tx_total,
                disks, temperatures, gpus, docker_stats,
                ROW_NUMBER() OVER (PARTITION BY host_key
                                   ORDER BY timestamp DESC, id DESC) AS rn
            FROM metrics
            WHERE timestamp >= ?1 AND timestamp < ?2
        )
        INSERT INTO metrics_5min (
            host_key, bucket,
            cpu_usage_percent, memory_usage_percent,
            load_1min, load_5min, load_15min,
            is_online, sample_count,
            total_rx_bytes, total_tx_bytes,
            avg_rx_bytes_per_sec, avg_tx_bytes_per_sec,
            disks, temperatures, gpus, docker_stats
        )
        SELECT
            host_key,
            ?1 AS bucket,
            CAST(AVG(cpu_usage_percent) AS REAL)    AS cpu_usage_percent,
            CAST(AVG(memory_usage_percent) AS REAL) AS memory_usage_percent,
            CAST(AVG(load_1min)  AS REAL) AS load_1min,
            CAST(AVG(load_5min)  AS REAL) AS load_5min,
            CAST(AVG(load_15min) AS REAL) AS load_15min,
            MIN(is_online) AS is_online,
            COUNT(*)       AS sample_count,
            MAX(rx_total)  AS total_rx_bytes,
            MAX(tx_total)  AS total_tx_bytes,
            CAST(AVG(rx_bytes_per_sec) AS REAL) AS avg_rx_bytes_per_sec,
            CAST(AVG(tx_bytes_per_sec) AS REAL) AS avg_tx_bytes_per_sec,
            MAX(CASE WHEN rn = 1 THEN disks END)        AS disks,
            MAX(CASE WHEN rn = 1 THEN temperatures END) AS temperatures,
            MAX(CASE WHEN rn = 1 THEN gpus END)         AS gpus,
            MAX(CASE WHEN rn = 1 THEN docker_stats END) AS docker_stats
        FROM tagged
        GROUP BY host_key
        ON CONFLICT (host_key, bucket) DO UPDATE SET
            cpu_usage_percent    = excluded.cpu_usage_percent,
            memory_usage_percent = excluded.memory_usage_percent,
            load_1min            = excluded.load_1min,
            load_5min            = excluded.load_5min,
            load_15min           = excluded.load_15min,
            is_online            = excluded.is_online,
            sample_count         = excluded.sample_count,
            total_rx_bytes       = excluded.total_rx_bytes,
            total_tx_bytes       = excluded.total_tx_bytes,
            avg_rx_bytes_per_sec = excluded.avg_rx_bytes_per_sec,
            avg_tx_bytes_per_sec = excluded.avg_tx_bytes_per_sec,
            disks                = excluded.disks,
            temperatures         = excluded.temperatures,
            gpus                 = excluded.gpus,
            docker_stats         = excluded.docker_stats
        "#,
    )
    .bind(bucket_start)
    .bind(bucket_end)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// Spawn the background rollup task. The returned `JoinHandle` is
/// kept alive by the caller; dropping it aborts the worker on
/// shutdown, which is the intended behaviour.
pub fn spawn(pool: DbPool) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Skip the immediate first tick — let the rest of the startup
        // flow settle before we touch the database.
        let mut interval = tokio::time::interval(Duration::from_secs(TICK_SECS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;

        loop {
            interval.tick().await;
            match run_once(&pool).await {
                Ok(rows) => tracing::debug!(rows, "[rollup_worker] tick complete"),
                Err(e) => tracing::warn!(err = ?e, "[rollup_worker] tick failed"),
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

    /// Drop a synthetic raw-metrics row into a specific second.
    async fn seed_metric(
        pool: &DbPool,
        host: &str,
        cpu: f32,
        online: bool,
        rx: i64,
        tx: i64,
        timestamp: i64,
    ) {
        // Seed a bandwidth proportional to the counter so
        // `test_rollup_aggregates_bandwidth_averages` below can verify
        // the AVG() column without having to hand-tune three inputs.
        let rx_bps = rx as f64;
        let tx_bps = tx as f64;
        let networks = format!(
            r#"{{"total_rx_bytes":{rx},"total_tx_bytes":{tx},"rx_bytes_per_sec":{rx_bps},"tx_bytes_per_sec":{tx_bps}}}"#,
        );
        sqlx::query(
            r#"
            INSERT INTO metrics (
                host_key, display_name, is_online,
                cpu_usage_percent, memory_usage_percent,
                load_1min, load_5min, load_15min,
                networks, rx_bytes_per_sec, tx_bytes_per_sec,
                disks, temperatures, gpus, docker_stats,
                timestamp
            )
            VALUES (?1, 'test', ?2, ?3, 0.0, 0.0, 0.0, 0.0, ?4,
                    ?5, ?6,
                    '[{"name":"/","usage_percent":10}]',
                    '[{"label":"cpu","temperature_c":42}]',
                    '[]', '[]',
                    ?7)
            "#,
        )
        .bind(host)
        .bind(online)
        .bind(cpu)
        .bind(&networks)
        .bind(rx_bps)
        .bind(tx_bps)
        .bind(timestamp)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Align timestamp to the current bucket so `run_once` aggregates
    /// rows we control.
    async fn current_bucket_start(pool: &DbPool) -> i64 {
        let now: i64 = sqlx::query_scalar("SELECT CAST(strftime('%s','now') AS INTEGER)")
            .fetch_one(pool)
            .await
            .unwrap();
        (now / BUCKET_SECS) * BUCKET_SECS
    }

    #[tokio::test]
    async fn rollup_aggregates_scalar_columns() {
        let pool = fresh_pool().await;
        let bucket = current_bucket_start(&pool).await;

        // Three samples for host "a" spread over the bucket.
        seed_metric(&pool, "a:9101", 10.0, true, 100, 50, bucket + 10).await;
        seed_metric(&pool, "a:9101", 30.0, true, 300, 150, bucket + 120).await;
        seed_metric(&pool, "a:9101", 50.0, false, 500, 250, bucket + 240).await;

        let rows = run_once(&pool).await.unwrap();
        assert!(rows >= 1);

        let row: (f32, i32, i64, i64, i64) = sqlx::query_as(
            "SELECT cpu_usage_percent, is_online, sample_count, total_rx_bytes, total_tx_bytes \
             FROM metrics_5min WHERE host_key = 'a:9101' AND bucket = ?1",
        )
        .bind(bucket)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!((row.0 - 30.0).abs() < 0.01, "AVG(cpu) mismatch: {}", row.0);
        assert_eq!(
            row.1, 0,
            "MIN(is_online) must be 0 when any sample is offline"
        );
        assert_eq!(row.2, 3, "sample_count");
        assert_eq!(
            row.3, 500,
            "MAX(rx) reads json_extract on the raw networks column"
        );
        assert_eq!(row.4, 250);
    }

    #[tokio::test]
    async fn rollup_aggregates_bandwidth_averages() {
        // `seed_metric` stores scalar `rx_bytes_per_sec = rx` and same for tx.
        // With three samples of rx = 100, 300, 500 the expected bucket
        // average is 300 — this pins the raw scalar-column rollup path.
        let pool = fresh_pool().await;
        let bucket = current_bucket_start(&pool).await;

        seed_metric(&pool, "a:9101", 10.0, true, 100, 50, bucket + 10).await;
        seed_metric(&pool, "a:9101", 30.0, true, 300, 150, bucket + 120).await;
        seed_metric(&pool, "a:9101", 50.0, true, 500, 250, bucket + 240).await;

        run_once(&pool).await.unwrap();

        let (avg_rx, avg_tx): (f64, f64) = sqlx::query_as(
            "SELECT avg_rx_bytes_per_sec, avg_tx_bytes_per_sec \
             FROM metrics_5min WHERE host_key = 'a:9101' AND bucket = ?1",
        )
        .bind(bucket)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(
            (avg_rx - 300.0).abs() < 0.01,
            "AVG(rx_bytes_per_sec) mismatch: {avg_rx}"
        );
        assert!(
            (avg_tx - 150.0).abs() < 0.01,
            "AVG(tx_bytes_per_sec) mismatch: {avg_tx}"
        );
    }

    #[tokio::test]
    async fn rollup_is_idempotent() {
        let pool = fresh_pool().await;
        let bucket = current_bucket_start(&pool).await;
        seed_metric(&pool, "a:9101", 10.0, true, 100, 50, bucket + 10).await;

        run_once(&pool).await.unwrap();
        run_once(&pool).await.unwrap();

        let cnt: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM metrics_5min WHERE host_key = 'a:9101' AND bucket = ?1",
        )
        .bind(bucket)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(cnt, 1, "second run must UPDATE, not duplicate");
    }

    #[tokio::test]
    async fn rollup_picks_last_snapshot_deterministically() {
        // Two snapshots inserted in the same second must not leave the
        // "latest-in-bucket" column ambiguous — (timestamp DESC, id DESC)
        // pins it to the higher rowid.
        let pool = fresh_pool().await;
        let bucket = current_bucket_start(&pool).await;

        seed_metric(&pool, "a:9101", 10.0, true, 100, 50, bucket + 10).await;
        seed_metric(&pool, "a:9101", 20.0, true, 200, 100, bucket + 10).await;
        // Manually override the second insert's disks payload via UPDATE
        // — easier than a dedicated seeder. The higher id wins.
        sqlx::query(
            "UPDATE metrics SET disks = '[{\"name\":\"/var\",\"usage_percent\":77}]' \
             WHERE id = (SELECT MAX(id) FROM metrics WHERE host_key = 'a:9101')",
        )
        .execute(&pool)
        .await
        .unwrap();

        run_once(&pool).await.unwrap();

        let disks: String = sqlx::query_scalar(
            "SELECT disks FROM metrics_5min WHERE host_key = 'a:9101' AND bucket = ?1",
        )
        .bind(bucket)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(disks.contains("/var"), "last-in-bucket should win: {disks}");
    }

    #[tokio::test]
    async fn fetch_metrics_range_returns_rollup_rows_after_worker_runs() {
        // End-to-end proof: once the rollup worker has run, the 6h–14d
        // tier of fetch_metrics_range starts returning rows. Without
        // this test, a regression in the rollup SQL would silently
        // empty the dashboard's longer ranges.
        use crate::repositories::metrics_repo::fetch_metrics_range;
        use chrono::{TimeZone, Utc};

        let pool = fresh_pool().await;
        let bucket = current_bucket_start(&pool).await;
        seed_metric(&pool, "a:9101", 42.0, true, 1_000_000, 500_000, bucket + 30).await;

        run_once(&pool).await.unwrap();

        // Ask for a 24-hour window around the bucket — that falls in
        // the 6h–14d tier which reads from `metrics_5min` directly.
        let center = Utc.timestamp_opt(bucket + 150, 0).unwrap();
        let rows = fetch_metrics_range(
            &pool,
            "a:9101",
            center - chrono::Duration::hours(12),
            center + chrono::Duration::hours(12),
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 1, "rollup row should populate the 6h–14d tier");
        assert!((rows[0].cpu_usage_percent - 42.0).abs() < 0.01);
        // `networks` was synthesized from the scalar rx/tx in
        // `metrics_5min` via `json_object` at read time.
        let net = rows[0].networks.as_ref().expect("networks JSON");
        assert_eq!(net["total_rx_bytes"], 1_000_000i64);
    }
}
