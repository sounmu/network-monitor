use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::db::DbPool;
use crate::models::app_state::{AlertConfig, MetricAlertRule};

/// Alert metric type — compile-time exhaustive matching.
///
/// Every variant is evaluated by the scraper each cycle. Load/Network/Temperature/Gpu
/// rules ship disabled by default (see `AlertConfig::default()`) so existing
/// deployments keep the historical CPU/Memory/Disk behaviour until operators
/// opt in from the /alerts page. Sub-scoped rules (per sensor / interface /
/// GPU) are persisted via `sub_key` but not yet routed into the runtime rule
/// map — rules with a non-null `sub_key` are stored and surfaced in the API
/// only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum MetricType {
    Cpu,
    Memory,
    Disk,
    Load,
    Network,
    Temperature,
    Gpu,
}

impl std::fmt::Display for MetricType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetricType::Cpu => write!(f, "cpu"),
            MetricType::Memory => write!(f, "memory"),
            MetricType::Disk => write!(f, "disk"),
            MetricType::Load => write!(f, "load"),
            MetricType::Network => write!(f, "network"),
            MetricType::Temperature => write!(f, "temperature"),
            MetricType::Gpu => write!(f, "gpu"),
        }
    }
}

/// Row struct for the `alert_configs` table.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AlertConfigRow {
    pub id: i32,
    pub host_key: Option<String>,
    pub metric_type: MetricType,
    pub enabled: bool,
    pub threshold: f64,
    pub sustained_secs: i32,
    pub cooldown_secs: i32,
    pub updated_at: DateTime<Utc>,
    /// Optional scope within a metric type — e.g. a specific sensor label,
    /// interface name, or GPU index. `NULL` means the rule applies to the
    /// whole metric category on the host.
    #[serde(default)]
    pub sub_key: Option<String>,
}

/// Resolve the effective AlertConfig for a host (host override → global fallback).
pub fn resolve_alert_config(
    host_key: &str,
    load_threshold: f64,
    alert_map: &HashMap<String, AlertConfig>,
) -> AlertConfig {
    let base = alert_map
        .get(host_key)
        .or_else(|| alert_map.get("__global__"))
        .cloned()
        .unwrap_or_default();

    AlertConfig {
        load_threshold,
        ..base
    }
}

/// Request body for upserting an alert config entry.
#[derive(Debug, Deserialize)]
pub struct UpsertAlertRequest {
    pub metric_type: MetricType,
    pub enabled: bool,
    pub threshold: f64,
    pub sustained_secs: i32,
    pub cooldown_secs: i32,
    /// Optional scope within a metric type (sensor / interface / device).
    #[serde(default)]
    pub sub_key: Option<String>,
}

fn row_to_rule(row: &AlertConfigRow) -> MetricAlertRule {
    MetricAlertRule {
        enabled: row.enabled,
        threshold: row.threshold,
        sustained_secs: row.sustained_secs as u64,
        cooldown_secs: row.cooldown_secs as u64,
    }
}

// NULLS NOT DISTINCT emulation: the migration installs a UNIQUE INDEX
// on `(coalesce(host_key, ''), metric_type, coalesce(sub_key, ''))`.
// `ON CONFLICT` targets the same expression list so globals (NULL
// host_key) and sub-scope (NULL sub_key) collapse to single rows.
// `AlertConfigRow` has no JSON columns, so direct `sqlx::FromRow`
// works without the Raw adapter.

const ALERT_CONFIG_COLUMNS: &str = "id, host_key, metric_type, enabled, threshold, sustained_secs, cooldown_secs, \
     updated_at, sub_key";

pub async fn get_global_configs(pool: &DbPool) -> Result<Vec<AlertConfigRow>, sqlx::Error> {
    let sql = format!(
        "SELECT {ALERT_CONFIG_COLUMNS} FROM alert_configs \
         WHERE host_key IS NULL ORDER BY metric_type, sub_key"
    );
    sqlx::query_as::<_, AlertConfigRow>(&sql)
        .fetch_all(pool)
        .await
}

pub async fn get_host_configs(
    pool: &DbPool,
    host_key: &str,
) -> Result<Vec<AlertConfigRow>, sqlx::Error> {
    let sql = format!(
        "SELECT {ALERT_CONFIG_COLUMNS} FROM alert_configs \
         WHERE host_key = ?1 ORDER BY metric_type, sub_key"
    );
    sqlx::query_as::<_, AlertConfigRow>(&sql)
        .bind(host_key)
        .fetch_all(pool)
        .await
}

/// Load all alert configs and build a per-host AlertConfig map.
///
/// All seven metric variants feed `MetricAlertRule`s in the returned
/// `AlertConfig`. For each (host, metric) we pick the host-scoped override
/// when present, else fall back to the global default (sub_key NULL) row,
/// else the hard-coded `AlertConfig::default()` rule. Sub-scoped rows
/// (sub_key IS NOT NULL) are intentionally ignored here — scraper
/// evaluation currently applies a single threshold per metric category.
pub async fn load_all_as_map(pool: &DbPool) -> Result<HashMap<String, AlertConfig>, sqlx::Error> {
    let sql = format!(
        "SELECT {ALERT_CONFIG_COLUMNS} FROM alert_configs \
         ORDER BY host_key NULLS FIRST, metric_type"
    );
    let rows = sqlx::query_as::<_, AlertConfigRow>(&sql)
        .fetch_all(pool)
        .await?;

    let default_cfg = AlertConfig::default();
    let mut global_cpu = default_cfg.cpu;
    let mut global_mem = default_cfg.memory;
    let mut global_disk = default_cfg.disk;
    let mut global_load = default_cfg.load;
    let mut global_network = default_cfg.network;
    let mut global_temperature = default_cfg.temperature;
    let mut global_gpu = default_cfg.gpu;

    for row in &rows {
        if row.host_key.is_none() && row.sub_key.is_none() {
            match row.metric_type {
                MetricType::Cpu => global_cpu = row_to_rule(row),
                MetricType::Memory => global_mem = row_to_rule(row),
                MetricType::Disk => global_disk = row_to_rule(row),
                MetricType::Load => global_load = row_to_rule(row),
                MetricType::Network => global_network = row_to_rule(row),
                MetricType::Temperature => global_temperature = row_to_rule(row),
                MetricType::Gpu => global_gpu = row_to_rule(row),
            }
        }
    }

    let mut map = HashMap::new();
    let mut host_overrides: HashMap<(&str, MetricType), MetricAlertRule> = HashMap::new();

    // Pass 1 — collect overrides only. Previously a parallel
    // `HashSet<&str>` recorded which hosts appeared, then pass 2
    // iterated it and called `.to_string()` on each entry → two
    // allocations per unique host (the hash entry + the final String
    // key). Drop the HashSet and let `map` itself act as the dedupe
    // structure via `contains_key`.
    for row in &rows {
        if let Some(ref hk) = row.host_key
            && row.sub_key.is_none()
        {
            host_overrides.insert((hk.as_str(), row.metric_type), row_to_rule(row));
        }
    }

    // Pass 2 — materialize one `AlertConfig` per distinct `host_key`.
    // `contains_key(&str)` avoids allocating until we know the entry is
    // new; the allocation that remains (`hk.clone()`) is the single
    // unavoidable `String` that becomes the map key.
    for row in &rows {
        if let Some(ref hk) = row.host_key
            && !map.contains_key(hk.as_str())
        {
            let hk_ref = hk.as_str();
            let pick = |metric: MetricType, fallback: MetricAlertRule| -> MetricAlertRule {
                host_overrides
                    .get(&(hk_ref, metric))
                    .copied()
                    .unwrap_or(fallback)
            };
            map.insert(
                hk.clone(),
                AlertConfig {
                    cpu: pick(MetricType::Cpu, global_cpu),
                    memory: pick(MetricType::Memory, global_mem),
                    disk: pick(MetricType::Disk, global_disk),
                    load: pick(MetricType::Load, global_load),
                    network: pick(MetricType::Network, global_network),
                    temperature: pick(MetricType::Temperature, global_temperature),
                    gpu: pick(MetricType::Gpu, global_gpu),
                    load_threshold: 4.0,
                    load_cooldown_secs: 60,
                },
            );
        }
    }

    map.insert(
        "__global__".to_string(),
        AlertConfig {
            cpu: global_cpu,
            memory: global_mem,
            disk: global_disk,
            load: global_load,
            network: global_network,
            temperature: global_temperature,
            gpu: global_gpu,
            load_threshold: 4.0,
            load_cooldown_secs: 60,
        },
    );

    Ok(map)
}

/// Upsert a global or per-host alert config row.
///
/// Generic over `SqliteExecutor` so callers can pass either a `&DbPool`
/// (single-shot upsert with implicit commit) or `&mut *tx` from
/// `pool.begin()` (batched upsert that all-or-nothing-commits with the
/// caller's transaction). The handler-side update endpoints loop over
/// 7+ rules per call; running each through its own pool acquisition
/// used to leave the DB in a half-applied state when rule N+1 failed
/// validation or hit a row-level conflict mid-loop.
pub async fn upsert_alert_config<'e, E>(
    executor: E,
    host_key: Option<&str>,
    req: &UpsertAlertRequest,
) -> Result<AlertConfigRow, sqlx::Error>
where
    E: sqlx::SqliteExecutor<'e>,
{
    let sql = format!(
        r#"
        INSERT INTO alert_configs
            (host_key, metric_type, sub_key, enabled, threshold, sustained_secs, cooldown_secs, updated_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%s','now'))
        ON CONFLICT (coalesce(host_key, ''), metric_type, coalesce(sub_key, ''))
        DO UPDATE SET
            enabled        = excluded.enabled,
            threshold      = excluded.threshold,
            sustained_secs = excluded.sustained_secs,
            cooldown_secs  = excluded.cooldown_secs,
            updated_at     = strftime('%s','now')
        RETURNING {ALERT_CONFIG_COLUMNS}
        "#
    );
    sqlx::query_as::<_, AlertConfigRow>(&sql)
        .bind(host_key)
        .bind(req.metric_type)
        .bind(req.sub_key.as_deref())
        .bind(req.enabled)
        .bind(req.threshold)
        .bind(req.sustained_secs)
        .bind(req.cooldown_secs)
        .fetch_one(executor)
        .await
}

/// Apply the same set of overrides to every host.
///
/// Previously issued N×M individual `sqlx::query_as::<_, AlertConfigRow>...`
/// round-trips inside a transaction. At 500 hosts × 10 rules that was 5 000
/// network round-trips competing for the SQLite writer lock. This version
/// batches into a single `INSERT ... VALUES (...), (...) ... ON CONFLICT ...
/// RETURNING` per chunk, where the chunk size is bounded by SQLite's variable
/// limit (`SQLITE_MAX_VARIABLE_NUMBER`, default 32 766).
///
/// Each row binds 7 parameters (`updated_at` is set from a SQL expression,
/// not a bind), so a chunk of 4 000 rows uses 28 000 binds — safely under
/// the cap even on SQLite builds that still ship the legacy 999 limit we
/// never target.
pub async fn bulk_upsert_host_configs(
    pool: &DbPool,
    host_keys: &[String],
    requests: &[UpsertAlertRequest],
) -> Result<Vec<AlertConfigRow>, sqlx::Error> {
    if host_keys.is_empty() || requests.is_empty() {
        return Ok(Vec::new());
    }

    /// Cap chosen so `chunk_rows * 7 binds` stays below the SQLite 32 766
    /// variable limit with headroom for future column additions.
    const CHUNK_ROWS: usize = 4_000;

    let total = host_keys.len().saturating_mul(requests.len());
    let mut out = Vec::with_capacity(total);
    let mut buffer: Vec<BulkUpsertRow<'_>> = Vec::with_capacity(CHUNK_ROWS.min(total));

    for hk in host_keys {
        for req in requests {
            buffer.push(BulkUpsertRow {
                host_key: hk.as_str(),
                metric_type: req.metric_type,
                sub_key: req.sub_key.as_deref(),
                enabled: req.enabled,
                threshold: req.threshold,
                sustained_secs: req.sustained_secs,
                cooldown_secs: req.cooldown_secs,
            });
            if buffer.len() >= CHUNK_ROWS {
                flush_bulk_chunk(pool, &buffer, &mut out).await?;
                buffer.clear();
            }
        }
    }
    flush_bulk_chunk(pool, &buffer, &mut out).await?;
    Ok(out)
}

#[derive(Clone, Copy)]
struct BulkUpsertRow<'a> {
    host_key: &'a str,
    metric_type: MetricType,
    sub_key: Option<&'a str>,
    enabled: bool,
    threshold: f64,
    sustained_secs: i32,
    cooldown_secs: i32,
}

/// Flush a chunk of up to `CHUNK_ROWS` bulk-upsert rows into a single
/// SQLite statement. Extracted from `bulk_upsert_host_configs` because
/// async closures still require awkward lifetime gymnastics — a plain
/// async fn inherits the borrow checker's normal elision rules.
async fn flush_bulk_chunk(
    pool: &DbPool,
    chunk: &[BulkUpsertRow<'_>],
    out: &mut Vec<AlertConfigRow>,
) -> Result<(), sqlx::Error> {
    use sqlx::{QueryBuilder, Sqlite};

    if chunk.is_empty() {
        return Ok(());
    }
    let mut qb = QueryBuilder::<Sqlite>::new(
        "INSERT INTO alert_configs \
         (host_key, metric_type, sub_key, enabled, threshold, sustained_secs, cooldown_secs, updated_at) ",
    );
    qb.push_values(chunk.iter(), |mut b, row| {
        b.push_bind(row.host_key)
            .push_bind(row.metric_type)
            .push_bind(row.sub_key)
            .push_bind(row.enabled)
            .push_bind(row.threshold)
            .push_bind(row.sustained_secs)
            .push_bind(row.cooldown_secs)
            .push("strftime('%s','now')");
    });
    qb.push(
        " ON CONFLICT (coalesce(host_key, ''), metric_type, coalesce(sub_key, '')) \
          DO UPDATE SET \
              enabled = excluded.enabled, \
              threshold = excluded.threshold, \
              sustained_secs = excluded.sustained_secs, \
              cooldown_secs = excluded.cooldown_secs, \
              updated_at = strftime('%s','now') \
          RETURNING ",
    );
    qb.push(ALERT_CONFIG_COLUMNS);
    let rows = qb
        .build_query_as::<AlertConfigRow>()
        .fetch_all(pool)
        .await?;
    out.extend(rows);
    Ok(())
}

/// Delete all per-host alert config overrides (host reverts to global defaults).
pub async fn delete_host_configs(pool: &DbPool, host_key: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM alert_configs WHERE host_key = ?1")
        .bind(host_key)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
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

    fn req(metric: MetricType, threshold: f64) -> UpsertAlertRequest {
        UpsertAlertRequest {
            metric_type: metric,
            enabled: true,
            threshold,
            sustained_secs: 300,
            cooldown_secs: 1800,
            sub_key: None,
        }
    }

    #[tokio::test]
    async fn nulls_not_distinct_collapses_globals() {
        let pool = fresh_pool().await;

        let first = upsert_alert_config(&pool, None, &req(MetricType::Cpu, 80.0))
            .await
            .unwrap();
        assert!(first.host_key.is_none());
        assert_eq!(first.threshold, 80.0);

        // Second upsert with the SAME scope (NULL host_key, NULL sub_key)
        // must UPDATE the existing row — the expression-based UNIQUE
        // index stops SQLite from treating the two NULLs as distinct.
        let second = upsert_alert_config(&pool, None, &req(MetricType::Cpu, 90.0))
            .await
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(second.threshold, 90.0);

        let globals = get_global_configs(&pool).await.unwrap();
        assert_eq!(globals.len(), 1);
    }

    #[tokio::test]
    async fn per_host_overrides_coexist_with_globals() {
        let pool = fresh_pool().await;
        upsert_alert_config(&pool, None, &req(MetricType::Cpu, 80.0))
            .await
            .unwrap();
        upsert_alert_config(
            &pool,
            Some("192.168.1.10:9101"),
            &req(MetricType::Cpu, 60.0),
        )
        .await
        .unwrap();

        assert_eq!(get_global_configs(&pool).await.unwrap().len(), 1);
        let per_host = get_host_configs(&pool, "192.168.1.10:9101").await.unwrap();
        assert_eq!(per_host.len(), 1);
        assert_eq!(per_host[0].threshold, 60.0);

        // delete_host_configs clears the override only.
        assert!(
            delete_host_configs(&pool, "192.168.1.10:9101")
                .await
                .unwrap()
        );
        assert!(
            get_host_configs(&pool, "192.168.1.10:9101")
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(get_global_configs(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn bulk_upsert_applies_matrix() {
        let pool = fresh_pool().await;
        let hosts = vec![
            "a:9101".to_string(),
            "b:9101".to_string(),
            "c:9101".to_string(),
        ];
        let requests = vec![req(MetricType::Cpu, 70.0), req(MetricType::Memory, 85.0)];

        let rows = bulk_upsert_host_configs(&pool, &hosts, &requests)
            .await
            .unwrap();
        assert_eq!(rows.len(), 6); // 3 hosts × 2 metrics

        // Re-run with different thresholds — should update in place, not duplicate.
        let new_requests = vec![req(MetricType::Cpu, 75.0), req(MetricType::Memory, 90.0)];
        let rows2 = bulk_upsert_host_configs(&pool, &hosts, &new_requests)
            .await
            .unwrap();
        assert_eq!(rows2.len(), 6);

        let a = get_host_configs(&pool, "a:9101").await.unwrap();
        assert_eq!(a.len(), 2);
        assert!(
            a.iter()
                .any(|r| r.metric_type == MetricType::Cpu && r.threshold == 75.0)
        );
    }

    #[tokio::test]
    async fn upsert_inside_rolled_back_tx_does_not_persist() {
        // Pin the new transactional contract: if a handler-level batch
        // calls `upsert_alert_config(&mut *tx, ...)` and the surrounding
        // transaction rolls back (whether explicit or via `?` early-exit),
        // the upserted row must not survive. Regression-protects the
        // half-applied state shape that was possible when the handler
        // looped over the bare pool.
        let pool = fresh_pool().await;
        let mut tx = pool.begin().await.unwrap();

        let row = upsert_alert_config(&mut *tx, None, &req(MetricType::Cpu, 42.0))
            .await
            .unwrap();
        // Inside the tx, the row exists; we just got it back from RETURNING.
        assert_eq!(row.threshold, 42.0);

        // Drop the transaction *without* committing — sqlx auto-rolls
        // back on Drop, but make the rollback explicit so the assertion
        // below tests the semantics, not the Drop timing.
        tx.rollback().await.unwrap();

        let globals = get_global_configs(&pool).await.unwrap();
        assert!(
            globals.is_empty(),
            "rolled-back upsert must not leave a row behind"
        );
    }

    #[tokio::test]
    async fn upsert_committed_tx_persists() {
        // Mirror of the rollback test: if the transaction commits, the
        // row must be visible to a subsequent fresh read. Catches a
        // future refactor that accidentally double-buffers the upserts
        // in memory and forgets to commit.
        let pool = fresh_pool().await;
        let mut tx = pool.begin().await.unwrap();

        upsert_alert_config(&mut *tx, None, &req(MetricType::Cpu, 42.0))
            .await
            .unwrap();
        upsert_alert_config(&mut *tx, None, &req(MetricType::Memory, 88.0))
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let globals = get_global_configs(&pool).await.unwrap();
        assert_eq!(globals.len(), 2);
    }

    #[tokio::test]
    async fn load_all_as_map_resolves_overrides() {
        let pool = fresh_pool().await;
        upsert_alert_config(&pool, None, &req(MetricType::Cpu, 80.0))
            .await
            .unwrap();
        upsert_alert_config(
            &pool,
            Some("override.example:9101"),
            &req(MetricType::Cpu, 50.0),
        )
        .await
        .unwrap();

        let map = load_all_as_map(&pool).await.unwrap();
        // "__global__" + one host
        assert!(map.contains_key("__global__"));
        assert!(map.contains_key("override.example:9101"));
        assert_eq!(map["override.example:9101"].cpu.threshold, 50.0);
        assert_eq!(map["__global__"].cpu.threshold, 80.0);
    }
}
