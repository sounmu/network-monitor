use std::collections::HashMap;

use crate::db::DbPool;
use chrono::{DateTime, Utc};
use chrono_tz::Asia::Seoul;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::agent_metrics::{AgentMetrics, DiskInfo, DockerContainerStats, TemperatureInfo};

pub const CHART_RAW_BOUNDARY_SECS: i64 = 60 * 60;

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

#[derive(Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct ChartNetwork {
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
    pub rx_bytes_per_sec: f64,
    pub tx_bytes_per_sec: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ChartDiskInfo {
    pub name: String,
    pub mount_point: String,
    pub usage_percent: f32,
    pub read_bytes_per_sec: f64,
    pub write_bytes_per_sec: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ChartDockerStats {
    pub container_name: String,
    pub cpu_percent: f32,
    pub memory_usage_mb: u64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ChartMetricsRow {
    pub id: i64,
    pub host_key: String,
    pub display_name: String,
    pub is_online: bool,
    pub cpu_usage_percent: f32,
    pub memory_usage_percent: f32,
    pub load_1min: f32,
    pub load_5min: f32,
    pub load_15min: f32,
    pub networks: Option<ChartNetwork>,
    pub disks: Vec<ChartDiskInfo>,
    pub temperatures: Vec<TemperatureInfo>,
    pub docker_stats: Vec<ChartDockerStats>,
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
    /// Scalar totals sourced from `metrics_5min` in the rollup/wide
    /// branches. Populated as `Some(_)` only when `networks` is `None`
    /// (i.e. the SQL did not pre-build the JSON via `json_object`); the
    /// `TryFrom` synthesizes the networks object in Rust.
    total_rx_bytes: Option<i64>,
    total_tx_bytes: Option<i64>,
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

        // Synthesize the `networks` JSON object on the Rust side when the
        // rollup / wide-aggregation branches supply scalar totals instead.
        // Skipping SQLite's `json_object(...)` per-row call saves the
        // planner's string-building pass inside the query — measurable on
        // 30-day windows at >14d.
        //
        // Gate on `total_*_bytes` specifically — only the rollup branches
        // populate those, never the raw branches. This preserves the
        // contract that offline rows from `insert_offline_metrics_batch`
        // (which bind `rx_bytes_per_sec = 0.0` but leave `networks` NULL)
        // surface as `networks = None`, not a misleading
        // `{"rx_bytes_per_sec": 0, "tx_bytes_per_sec": 0}` object.
        if networks.is_none() && (raw.total_rx_bytes.is_some() || raw.total_tx_bytes.is_some()) {
            let mut map = serde_json::Map::with_capacity(4);
            if let Some(v) = raw.total_rx_bytes {
                map.insert("total_rx_bytes".into(), Value::from(v));
            }
            if let Some(v) = raw.total_tx_bytes {
                map.insert("total_tx_bytes".into(), Value::from(v));
            }
            if let Some(v) = raw.rx_bytes_per_sec {
                map.insert("rx_bytes_per_sec".into(), Value::from(v));
            }
            if let Some(v) = raw.tx_bytes_per_sec {
                map.insert("tx_bytes_per_sec".into(), Value::from(v));
            }
            networks = Some(Value::Object(map));
        } else if let Some(Value::Object(ref mut map)) = networks {
            // Raw-branch path: `networks` was read as JSON text from
            // `metrics.networks`; merge the scalar rate columns so the
            // shape matches the rollup branches above.
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

#[derive(sqlx::FromRow)]
struct ChartMetricsRowRaw {
    id: i64,
    host_key: String,
    display_name: String,
    is_online: Option<bool>,
    cpu_usage_percent: f32,
    memory_usage_percent: f32,
    load_1min: f32,
    load_5min: f32,
    load_15min: f32,
    total_rx_bytes: Option<i64>,
    total_tx_bytes: Option<i64>,
    rx_bytes_per_sec: Option<f64>,
    tx_bytes_per_sec: Option<f64>,
    disks: Option<String>,
    temperatures: Option<String>,
    docker_stats: Option<String>,
    timestamp: DateTime<Utc>,
}

fn parse_json_vec<T>(s: Option<String>) -> Result<Vec<T>, sqlx::Error>
where
    T: for<'de> Deserialize<'de>,
{
    match s {
        Some(text) => serde_json::from_str(&text).map_err(|e| sqlx::Error::Decode(Box::new(e))),
        None => Ok(Vec::new()),
    }
}

impl TryFrom<ChartMetricsRowRaw> for ChartMetricsRow {
    type Error = sqlx::Error;

    fn try_from(raw: ChartMetricsRowRaw) -> Result<Self, Self::Error> {
        let networks = match (raw.total_rx_bytes, raw.total_tx_bytes) {
            (Some(rx_total), Some(tx_total)) => Some(ChartNetwork {
                total_rx_bytes: rx_total.max(0) as u64,
                total_tx_bytes: tx_total.max(0) as u64,
                rx_bytes_per_sec: raw.rx_bytes_per_sec.unwrap_or(0.0),
                tx_bytes_per_sec: raw.tx_bytes_per_sec.unwrap_or(0.0),
            }),
            _ => None,
        };

        let disks: Vec<DiskInfo> = parse_json_vec(raw.disks)?;
        let docker_stats: Vec<DockerContainerStats> = parse_json_vec(raw.docker_stats)?;

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
            disks: disks
                .into_iter()
                .map(|d| ChartDiskInfo {
                    name: d.name,
                    mount_point: d.mount_point,
                    usage_percent: d.usage_percent,
                    read_bytes_per_sec: d.read_bytes_per_sec,
                    write_bytes_per_sec: d.write_bytes_per_sec,
                })
                .collect(),
            temperatures: parse_json_vec(raw.temperatures)?,
            docker_stats: docker_stats
                .into_iter()
                .map(|s| ChartDockerStats {
                    container_name: s.container_name,
                    cpu_percent: s.cpu_percent,
                    memory_usage_mb: s.memory_usage_mb,
                })
                .collect(),
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
         rx_bytes_per_sec, tx_bytes_per_sec, \
         total_rx_bytes, total_tx_bytes) ",
    );

    qb.push_values(batch, |mut b, (host_key, metrics)| {
        // SQLite stores INTEGER as i64; the agent-reported counters are u64.
        // Saturating-cast is the right behaviour: a host that has actually
        // moved >9 EB on a counter is theoretical-only, but i64 overflow on
        // the bind would error out the whole batch.
        let total_rx = i64::try_from(metrics.network.total_rx_bytes).unwrap_or(i64::MAX);
        let total_tx = i64::try_from(metrics.network.total_tx_bytes).unwrap_or(i64::MAX);
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
            .push_bind(metrics.network.tx_bytes_per_sec)
            .push_bind(total_rx)
            .push_bind(total_tx);
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
///
/// Trimmed projection — `processes` / `cpu_cores` / `network_interfaces` /
/// `ports` / `docker_containers` are returned as NULL because:
///   1. Only the latest row drives the dashboard's headline summary cards
///      (cpu / memory / disk / network rate). Older points are catch-up
///      data, not history — for history the UI calls the chart endpoint.
///   2. Live updates flow through SSE, so any heavy-JSON snapshot is
///      already streamed in within seconds of the page mounting.
///   3. Each row's full snapshot can be tens of KB on hosts with many
///      processes / containers; 50 rows × that × N hosts cold-loads was a
///      noticeable dashboard p95 hit before this trim.
///
/// Mirrors the ≤6h branch of `fetch_metrics_range` to keep the row shape
/// consistent across both code paths that feed the same `MetricsRow` type.
pub async fn fetch_recent_metrics(
    pool: &DbPool,
    host_key: &str,
) -> Result<Vec<MetricsRow>, sqlx::Error> {
    // `total_rx_bytes` / `total_tx_bytes` are now read directly (migration
    // 0005). The TryFrom path uses these to synthesize `networks` for
    // headline-card display when the heavy `networks` JSON has been read.
    // Keep `networks` itself in the projection because it carries
    // per-interface detail the host card may render (top device, interface
    // counters); only the totals are needed for the chart-style summary.
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
               rx_bytes_per_sec, tx_bytes_per_sec,
               total_rx_bytes,
               total_tx_bytes,
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
        // `total_rx_bytes` / `total_tx_bytes` come from migration 0005's
        // scalar columns instead of `json_extract(networks, …)` per row.
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
                   total_rx_bytes,
                   total_tx_bytes,
                   timestamp
            FROM metrics
            WHERE host_key = ?1
              AND timestamp >= ?2
              AND timestamp <= ?3
            ORDER BY timestamp ASC, id ASC
            "#,
        )
        .bind(host_key)
        .bind(start.timestamp())
        .bind(end.timestamp())
        .fetch_all(pool)
        .await?;
        // `id ASC` is the tie-breaker for rows inserted within the same
        // second — without it, `ORDER BY timestamp ASC` alone leaves the
        // relative ordering of same-second rows unspecified, which showed
        // up as line-chart jitter when a flapping host emitted two scrapes
        // in the same wall-clock second.
        return raws.into_iter().map(MetricsRow::try_from).collect();
    }

    if hours <= 336 {
        // 6h–14d: direct read from metrics_5min rollup (populated by the
        // rollup worker). Return the scalar bandwidth totals directly; the
        // `networks` JSON object is synthesized Rust-side in
        // `TryFrom<MetricsRowRaw>`. Skipping SQLite's per-row
        // `json_object(...)` measurably lowers query CPU over long ranges.
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
                NULL AS networks,
                NULL AS docker_containers,
                NULL AS ports,
                disks,
                NULL AS processes,
                temperatures,
                gpus,
                NULL AS cpu_cores,
                NULL AS network_interfaces,
                docker_stats,
                avg_rx_bytes_per_sec AS rx_bytes_per_sec,
                avg_tx_bytes_per_sec AS tx_bytes_per_sec,
                total_rx_bytes,
                total_tx_bytes,
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
            NULL AS networks,
            NULL AS docker_containers,
            NULL AS ports,
            MAX(CASE WHEN rn = 1 THEN disks END) AS disks,
            NULL AS processes,
            MAX(CASE WHEN rn = 1 THEN temperatures END) AS temperatures,
            MAX(CASE WHEN rn = 1 THEN gpus END) AS gpus,
            NULL AS cpu_cores,
            NULL AS network_interfaces,
            MAX(CASE WHEN rn = 1 THEN docker_stats END) AS docker_stats,
            CAST(AVG(avg_rx_bytes_per_sec) AS REAL) AS rx_bytes_per_sec,
            CAST(AVG(avg_tx_bytes_per_sec) AS REAL) AS tx_bytes_per_sec,
            MAX(total_rx_bytes) AS total_rx_bytes,
            MAX(total_tx_bytes) AS total_tx_bytes,
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

/// Fetch chart-ready metrics for a host within a time range.
///
/// This is intentionally narrower than `fetch_metrics_range`: it keeps the
/// scalar time series and the small chart-only projections for disk,
/// temperature, and Docker graphs, while omitting large snapshot fields that
/// belong to status/detail panels.
pub async fn fetch_chart_metrics_range(
    pool: &DbPool,
    host_key: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<ChartMetricsRow>, sqlx::Error> {
    let duration = end - start;
    let seconds = duration.num_seconds();

    if seconds <= CHART_RAW_BOUNDARY_SECS {
        // Migration 0005 projects `total_rx_bytes` / `total_tx_bytes` into
        // their own INTEGER columns. Read them directly. Rows inserted
        // before 0005 ran will have NULL for these columns; fall back to
        // the JSON blob in that case so historical queries remain correct
        // until rolling deploys finish and old raw rows age out (3 day
        // retention).
        let raws = sqlx::query_as::<_, ChartMetricsRowRaw>(
            r#"
            SELECT id, host_key, display_name, is_online,
                   cpu_usage_percent, memory_usage_percent,
                   load_1min, load_5min, load_15min,
                   COALESCE(
                       total_rx_bytes,
                       CAST(json_extract(networks, '$.total_rx_bytes') AS INTEGER)
                   ) AS total_rx_bytes,
                   COALESCE(
                       total_tx_bytes,
                       CAST(json_extract(networks, '$.total_tx_bytes') AS INTEGER)
                   ) AS total_tx_bytes,
                   rx_bytes_per_sec,
                   tx_bytes_per_sec,
                   disks,
                   temperatures,
                   docker_stats,
                   timestamp
            FROM metrics
            WHERE host_key = ?1
              AND timestamp >= ?2
              AND timestamp <= ?3
            ORDER BY timestamp ASC, id ASC
            "#,
        )
        .bind(host_key)
        .bind(start.timestamp())
        .bind(end.timestamp())
        .fetch_all(pool)
        .await?;
        return raws.into_iter().map(ChartMetricsRow::try_from).collect();
    }

    if seconds <= 14 * 24 * 3600 {
        let raws = sqlx::query_as::<_, ChartMetricsRowRaw>(
            r#"
            SELECT
                0 AS id,
                host_key,
                '' AS display_name,
                CAST(is_online AS INTEGER) AS is_online,
                cpu_usage_percent,
                memory_usage_percent,
                load_1min, load_5min, load_15min,
                total_rx_bytes,
                total_tx_bytes,
                avg_rx_bytes_per_sec AS rx_bytes_per_sec,
                avg_tx_bytes_per_sec AS tx_bytes_per_sec,
                disks,
                temperatures,
                docker_stats,
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
        return raws.into_iter().map(ChartMetricsRow::try_from).collect();
    }

    let raws = sqlx::query_as::<_, ChartMetricsRowRaw>(
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
                disks, temperatures, docker_stats,
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
            MAX(total_rx_bytes) AS total_rx_bytes,
            MAX(total_tx_bytes) AS total_tx_bytes,
            CAST(AVG(avg_rx_bytes_per_sec) AS REAL) AS rx_bytes_per_sec,
            CAST(AVG(avg_tx_bytes_per_sec) AS REAL) AS tx_bytes_per_sec,
            MAX(CASE WHEN rn = 1 THEN disks END) AS disks,
            MAX(CASE WHEN rn = 1 THEN temperatures END) AS temperatures,
            MAX(CASE WHEN rn = 1 THEN docker_stats END) AS docker_stats,
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
    raws.into_iter().map(ChartMetricsRow::try_from).collect()
}

/// Fetch all monitored hosts with their latest online status.
///
/// A host is online iff its most recent metric landed in the past 60 s.
/// Window the subquery to the past 5 minutes so SQLite can skip older
/// chunks entirely.
///
/// Shape note: the previous implementation issued **two correlated scalar
/// subqueries** — one for `is_online`, another for `last_seen` — meaning
/// SQLite re-scanned the `recent` CTE twice per host row. This rewrite
/// picks the latest-per-host row once via `LEFT JOIN` on the `rn = 1`
/// filter of the window function, reusing the same scan for both output
/// columns. Hosts with no recent metric land at `is_online = 0` /
/// `last_seen = NULL` via the LEFT side of the join.
pub async fn fetch_host_summaries(pool: &DbPool) -> Result<Vec<HostSummary>, sqlx::Error> {
    sqlx::query_as::<_, HostSummary>(
        r#"
        SELECT
            h.host_key,
            h.display_name,
            COALESCE(r.timestamp > strftime('%s','now') - 60, 0) AS is_online,
            r.timestamp AS last_seen
        FROM hosts h
        LEFT JOIN (
            SELECT host_key, is_online, timestamp,
                   ROW_NUMBER() OVER (PARTITION BY host_key
                                      ORDER BY timestamp DESC, id DESC) AS rn
            FROM metrics
            WHERE timestamp > strftime('%s','now') - 300
        ) r ON r.host_key = h.host_key AND r.rn = 1
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
        AgentMetrics, DiskInfo, DockerContainer, DockerContainerStats, LoadAverage, NetworkTotal,
        PortStatus, SystemMetrics, TemperatureInfo,
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
    async fn host_summaries_returns_offline_for_hosts_without_recent_metrics() {
        // Regression pin for the LEFT JOIN rewrite: a host that exists in
        // `hosts` but has no row in `metrics` within the 5-minute window
        // must still appear in the summary with `is_online = false` and
        // `last_seen = None`. The previous correlated-subquery shape
        // happened to return the row via `COALESCE(..., 0)`; the LEFT JOIN
        // shape must match that contract.
        let pool = fresh_pool().await;
        // Insert a row for h1 only — h2 has no metrics.
        insert_metrics_batch(&pool, &[("h1:9101", &synthetic_metrics())])
            .await
            .unwrap();

        let summaries = fetch_host_summaries(&pool).await.unwrap();
        assert_eq!(summaries.len(), 2, "both hosts must be listed");
        let h2 = summaries
            .iter()
            .find(|s| s.host_key == "h2:9101")
            .expect("h2 present");
        assert!(!h2.is_online, "h2 has no metrics → offline");
        assert!(h2.last_seen.is_none());
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
            now - chrono::Duration::minutes(30),
            now + chrono::Duration::minutes(30),
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
    async fn chart_metrics_range_returns_lightweight_projection() {
        let pool = fresh_pool().await;
        let mut m = synthetic_metrics();
        m.system.disks = vec![DiskInfo {
            name: "disk0".into(),
            mount_point: "/".into(),
            total_gb: 100.0,
            available_gb: 40.0,
            usage_percent: 60.0,
            read_bytes_per_sec: 123.0,
            write_bytes_per_sec: 456.0,
        }];
        m.system.temperatures = vec![TemperatureInfo {
            label: "CPU".into(),
            temperature_c: 55.0,
        }];
        m.docker_stats = vec![DockerContainerStats {
            container_name: "app".into(),
            cpu_percent: 7.5,
            memory_usage_mb: 128,
            memory_limit_mb: 1024,
            net_rx_bytes: 99,
            net_tx_bytes: 100,
        }];
        m.network.rx_bytes_per_sec = 11.0;
        m.network.tx_bytes_per_sec = 22.0;

        insert_metrics_batch(&pool, &[("h1:9101", &m)])
            .await
            .unwrap();

        let now = Utc::now();
        let rows = fetch_chart_metrics_range(
            &pool,
            "h1:9101",
            now - chrono::Duration::minutes(30),
            now + chrono::Duration::minutes(30),
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.disks[0].mount_point, "/");
        assert!((row.disks[0].usage_percent - 60.0).abs() < 0.01);
        assert_eq!(row.temperatures[0].label, "CPU");
        assert_eq!(row.docker_stats[0].container_name, "app");
        assert!((row.docker_stats[0].cpu_percent - 7.5).abs() < 0.01);
        assert_eq!(row.networks.as_ref().unwrap().total_rx_bytes, 1_000_000);
        assert_eq!(row.networks.as_ref().unwrap().rx_bytes_per_sec, 11.0);
    }

    #[tokio::test]
    async fn chart_metrics_range_uses_rollup_after_one_hour() {
        let pool = fresh_pool().await;
        let now = Utc::now();
        let bucket = (now.timestamp() / 300) * 300;

        sqlx::query(
            r#"
            INSERT INTO metrics_5min (
                host_key, bucket, cpu_usage_percent, memory_usage_percent,
                load_1min, load_5min, load_15min, is_online, sample_count,
                total_rx_bytes, total_tx_bytes, disks, temperatures, docker_stats,
                avg_rx_bytes_per_sec, avg_tx_bytes_per_sec
            )
            VALUES (
                'h1:9101', ?1, 12.5, 34.5,
                0.1, 0.2, 0.3, 1, 30,
                12345, 67890,
                '[{"name":"disk0","mount_point":"/","total_gb":100,"available_gb":50,"usage_percent":50,"read_bytes_per_sec":1,"write_bytes_per_sec":2}]',
                '[{"label":"CPU","temperature_c":44}]',
                '[{"container_name":"app","cpu_percent":3.5,"memory_usage_mb":64,"memory_limit_mb":1024,"net_rx_bytes":1,"net_tx_bytes":2}]',
                111.0, 222.0
            )
            "#,
        )
        .bind(bucket)
        .execute(&pool)
        .await
        .unwrap();

        let rows = fetch_chart_metrics_range(
            &pool,
            "h1:9101",
            now - chrono::Duration::hours(2),
            now + chrono::Duration::hours(2),
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, 0, "rollup rows are synthetic chart rows");
        assert!((rows[0].cpu_usage_percent - 12.5).abs() < 0.01);
        assert_eq!(rows[0].networks.as_ref().unwrap().rx_bytes_per_sec, 111.0);
        assert_eq!(rows[0].disks[0].mount_point, "/");
    }

    #[tokio::test]
    async fn chart_metrics_range_wide_re_aggregates_into_15min_buckets() {
        // > 14 d windows route through the window-function CTE and pick the
        // last-in-bucket JSON snapshot via `MAX(CASE WHEN rn = 1 ...)`. This
        // path is the easiest to break (CTE column shape, alias drift,
        // SQLite window-function support) and the cheapest to regression-pin.
        let pool = fresh_pool().await;

        // Pin "now" to a 15-min boundary so both inserted rows fall inside
        // the same 15-min re-aggregation bucket and the `rn = 1` selector
        // has a deterministic winner.
        let bucket_anchor = (Utc::now().timestamp() / 900) * 900;
        let bucket_first = bucket_anchor; // older 5-min bucket
        let bucket_last = bucket_anchor + 300; // newer 5-min bucket within the same 15-min window

        // Older 5-min bucket: lower CPU, alternative disk JSON.
        sqlx::query(
            r#"
            INSERT INTO metrics_5min (
                host_key, bucket, cpu_usage_percent, memory_usage_percent,
                load_1min, load_5min, load_15min, is_online, sample_count,
                total_rx_bytes, total_tx_bytes, disks, temperatures, docker_stats,
                avg_rx_bytes_per_sec, avg_tx_bytes_per_sec
            )
            VALUES (
                'h1:9101', ?1, 10.0, 20.0,
                0.1, 0.2, 0.3, 1, 30,
                100, 200,
                '[{"name":"older","mount_point":"/older","total_gb":50,"available_gb":25,"usage_percent":50,"read_bytes_per_sec":1,"write_bytes_per_sec":2}]',
                '[{"label":"OLD","temperature_c":30}]',
                '[{"container_name":"old","cpu_percent":1.0,"memory_usage_mb":10,"memory_limit_mb":256,"net_rx_bytes":1,"net_tx_bytes":2}]',
                100.0, 200.0
            )
            "#,
        )
        .bind(bucket_first)
        .execute(&pool)
        .await
        .unwrap();

        // Newer 5-min bucket inside the same 15-min window: higher CPU,
        // distinct disk/temperature/docker JSON.  The wide branch must
        // surface *this* row's JSON snapshots (rn = 1).
        sqlx::query(
            r#"
            INSERT INTO metrics_5min (
                host_key, bucket, cpu_usage_percent, memory_usage_percent,
                load_1min, load_5min, load_15min, is_online, sample_count,
                total_rx_bytes, total_tx_bytes, disks, temperatures, docker_stats,
                avg_rx_bytes_per_sec, avg_tx_bytes_per_sec
            )
            VALUES (
                'h1:9101', ?1, 30.0, 60.0,
                0.4, 0.5, 0.6, 1, 30,
                500, 700,
                '[{"name":"newer","mount_point":"/newer","total_gb":100,"available_gb":40,"usage_percent":60,"read_bytes_per_sec":3,"write_bytes_per_sec":4}]',
                '[{"label":"NEW","temperature_c":55}]',
                '[{"container_name":"new","cpu_percent":7.5,"memory_usage_mb":128,"memory_limit_mb":1024,"net_rx_bytes":9,"net_tx_bytes":10}]',
                300.0, 400.0
            )
            "#,
        )
        .bind(bucket_last)
        .execute(&pool)
        .await
        .unwrap();

        // 30-day window forces the wide branch (>14 d).
        let now = Utc::now();
        let rows = fetch_chart_metrics_range(
            &pool,
            "h1:9101",
            now - chrono::Duration::days(30),
            now + chrono::Duration::days(1),
        )
        .await
        .unwrap();

        // Both 5-min rows fall in the same 15-min bucket → exactly one
        // output row.
        assert_eq!(
            rows.len(),
            1,
            "two 5-min rows collapse into one 15-min bucket"
        );
        let row = &rows[0];

        // Scalars are AVG over both rows.
        assert!(
            (row.cpu_usage_percent - 20.0).abs() < 0.01,
            "CPU should be the average of 10 and 30, got {}",
            row.cpu_usage_percent
        );
        assert!(
            (row.memory_usage_percent - 40.0).abs() < 0.01,
            "memory should be the average of 20 and 60, got {}",
            row.memory_usage_percent
        );

        // Cumulative counters are MAX (latest absolute value).
        let net = row.networks.as_ref().expect("network rates synthesized");
        assert_eq!(net.total_rx_bytes, 500);
        assert_eq!(net.total_tx_bytes, 700);

        // JSON snapshots come from the rn=1 row (the *newer* bucket).
        assert_eq!(row.disks.len(), 1);
        assert_eq!(row.disks[0].mount_point, "/newer");
        assert_eq!(row.temperatures[0].label, "NEW");
        assert_eq!(row.docker_stats[0].container_name, "new");
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
