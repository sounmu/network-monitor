use std::collections::HashMap;

use crate::db::DbPool;
use chrono::{DateTime, Utc};
use chrono_tz::Asia::Seoul;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::agent_metrics::AgentMetrics;

pub mod kst_date_format {
    use super::*;
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(date: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // `%:z` emits the RFC 3339 offset with a colon (`+09:00`), unlike
        // `%z` (`+0900`) which Safari/Firefox reject in `Date.parse` edge
        // cases and which our own `parse_from_rfc3339` (used in
        // `deserialize` below) refuses to parse — making the previous
        // format asymmetric with its own round-trip.
        let kst_date = date.with_timezone(&Seoul);
        let s = kst_date.format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string();
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
    pub cpu_cores: Option<Value>,
    pub network_interfaces: Option<Value>,
    pub docker_stats: Option<Value>,
    #[serde(with = "kst_date_format")]
    pub timestamp: DateTime<Utc>,
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

// Raw-metrics write + read paths. The 6h–14d and >14d tiers of
// `fetch_metrics_range` query the `metrics_5min` rollup table — the
// rollup worker (services/rollup_worker.rs) populates that table on a
// schedule.
//
// `MetricsRowRaw` holds JSON columns as `Option<String>` and decodes
// them into `serde_json::Value` at the boundary via `TryFrom`.

#[derive(sqlx::FromRow)]
struct MetricsRowRaw {
    id: i64,
    host_key: String,
    display_name: String,
    is_online: Option<bool>,
    cpu_usage_percent: f32,
    memory_usage_percent: f32,
    load_1min: f32,
    load_5min: f32,
    load_15min: f32,
    networks: Option<String>,
    docker_containers: Option<String>,
    ports: Option<String>,
    disks: Option<String>,
    processes: Option<String>,
    temperatures: Option<String>,
    gpus: Option<String>,
    cpu_cores: Option<String>,
    network_interfaces: Option<String>,
    docker_stats: Option<String>,
    rx_bytes_per_sec: Option<f64>,
    tx_bytes_per_sec: Option<f64>,
    timestamp: DateTime<Utc>,
}

fn parse_opt_json(s: Option<String>) -> Result<Option<Value>, sqlx::Error> {
    match s {
        Some(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| sqlx::Error::Decode(Box::new(e))),
        None => Ok(None),
    }
}

impl TryFrom<MetricsRowRaw> for MetricsRow {
    type Error = sqlx::Error;

    fn try_from(raw: MetricsRowRaw) -> Result<Self, Self::Error> {
        let mut networks = parse_opt_json(raw.networks)?;
        if let Some(Value::Object(ref mut map)) = networks {
            if let Some(rx) = raw.rx_bytes_per_sec {
                map.insert("rx_bytes_per_sec".to_string(), Value::from(rx));
            }
            if let Some(tx) = raw.tx_bytes_per_sec {
                map.insert("tx_bytes_per_sec".to_string(), Value::from(tx));
            }
        }

        Ok(Self {
            id: raw.id,
            host_key: raw.host_key,
            display_name: raw.display_name,
            is_online: raw.is_online.unwrap_or(false),
            cpu_usage_percent: raw.cpu_usage_percent,
            memory_usage_percent: raw.memory_usage_percent,
            load_1min: raw.load_1min,
            load_5min: raw.load_5min,
            load_15min: raw.load_15min,
            networks,
            docker_containers: parse_opt_json(raw.docker_containers)?,
            ports: parse_opt_json(raw.ports)?,
            disks: parse_opt_json(raw.disks)?,
            processes: parse_opt_json(raw.processes)?,
            temperatures: parse_opt_json(raw.temperatures)?,
            gpus: parse_opt_json(raw.gpus)?,
            cpu_cores: parse_opt_json(raw.cpu_cores)?,
            network_interfaces: parse_opt_json(raw.network_interfaces)?,
            docker_stats: parse_opt_json(raw.docker_stats)?,
            timestamp: raw.timestamp,
        })
    }
}

/// Persist collected agent metrics to the database.
/// Batch-insert metrics for multiple hosts in a single query.
/// Reduces DB round-trips from N (one per host) to 1 per scrape cycle.
pub async fn insert_metrics_batch(
    pool: &DbPool,
    batch: &[(&str, &AgentMetrics)],
) -> Result<(), sqlx::Error> {
    if batch.is_empty() {
        return Ok(());
    }

    let mut qb: sqlx::QueryBuilder<sqlx::Sqlite> = sqlx::QueryBuilder::new(
        "INSERT INTO metrics (\
         host_key, display_name, is_online, \
         cpu_usage_percent, memory_usage_percent, \
         load_1min, load_5min, load_15min, \
         networks, docker_containers, ports, disks, \
         processes, temperatures, gpus, \
         cpu_cores, network_interfaces, docker_stats, \
         rx_bytes_per_sec, tx_bytes_per_sec) ",
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
            .push_bind(sqlx::types::Json(&metrics.system.gpus))
            .push_bind(sqlx::types::Json(&metrics.cpu_cores))
            .push_bind(sqlx::types::Json(&metrics.network_interfaces))
            .push_bind(sqlx::types::Json(&metrics.docker_stats))
            .push_bind(metrics.network.rx_bytes_per_sec)
            .push_bind(metrics.network.tx_bytes_per_sec);
    });

    qb.build().execute(pool).await?;
    Ok(())
}

/// Batch-insert offline metric records for multiple unreachable hosts.
pub async fn insert_offline_metrics_batch(
    pool: &DbPool,
    batch: &[(&str, &str)],
) -> Result<(), sqlx::Error> {
    if batch.is_empty() {
        return Ok(());
    }

    let mut qb: sqlx::QueryBuilder<sqlx::Sqlite> = sqlx::QueryBuilder::new(
        "INSERT INTO metrics (\
         host_key, display_name, is_online, \
         cpu_usage_percent, memory_usage_percent, \
         load_1min, load_5min, load_15min, \
         rx_bytes_per_sec, tx_bytes_per_sec) ",
    );
    qb.push_values(batch, |mut b, (host_key, display_name)| {
        b.push_bind(host_key.to_string())
            .push_bind(display_name.to_string())
            .push_bind(false)
            .push_bind(0f32)
            .push_bind(0f32)
            .push_bind(0f32)
            .push_bind(0f32)
            .push_bind(0f32)
            .push_bind(0f64)
            .push_bind(0f64);
    });
    qb.build().execute(pool).await?;
    Ok(())
}

/// Fetch the most recent 50 metrics for a host, ordered newest first.
pub async fn fetch_recent_metrics(
    pool: &DbPool,
    host_key: &str,
) -> Result<Vec<MetricsRow>, sqlx::Error> {
    let raws = sqlx::query_as::<_, MetricsRowRaw>(
        r#"
        SELECT id, host_key, display_name, is_online,
               cpu_usage_percent, memory_usage_percent,
               load_1min, load_5min, load_15min,
               networks, docker_containers, ports, disks,
               processes, temperatures, gpus,
               cpu_cores, network_interfaces, docker_stats,
               rx_bytes_per_sec, tx_bytes_per_sec,
               timestamp
        FROM metrics
        WHERE host_key = ?1
        ORDER BY timestamp DESC, id DESC
        LIMIT 50
        "#,
    )
    .bind(host_key)
    .fetch_all(pool)
    .await?;
    raws.into_iter().map(MetricsRow::try_from).collect()
}

/// Fetch metrics for a host within a given time range, ordered oldest first.
///
/// Automatically downsamples long ranges to keep response size manageable:
/// - ≤6h: raw rows (every 10s) from `metrics` table
/// - 6h–14d: 5-minute pre-aggregated rows from `metrics_5min` rollup table
/// - >14d: 15-minute re-aggregated rows from `metrics_5min`
///
/// JSON columns (processes, temperatures, gpus, disks, docker_containers, ports) are
/// trimmed from time-range queries when the chart layer doesn't need them.
pub async fn fetch_metrics_range(
    pool: &DbPool,
    host_key: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<MetricsRow>, sqlx::Error> {
    let duration = end - start;
    let hours = duration.num_hours();

    if hours <= 6 {
        // Short range: raw rows. Trim JSON columns the chart layer never
        // reads (ports, docker_containers, cpu_cores, processes).
        let raws = sqlx::query_as::<_, MetricsRowRaw>(
            r#"
            SELECT id, host_key, display_name, is_online,
                   cpu_usage_percent, memory_usage_percent,
                   load_1min, load_5min, load_15min,
                   networks,
                   NULL AS docker_containers,
                   NULL AS ports,
                   disks,
                   NULL AS processes,
                   temperatures,
                   gpus,
                   NULL AS cpu_cores,
                   NULL AS network_interfaces,
                   docker_stats,
                   rx_bytes_per_sec,
                   tx_bytes_per_sec,
                   timestamp
            FROM metrics
            WHERE host_key = ?1
              AND timestamp >= ?2
              AND timestamp <= ?3
            ORDER BY timestamp ASC
            "#,
        )
        .bind(host_key)
        .bind(start.timestamp())
        .bind(end.timestamp())
        .fetch_all(pool)
        .await?;
        return raws.into_iter().map(MetricsRow::try_from).collect();
    }

    if hours <= 336 {
        // 6h–14d: direct read from metrics_5min rollup (populated by the
        // rollup worker). `json_object` synthesizes the `networks`
        // snapshot from the scalar rx/tx totals + bucket-averaged
        // bandwidth. NULL rate columns are preserved for buckets rolled up
        // before the 0002 migration so the frontend can still fall back to
        // differentiating cumulative counters.
        let raws = sqlx::query_as::<_, MetricsRowRaw>(
            r#"
            SELECT
                0 AS id,
                host_key,
                '' AS display_name,
                CAST(is_online AS INTEGER) AS is_online,
                cpu_usage_percent,
                memory_usage_percent,
                load_1min, load_5min, load_15min,
                json_object('total_rx_bytes', total_rx_bytes,
                            'total_tx_bytes', total_tx_bytes,
                            'rx_bytes_per_sec', avg_rx_bytes_per_sec,
                            'tx_bytes_per_sec', avg_tx_bytes_per_sec) AS networks,
                NULL AS docker_containers,
                NULL AS ports,
                disks,
                NULL AS processes,
                temperatures,
                gpus,
                NULL AS cpu_cores,
                NULL AS network_interfaces,
                docker_stats,
                NULL AS rx_bytes_per_sec,
                NULL AS tx_bytes_per_sec,
                bucket AS timestamp
            FROM metrics_5min
            WHERE host_key = ?1
              AND bucket >= ?2
              AND bucket <= ?3
            ORDER BY bucket ASC
            "#,
        )
        .bind(host_key)
        .bind(start.timestamp())
        .bind(end.timestamp())
        .fetch_all(pool)
        .await?;
        return raws.into_iter().map(MetricsRow::try_from).collect();
    }

    // >14d: re-aggregate the 5-min rollup into 15-min buckets.
    //
    // The previous shape issued **four correlated subqueries per output row**
    // (disks / temperatures / gpus / docker_stats), each a separate index
    // lookup on `metrics_5min`. At 30 days × 96 buckets/day that is 11 520
    // subquery probes per host per request — the biggest single contributor
    // to dashboard p95 latency for long ranges.
    //
    // Rewritten shape: a single CTE tags every 5-min row with its 15-min
    // bucket and a `ROW_NUMBER() OVER (PARTITION BY host_key, bucket_15m
    // ORDER BY bucket DESC)` window so the "last row in bucket" can be
    // picked in one pass with `MAX(CASE WHEN rn = 1 THEN col END)`. Scalar
    // averages stay as plain aggregates. Net effect: 1 table scan + 1
    // window + 1 GROUP BY instead of the N×4 correlated subqueries.
    let raws = sqlx::query_as::<_, MetricsRowRaw>(
        r#"
        WITH tagged AS (
            SELECT
                host_key,
                (bucket / 900) * 900 AS bucket_15m,
                bucket,
                is_online,
                cpu_usage_percent,
                memory_usage_percent,
                load_1min, load_5min, load_15min,
                total_rx_bytes, total_tx_bytes,
                avg_rx_bytes_per_sec, avg_tx_bytes_per_sec,
                disks, temperatures, gpus, docker_stats,
                ROW_NUMBER() OVER (
                    PARTITION BY host_key, (bucket / 900) * 900
                    ORDER BY bucket DESC
                ) AS rn
            FROM metrics_5min
            WHERE host_key = ?1
              AND bucket >= ?2
              AND bucket <= ?3
        )
        SELECT
            0 AS id,
            host_key,
            '' AS display_name,
            CAST(MIN(is_online) AS INTEGER) AS is_online,
            CAST(AVG(cpu_usage_percent) AS REAL) AS cpu_usage_percent,
            CAST(AVG(memory_usage_percent) AS REAL) AS memory_usage_percent,
            CAST(AVG(load_1min) AS REAL) AS load_1min,
            CAST(AVG(load_5min) AS REAL) AS load_5min,
            CAST(AVG(load_15min) AS REAL) AS load_15min,
            json_object('total_rx_bytes', MAX(total_rx_bytes),
                        'total_tx_bytes', MAX(total_tx_bytes),
                        'rx_bytes_per_sec', AVG(avg_rx_bytes_per_sec),
                        'tx_bytes_per_sec', AVG(avg_tx_bytes_per_sec)) AS networks,
            NULL AS docker_containers,
            NULL AS ports,
            MAX(CASE WHEN rn = 1 THEN disks END) AS disks,
            NULL AS processes,
            MAX(CASE WHEN rn = 1 THEN temperatures END) AS temperatures,
            MAX(CASE WHEN rn = 1 THEN gpus END) AS gpus,
            NULL AS cpu_cores,
            NULL AS network_interfaces,
            MAX(CASE WHEN rn = 1 THEN docker_stats END) AS docker_stats,
            NULL AS rx_bytes_per_sec,
            NULL AS tx_bytes_per_sec,
            bucket_15m AS timestamp
        FROM tagged
        GROUP BY host_key, bucket_15m
        ORDER BY timestamp ASC
        "#,
    )
    .bind(host_key)
    .bind(start.timestamp())
    .bind(end.timestamp())
    .fetch_all(pool)
    .await?;
    raws.into_iter().map(MetricsRow::try_from).collect()
}

/// Fetch all monitored hosts with their latest online status.
///
/// A host is online iff its most recent metric landed in the past 60 s.
/// Window the subquery to the past 5 minutes so SQLite can skip older
/// chunks entirely.
pub async fn fetch_host_summaries(pool: &DbPool) -> Result<Vec<HostSummary>, sqlx::Error> {
    sqlx::query_as::<_, HostSummary>(
        r#"
        WITH recent AS (
            SELECT host_key, is_online, timestamp,
                   ROW_NUMBER() OVER (PARTITION BY host_key
                                      ORDER BY timestamp DESC, id DESC) AS rn
            FROM metrics
            WHERE timestamp > strftime('%s','now') - 300
        )
        SELECT
            h.host_key,
            h.display_name,
            COALESCE(
                (SELECT timestamp > strftime('%s','now') - 60
                 FROM recent r WHERE r.host_key = h.host_key AND r.rn = 1),
                0
            ) AS is_online,
            (SELECT timestamp FROM recent r
             WHERE r.host_key = h.host_key AND r.rn = 1) AS last_seen
        FROM hosts h
        ORDER BY h.host_key
        "#,
    )
    .fetch_all(pool)
    .await
}

/// Fetch N-day overall uptime percentage for all hosts in a single query.
/// Returns a HashMap<host_key, uptime_pct> — used by public_status to avoid N+1 queries.
pub async fn fetch_batch_uptime_pct(
    pool: &DbPool,
    days: i32,
) -> Result<HashMap<String, f64>, sqlx::Error> {
    let rows: Vec<(String, f64)> = sqlx::query_as(
        r#"
        SELECT
            host_key,
            CASE
                WHEN SUM(sample_count) > 0
                THEN (CAST(SUM(CASE WHEN is_online = 1 THEN sample_count ELSE 0 END) AS REAL)
                      / CAST(SUM(sample_count) AS REAL)) * 100.0
                ELSE 0.0
            END AS uptime_pct
        FROM metrics_5min
        WHERE bucket >= strftime('%s','now') - ?1 * 86400
        GROUP BY host_key
        "#,
    )
    .bind(days)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Compute daily uptime percentage for a host over the given number of days.
/// Uses the metrics_5min rollup for efficient daily re-aggregation;
/// sample_count provides weighted averages for accurate uptime calculation.
pub async fn fetch_uptime(
    pool: &DbPool,
    host_key: &str,
    days: i32,
) -> Result<UptimeSummary, sqlx::Error> {
    let daily = sqlx::query_as::<_, UptimePoint>(
        r#"
        SELECT
            (bucket / 86400) * 86400 AS day,
            CAST(SUM(sample_count) AS INTEGER) AS total_count,
            CAST(SUM(CASE WHEN is_online = 1 THEN sample_count ELSE 0 END) AS INTEGER)
                AS online_count,
            CASE
                WHEN SUM(sample_count) > 0
                THEN (CAST(SUM(CASE WHEN is_online = 1 THEN sample_count ELSE 0 END) AS REAL)
                      / CAST(SUM(sample_count) AS REAL)) * 100.0
                ELSE 0.0
            END AS uptime_pct
        FROM metrics_5min
        WHERE host_key = ?1
          AND bucket >= strftime('%s','now') - ?2 * 86400
        GROUP BY (bucket / 86400) * 86400
        ORDER BY day DESC
        "#,
    )
    .bind(host_key)
    .bind(days)
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

#[cfg(test)]
mod sqlite_tests {
    use super::*;
    use crate::models::agent_metrics::{
        AgentMetrics, DockerContainer, LoadAverage, NetworkTotal, PortStatus, SystemMetrics,
    };
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
        // A hosts row is required for fetch_host_summaries; seed one
        // per test via the hosts_repo-compatible raw insert. Test pools
        // run with foreign_keys=false so the FK back to hosts from
        // `metrics` is non-issue.
        sqlx::query(
            "INSERT INTO hosts (host_key, display_name) VALUES ('h1:9101', 'box-1'), ('h2:9101', 'box-2')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    fn synthetic_metrics() -> AgentMetrics {
        AgentMetrics {
            hostname: "box-1".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            is_online: true,
            system: SystemMetrics {
                cpu_usage_percent: 42.5,
                memory_total_mb: 8192,
                memory_used_mb: 4096,
                memory_usage_percent: 50.0,
                disks: vec![],
                processes: vec![],
                temperatures: vec![],
                gpus: vec![],
            },
            network: NetworkTotal {
                total_rx_bytes: 1_000_000,
                total_tx_bytes: 500_000,
                ..Default::default()
            },
            network_interfaces: vec![],
            cpu_cores: vec![],
            load_average: LoadAverage {
                one_min: 1.2,
                five_min: 1.5,
                fifteen_min: 2.0,
            },
            docker_containers: vec![] as Vec<DockerContainer>,
            docker_stats: vec![],
            ports: vec![] as Vec<PortStatus>,
            agent_version: "0.3.5".into(),
        }
    }

    #[tokio::test]
    async fn insert_batch_then_fetch_recent() {
        let pool = fresh_pool().await;
        let m = synthetic_metrics();

        insert_metrics_batch(&pool, &[("h1:9101", &m)])
            .await
            .unwrap();
        insert_metrics_batch(&pool, &[("h1:9101", &m)])
            .await
            .unwrap();
        insert_metrics_batch(&pool, &[("h2:9101", &m)])
            .await
            .unwrap();

        let recent = fetch_recent_metrics(&pool, "h1:9101").await.unwrap();
        assert_eq!(recent.len(), 2);
        for row in &recent {
            assert!(row.is_online);
            assert!((row.cpu_usage_percent - 42.5).abs() < 0.01);
            // networks JSON round-tripped from TEXT.
            let net = row.networks.as_ref().unwrap();
            assert_eq!(net["total_rx_bytes"], 1_000_000i64);
        }
    }

    #[tokio::test]
    async fn offline_batch_sets_is_online_false() {
        let pool = fresh_pool().await;
        insert_offline_metrics_batch(&pool, &[("h1:9101", "box-1")])
            .await
            .unwrap();
        let recent = fetch_recent_metrics(&pool, "h1:9101").await.unwrap();
        assert_eq!(recent.len(), 1);
        assert!(!recent[0].is_online);
        assert!(recent[0].networks.is_none());
    }

    #[tokio::test]
    async fn host_summaries_uses_latest_per_host() {
        let pool = fresh_pool().await;
        let m = synthetic_metrics();

        insert_metrics_batch(&pool, &[("h1:9101", &m), ("h2:9101", &m)])
            .await
            .unwrap();

        let summaries = fetch_host_summaries(&pool).await.unwrap();
        assert_eq!(summaries.len(), 2);
        for s in &summaries {
            assert!(s.is_online, "host {} should be online", s.host_key);
            assert!(s.last_seen.is_some());
        }
    }

    #[tokio::test]
    async fn metrics_range_raw_tier_returns_rows_within_window() {
        let pool = fresh_pool().await;
        let m = synthetic_metrics();
        insert_metrics_batch(&pool, &[("h1:9101", &m)])
            .await
            .unwrap();

        let now = Utc::now();
        let rows = fetch_metrics_range(
            &pool,
            "h1:9101",
            now - chrono::Duration::hours(1),
            now + chrono::Duration::hours(1),
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        // Trimmed JSON columns are NULL per the "raw tier minus heavy
        // columns" projection.
        assert!(rows[0].docker_containers.is_none());
        assert!(rows[0].ports.is_none());
        // Full-detail columns survive.
        assert!(rows[0].networks.is_some());
    }

    #[tokio::test]
    async fn uptime_returns_empty_until_rollup_worker_runs() {
        // Without the rollup worker running, uptime queries see an empty
        // aggregate — this test pins that expectation so nobody
        // accidentally turns the dashboard silent.
        let pool = fresh_pool().await;
        let batch = fetch_batch_uptime_pct(&pool, 7).await.unwrap();
        assert!(batch.is_empty());

        let summary = fetch_uptime(&pool, "h1:9101", 7).await.unwrap();
        assert_eq!(summary.overall_pct, 0.0);
        assert!(summary.daily.is_empty());
    }
}
