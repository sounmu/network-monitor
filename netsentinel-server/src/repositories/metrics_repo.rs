use std::collections::HashMap;

use chrono::{DateTime, Utc};
use chrono_tz::Asia::Seoul;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;

use crate::models::agent_metrics::AgentMetrics;

pub mod kst_date_format {
    use super::*;
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(date: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let kst_date = date.with_timezone(&Seoul);
        let s = format!("{}", kst_date.format("%Y-%m-%dT%H:%M:%S%z"));
        serializer.serialize_str(&s)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        DateTime::parse_from_rfc3339(&s)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(serde::de::Error::custom)
    }
}

// ──────────────────────────────────────────────
// Legacy database initialisation (replaced by sqlx migrations)
// ──────────────────────────────────────────────

/// Legacy init_db — now handled by sqlx::migrate!() in main.rs.
/// Kept for reference; migration files are in migrations/ directory.
#[allow(dead_code)]
#[tracing::instrument(skip(pool))]
pub async fn init_db(pool: &PgPool) -> Result<(), sqlx::Error> {
    // Enable the TimescaleDB extension (must run before hypertable conversion)
    sqlx::query("CREATE EXTENSION IF NOT EXISTS timescaledb")
        .execute(pool)
        .await?;

    // Create the metrics table — all columns declared upfront, no migration needed.
    // No PRIMARY KEY on id — TimescaleDB unique constraints must include the partition key (timestamp).
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS metrics (
            id                   BIGSERIAL,
            host_key             VARCHAR(255) NOT NULL,
            display_name         VARCHAR(255) NOT NULL,
            is_online            BOOLEAN NOT NULL,
            cpu_usage_percent    REAL NOT NULL,
            memory_usage_percent REAL NOT NULL,
            load_1min            REAL NOT NULL DEFAULT 0.0,
            load_5min            REAL NOT NULL DEFAULT 0.0,
            load_15min           REAL NOT NULL DEFAULT 0.0,
            networks             JSONB,
            docker_containers    JSONB,
            ports                JSONB,
            disks                JSONB,
            processes            JSONB,
            temperatures         JSONB,
            gpus                 JSONB,
            timestamp            TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    // Migration: add JSONB columns if missing (for existing databases)
    for col in ["disks", "processes", "temperatures", "gpus"] {
        sqlx::query(&format!(
            "ALTER TABLE metrics ADD COLUMN IF NOT EXISTS {} JSONB",
            col
        ))
        .execute(pool)
        .await?;
    }

    // Composite index: (host_key, timestamp DESC) — optimised for dashboard time-series queries
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_metrics_host_key_time
            ON metrics (host_key, timestamp DESC)
        "#,
    )
    .execute(pool)
    .await?;

    // ── TimescaleDB: hypertable conversion + automatic retention policy ──

    // Convert to hypertable: partitions time-series data into 1-day chunks.
    // - Chunk pruning: time-range queries skip irrelevant chunks, drastically reducing I/O.
    // - chunk_time_interval '1 day': ~8,640 rows/host/day at 10-second scrape interval — appropriate chunk size.
    // - migrate_data: redistributes any pre-existing rows into time-based chunks.
    sqlx::query(
        r#"
        SELECT create_hypertable(
            'metrics', 'timestamp',
            chunk_time_interval => INTERVAL '1 day',
            if_not_exists => TRUE,
            migrate_data => TRUE
        )
        "#,
    )
    .execute(pool)
    .await?;

    // Retention policy: chunks older than 90 days are automatically DROPped by a TimescaleDB background worker.
    // - Advantage over row-level DELETE: whole-chunk DROP eliminates table bloat and vacuum overhead.
    sqlx::query(
        r#"
        SELECT add_retention_policy(
            'metrics',
            INTERVAL '90 days',
            if_not_exists => TRUE
        )
        "#,
    )
    .execute(pool)
    .await?;

    // ── TimescaleDB: continuous aggregate (5-minute rollups) ──
    // Pre-computes 5-minute averages in the background so long-range queries (7d, 30d)
    // read ~50x fewer rows from a materialized view instead of scanning raw data.
    sqlx::query(
        r#"
        CREATE MATERIALIZED VIEW IF NOT EXISTS metrics_5min
        WITH (timescaledb.continuous) AS
        SELECT
            host_key,
            time_bucket('5 minutes', timestamp) AS bucket,
            AVG(cpu_usage_percent)::REAL AS cpu_usage_percent,
            AVG(memory_usage_percent)::REAL AS memory_usage_percent,
            AVG(load_1min)::REAL AS load_1min,
            AVG(load_5min)::REAL AS load_5min,
            AVG(load_15min)::REAL AS load_15min,
            bool_and(is_online) AS is_online,
            COUNT(*)::INT AS sample_count
        FROM metrics
        GROUP BY host_key, time_bucket('5 minutes', timestamp)
        WITH NO DATA
        "#,
    )
    .execute(pool)
    .await
    .ok(); // OK to fail if view already exists with different definition

    // Refresh policy: keep the continuous aggregate up-to-date.
    // start_offset = 3 days: re-aggregates recent data that may still receive late inserts.
    // end_offset = 5 minutes: don't aggregate the most recent incomplete bucket.
    // schedule_interval = 5 minutes: how often the background job runs.
    sqlx::query(
        r#"
        SELECT add_continuous_aggregate_policy('metrics_5min',
            start_offset => INTERVAL '3 days',
            end_offset   => INTERVAL '5 minutes',
            schedule_interval => INTERVAL '5 minutes',
            if_not_exists => TRUE
        )
        "#,
    )
    .execute(pool)
    .await
    .ok(); // OK to fail if policy already exists

    // ── TimescaleDB: chunk compression for older data ──
    // Compress chunks older than 7 days to reduce disk I/O on long-range scans.
    // Compressed chunks use columnar storage — ~10x smaller on disk.
    sqlx::query(
        r#"
        ALTER TABLE metrics SET (
            timescaledb.compress,
            timescaledb.compress_segmentby = 'host_key',
            timescaledb.compress_orderby = 'timestamp DESC'
        )
        "#,
    )
    .execute(pool)
    .await
    .ok(); // OK to fail if already configured

    sqlx::query(
        r#"
        SELECT add_compression_policy(
            'metrics',
            compress_after => INTERVAL '7 days',
            if_not_exists => TRUE
        )
        "#,
    )
    .execute(pool)
    .await
    .ok(); // OK to fail if policy already exists

    // ── hosts table: agent (host) registry ──
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hosts (
            host_key             TEXT PRIMARY KEY,
            display_name         TEXT NOT NULL,
            scrape_interval_secs INT NOT NULL DEFAULT 10,
            load_threshold       FLOAT NOT NULL DEFAULT 4.0,
            ports                INT[] NOT NULL DEFAULT '{80,443}',
            containers           TEXT[] NOT NULL DEFAULT '{}',
            created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    // ── alert_configs table: alert rules (global + per-host overrides) ──
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS alert_configs (
            id              SERIAL PRIMARY KEY,
            host_key        TEXT REFERENCES hosts(host_key) ON DELETE CASCADE,
            metric_type     TEXT NOT NULL CHECK (metric_type IN ('cpu', 'memory', 'disk')),
            enabled         BOOLEAN NOT NULL DEFAULT true,
            threshold       FLOAT NOT NULL,
            sustained_secs  INT NOT NULL DEFAULT 300,
            cooldown_secs   INT NOT NULL DEFAULT 60,
            updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            UNIQUE NULLS NOT DISTINCT (host_key, metric_type)
        )
        "#,
    )
    .execute(pool)
    .await?;

    // Migration: update CHECK constraint to include 'disk' metric type (for existing databases)
    sqlx::query(
        r#"
        DO $$
        BEGIN
            IF EXISTS (
                SELECT 1 FROM information_schema.check_constraints
                WHERE constraint_name = 'alert_configs_metric_type_check'
            ) THEN
                ALTER TABLE alert_configs DROP CONSTRAINT alert_configs_metric_type_check;
                ALTER TABLE alert_configs ADD CONSTRAINT alert_configs_metric_type_check
                    CHECK (metric_type IN ('cpu', 'memory', 'disk'));
            END IF;
        END $$
        "#,
    )
    .execute(pool)
    .await?;

    // Seed global default alert thresholds (ignored if already present)
    sqlx::query(
        r#"
        INSERT INTO alert_configs (host_key, metric_type, enabled, threshold, sustained_secs, cooldown_secs)
        VALUES (NULL, 'cpu', true, 80.0, 300, 60),
               (NULL, 'memory', true, 90.0, 300, 60),
               (NULL, 'disk', true, 90.0, 0, 300)
        ON CONFLICT (host_key, metric_type) DO NOTHING
        "#,
    )
    .execute(pool)
    .await?;

    // ── notification_channels table ──
    crate::repositories::notification_channels_repo::init_table(pool).await?;

    // ── alert_history table ──
    crate::repositories::alert_history_repo::init_table(pool).await?;

    // ── users + dashboard_layouts tables ──
    crate::repositories::users_repo::init_table(pool).await?;
    crate::repositories::dashboard_repo::init_table(pool).await?;

    // ── http_monitors + ping_monitors tables ──
    crate::repositories::http_monitors_repo::init_tables(pool).await?;
    crate::repositories::ping_monitors_repo::init_tables(pool).await?;

    tracing::info!(
        "✅ [DB] Tables ready: metrics (TimescaleDB), hosts, alert_configs, notification_channels."
    );
    Ok(())
}

// ──────────────────────────────────────────────
// Insert
// ──────────────────────────────────────────────

/// Persist collected agent metrics to the database.
/// Batch-insert metrics for multiple hosts in a single query.
/// Reduces DB round-trips from N (one per host) to 1 per scrape cycle.
pub async fn insert_metrics_batch(
    pool: &PgPool,
    batch: &[(&str, &AgentMetrics)],
) -> Result<(), sqlx::Error> {
    if batch.is_empty() {
        return Ok(());
    }

    let mut qb: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(
        "INSERT INTO metrics (\
         host_key, display_name, is_online, \
         cpu_usage_percent, memory_usage_percent, \
         load_1min, load_5min, load_15min, \
         networks, docker_containers, ports, disks, \
         processes, temperatures, gpus) ",
    );

    qb.push_values(batch, |mut b, (host_key, metrics)| {
        b.push_bind(host_key.to_string())
            .push_bind(metrics.hostname.clone())
            .push_bind(metrics.is_online)
            .push_bind(metrics.system.cpu_usage_percent)
            .push_bind(metrics.system.memory_usage_percent)
            .push_bind(metrics.load_average.one_min as f32)
            .push_bind(metrics.load_average.five_min as f32)
            .push_bind(metrics.load_average.fifteen_min as f32)
            .push_bind(sqlx::types::Json(&metrics.network))
            .push_bind(sqlx::types::Json(&metrics.docker_containers))
            .push_bind(sqlx::types::Json(&metrics.ports))
            .push_bind(sqlx::types::Json(&metrics.system.disks))
            .push_bind(sqlx::types::Json(&metrics.system.processes))
            .push_bind(sqlx::types::Json(&metrics.system.temperatures))
            .push_bind(sqlx::types::Json(&metrics.system.gpus));
    });

    qb.build().execute(pool).await?;
    Ok(())
}

/// Batch-insert offline metric records for multiple unreachable hosts.
pub async fn insert_offline_metrics_batch(
    pool: &PgPool,
    batch: &[(&str, &str)], // (host_key, display_name)
) -> Result<(), sqlx::Error> {
    if batch.is_empty() {
        return Ok(());
    }

    let mut qb: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(
        "INSERT INTO metrics (\
         host_key, display_name, is_online, \
         cpu_usage_percent, memory_usage_percent, \
         load_1min, load_5min, load_15min) ",
    );

    qb.push_values(batch, |mut b, (host_key, display_name)| {
        b.push_bind(host_key.to_string())
            .push_bind(display_name.to_string())
            .push_bind(false)
            .push_bind(0.0_f32)
            .push_bind(0.0_f32)
            .push_bind(0.0_f32)
            .push_bind(0.0_f32)
            .push_bind(0.0_f32);
    });

    qb.build().execute(pool).await?;
    Ok(())
}

// ──────────────────────────────────────────────
// Select (GET /api/metrics/:host_key)
// ──────────────────────────────────────────────

/// Row returned to the dashboard for chart rendering
#[derive(Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct MetricsRow {
    pub id: i64,
    /// Target-URL-based unique identifier
    pub host_key: String,
    /// UI display name (OS hostname reported by the agent)
    pub display_name: String,
    pub is_online: bool,
    pub cpu_usage_percent: f32,
    pub memory_usage_percent: f32,
    pub load_1min: f32,
    pub load_5min: f32,
    pub load_15min: f32,
    pub networks: Option<Value>,
    pub docker_containers: Option<Value>,
    pub ports: Option<Value>,
    pub disks: Option<Value>,
    pub processes: Option<Value>,
    pub temperatures: Option<Value>,
    pub gpus: Option<Value>,
    #[serde(with = "kst_date_format")]
    pub timestamp: DateTime<Utc>,
}

/// Fetch the most recent 50 metrics for a host, ordered newest first.
pub async fn fetch_recent_metrics(
    pool: &PgPool,
    host_key: &str,
) -> Result<Vec<MetricsRow>, sqlx::Error> {
    let rows = sqlx::query_as::<_, MetricsRow>(
        r#"
        SELECT id, host_key, display_name, is_online,
               cpu_usage_percent, memory_usage_percent,
               load_1min, load_5min, load_15min,
               networks, docker_containers, ports, disks,
               processes, temperatures, gpus,
               timestamp
        FROM metrics
        WHERE host_key = $1
        ORDER BY timestamp DESC
        LIMIT 50
        "#,
    )
    .bind(host_key)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Fetch metrics for a host within a given time range, ordered oldest first.
///
/// Automatically downsamples long ranges to keep response size manageable:
/// - ≤6h: raw rows (every 10s) from `metrics` table
/// - 6h–14d: 5-minute pre-aggregated rows from `metrics_5min` continuous aggregate (no GROUP BY)
/// - >14d: 15-minute re-aggregated rows from `metrics_5min` CA
///
/// The 6h–3d range previously used 1-minute GROUP BY on the raw table, which was the
/// slowest query path (~500ms on ARM). Switching to the 5-min CA eliminates GROUP BY
/// entirely and reduces response time to ~50ms at the cost of coarser granularity.
///
/// JSONB columns (processes, temperatures, gpus, disks, docker_containers, ports) are
/// excluded from time-range queries — chart rendering only needs scalar metrics.
pub async fn fetch_metrics_range(
    pool: &PgPool,
    host_key: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<MetricsRow>, sqlx::Error> {
    let duration = end - start;
    let hours = duration.num_hours();

    if hours <= 6 {
        // Short range: return raw rows (max ~2,160 rows), but skip heavy JSONB columns
        let rows = sqlx::query_as::<_, MetricsRow>(
            r#"
            SELECT id, host_key, display_name, is_online,
                   cpu_usage_percent, memory_usage_percent,
                   load_1min, load_5min, load_15min,
                   networks,
                   NULL::jsonb AS docker_containers,
                   NULL::jsonb AS ports,
                   NULL::jsonb AS disks,
                   NULL::jsonb AS processes,
                   NULL::jsonb AS temperatures,
                   NULL::jsonb AS gpus,
                   timestamp
            FROM metrics
            WHERE host_key = $1
              AND timestamp >= $2
              AND timestamp <= $3
            ORDER BY timestamp ASC
            "#,
        )
        .bind(host_key)
        .bind(start)
        .bind(end)
        .fetch_all(pool)
        .await?;
        Ok(rows)
    } else if hours <= 336 {
        // Long range (3d–14d): read directly from metrics_5min continuous aggregate.
        // No GROUP BY needed — CA bucket size matches exactly.
        let rows = sqlx::query_as::<_, MetricsRow>(
            r#"
            SELECT
                0::BIGINT AS id,
                $1::VARCHAR AS host_key,
                ''::VARCHAR AS display_name,
                is_online,
                cpu_usage_percent,
                memory_usage_percent,
                load_1min, load_5min, load_15min,
                jsonb_build_object(
                    'total_rx_bytes', total_rx_bytes,
                    'total_tx_bytes', total_tx_bytes
                ) AS networks,
                NULL::jsonb AS docker_containers,
                NULL::jsonb AS ports,
                NULL::jsonb AS disks,
                NULL::jsonb AS processes,
                NULL::jsonb AS temperatures,
                NULL::jsonb AS gpus,
                bucket AS timestamp
            FROM metrics_5min
            WHERE host_key = $1
              AND bucket >= $2
              AND bucket <= $3
            ORDER BY bucket ASC
            "#,
        )
        .bind(host_key)
        .bind(start)
        .bind(end)
        .fetch_all(pool)
        .await?;
        Ok(rows)
    } else {
        // Very long range (>14d): re-aggregate metrics_5min into 15-min buckets.
        // Reads ~8,640 pre-aggregated rows instead of millions of raw rows.
        let rows = sqlx::query_as::<_, MetricsRow>(
            r#"
            SELECT
                0::BIGINT AS id,
                $1::VARCHAR AS host_key,
                ''::VARCHAR AS display_name,
                bool_and(is_online) AS is_online,
                AVG(cpu_usage_percent)::REAL AS cpu_usage_percent,
                AVG(memory_usage_percent)::REAL AS memory_usage_percent,
                AVG(load_1min)::REAL AS load_1min,
                AVG(load_5min)::REAL AS load_5min,
                AVG(load_15min)::REAL AS load_15min,
                jsonb_build_object(
                    'total_rx_bytes', MAX(total_rx_bytes),
                    'total_tx_bytes', MAX(total_tx_bytes)
                ) AS networks,
                NULL::jsonb AS docker_containers,
                NULL::jsonb AS ports,
                NULL::jsonb AS disks,
                NULL::jsonb AS processes,
                NULL::jsonb AS temperatures,
                NULL::jsonb AS gpus,
                time_bucket('15 minutes', bucket) AS timestamp
            FROM metrics_5min
            WHERE host_key = $1
              AND bucket >= $2
              AND bucket <= $3
            GROUP BY time_bucket('15 minutes', bucket)
            ORDER BY timestamp ASC
            "#,
        )
        .bind(host_key)
        .bind(start)
        .bind(end)
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }
}

// ──────────────────────────────────────────────
// Select (GET /api/hosts)
// ──────────────────────────────────────────────

/// Host summary shown in the frontend sidebar
#[derive(Serialize, Deserialize, sqlx::FromRow)]
pub struct HostSummary {
    /// Target-URL-based unique identifier
    pub host_key: String,
    /// UI display name
    pub display_name: String,
    pub is_online: bool,
    #[serde(with = "kst_date_format_opt")]
    pub last_seen: Option<DateTime<Utc>>,
}

pub mod kst_date_format_opt {
    use super::*;
    use serde::{self, Deserializer, Serializer};

    pub fn serialize<S>(date: &Option<DateTime<Utc>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match date {
            Some(dt) => kst_date_format::serialize(dt, serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = Option::<String>::deserialize(deserializer)?;
        match s {
            Some(s) => DateTime::parse_from_rfc3339(&s)
                .map(|dt| Some(dt.with_timezone(&Utc)))
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

/// Fetch all monitored hosts with their latest online status.
///
/// Queries the `hosts` table and LEFT JOINs the most recent metric per host.
/// A host is considered online if its last metric timestamp is within the past 60 seconds.
pub async fn fetch_host_summaries(pool: &PgPool) -> Result<Vec<HostSummary>, sqlx::Error> {
    let rows = sqlx::query_as::<_, HostSummary>(
        r#"
        SELECT
            h.host_key,
            h.display_name,
            COALESCE(m.is_online, false) AS is_online,
            m.last_seen
        FROM hosts h
        LEFT JOIN (
            SELECT DISTINCT ON (host_key)
                host_key,
                (timestamp > NOW() - INTERVAL '60 seconds') AS is_online,
                timestamp AS last_seen
            FROM metrics
            ORDER BY host_key, timestamp DESC
        ) m ON h.host_key = m.host_key
        ORDER BY h.host_key
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

// ──────────────────────────────────────────────
// Uptime (GET /api/uptime/:host_key)
// ──────────────────────────────────────────────

/// Daily uptime data point
#[derive(Serialize, Deserialize, sqlx::FromRow)]
pub struct UptimePoint {
    pub day: DateTime<Utc>,
    pub total_count: i64,
    pub online_count: i64,
    pub uptime_pct: f64,
}

/// Overall uptime summary for a host
#[derive(Serialize)]
pub struct UptimeSummary {
    pub host_key: String,
    pub overall_pct: f64,
    pub daily: Vec<UptimePoint>,
}

/// Fetch 7-day overall uptime percentage for all hosts in a single query.
/// Returns a HashMap<host_key, uptime_pct> — used by public_status to avoid N+1 queries.
pub async fn fetch_batch_uptime_pct(
    pool: &PgPool,
    days: i32,
) -> Result<HashMap<String, f64>, sqlx::Error> {
    let rows: Vec<(String, f64)> = sqlx::query_as(
        r#"
        SELECT
            host_key,
            CASE
                WHEN SUM(sample_count) > 0
                THEN (SUM(CASE WHEN is_online THEN sample_count ELSE 0 END)::FLOAT
                      / SUM(sample_count)::FLOAT) * 100.0
                ELSE 0.0
            END AS uptime_pct
        FROM metrics_5min
        WHERE bucket >= NOW() - ($1 || ' days')::INTERVAL
        GROUP BY host_key
        "#,
    )
    .bind(days.to_string())
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().collect())
}

/// Compute daily uptime percentage for a host over the given number of days.
/// Uses the metrics_5min continuous aggregate for efficient daily re-aggregation.
/// sample_count provides weighted averages for accurate uptime calculation.
pub async fn fetch_uptime(
    pool: &PgPool,
    host_key: &str,
    days: i32,
) -> Result<UptimeSummary, sqlx::Error> {
    let daily = sqlx::query_as::<_, UptimePoint>(
        r#"
        SELECT
            time_bucket('1 day', bucket) AS day,
            SUM(sample_count)::BIGINT AS total_count,
            SUM(CASE WHEN is_online THEN sample_count ELSE 0 END)::BIGINT AS online_count,
            CASE
                WHEN SUM(sample_count) > 0
                THEN (SUM(CASE WHEN is_online THEN sample_count ELSE 0 END)::FLOAT
                      / SUM(sample_count)::FLOAT) * 100.0
                ELSE 0.0
            END AS uptime_pct
        FROM metrics_5min
        WHERE host_key = $1
          AND bucket >= NOW() - ($2 || ' days')::INTERVAL
        GROUP BY day
        ORDER BY day ASC
        "#,
    )
    .bind(host_key)
    .bind(days.to_string())
    .fetch_all(pool)
    .await?;

    let (total, online) = daily.iter().fold((0i64, 0i64), |(t, o), p| {
        (t + p.total_count, o + p.online_count)
    });
    let overall_pct = if total > 0 {
        (online as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    Ok(UptimeSummary {
        host_key: host_key.to_string(),
        overall_pct,
        daily,
    })
}
