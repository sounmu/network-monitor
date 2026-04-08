use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::models::app_state::{AlertConfig, MetricAlertRule};

/// Row struct for the `alert_configs` table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AlertConfigRow {
    pub id: i32,
    pub host_key: Option<String>,
    pub metric_type: String,
    pub enabled: bool,
    pub threshold: f64,
    pub sustained_secs: i32,
    pub cooldown_secs: i32,
    pub updated_at: DateTime<Utc>,
}

/// Fetch global default alert configs (host_key IS NULL)
pub async fn get_global_configs(pool: &PgPool) -> Result<Vec<AlertConfigRow>, sqlx::Error> {
    sqlx::query_as::<_, AlertConfigRow>(
        "SELECT * FROM alert_configs WHERE host_key IS NULL ORDER BY metric_type",
    )
    .fetch_all(pool)
    .await
}

/// Fetch per-host alert config overrides only
pub async fn get_host_configs(
    pool: &PgPool,
    host_key: &str,
) -> Result<Vec<AlertConfigRow>, sqlx::Error> {
    sqlx::query_as::<_, AlertConfigRow>(
        "SELECT * FROM alert_configs WHERE host_key = $1 ORDER BY metric_type",
    )
    .bind(host_key)
    .fetch_all(pool)
    .await
}

/// Load all alert configs and build a per-host AlertConfig map.
/// Resolution order: host-specific override → global default.
pub async fn load_all_as_map(pool: &PgPool) -> Result<HashMap<String, AlertConfig>, sqlx::Error> {
    let rows = sqlx::query_as::<_, AlertConfigRow>(
        "SELECT * FROM alert_configs ORDER BY host_key NULLS FIRST, metric_type",
    )
    .fetch_all(pool)
    .await?;

    // Extract global defaults
    let mut global_cpu = default_cpu_rule();
    let mut global_mem = default_memory_rule();
    let mut global_disk = default_disk_rule();

    for row in &rows {
        if row.host_key.is_none() {
            match row.metric_type.as_str() {
                "cpu" => global_cpu = row_to_rule(row),
                "memory" => global_mem = row_to_rule(row),
                "disk" => global_disk = row_to_rule(row),
                _ => {}
            }
        }
    }

    // Build per-host configs using a single-pass lookup table: (host_key, metric_type) → rule
    let mut map = HashMap::new();
    let mut host_overrides: HashMap<(&str, &str), MetricAlertRule> = HashMap::new();
    let mut host_keys_set = std::collections::HashSet::new();

    for row in &rows {
        if let Some(ref hk) = row.host_key {
            host_overrides.insert((hk.as_str(), row.metric_type.as_str()), row_to_rule(row));
            host_keys_set.insert(hk.as_str());
        }
    }

    for hk in host_keys_set {
        let cpu = host_overrides
            .get(&(hk, "cpu"))
            .cloned()
            .unwrap_or(global_cpu.clone());
        let mem = host_overrides
            .get(&(hk, "memory"))
            .cloned()
            .unwrap_or(global_mem.clone());
        let disk = host_overrides
            .get(&(hk, "disk"))
            .cloned()
            .unwrap_or(global_disk.clone());

        map.insert(
            hk.to_string(),
            AlertConfig {
                cpu,
                memory: mem,
                disk,
                load_threshold: 4.0, // sourced from the hosts table at scrape time
                load_cooldown_secs: 60,
            },
        );
    }

    // Store global defaults under "__global__" key as fallback for hosts with no overrides
    map.insert(
        "__global__".to_string(),
        AlertConfig {
            cpu: global_cpu,
            memory: global_mem,
            disk: global_disk,
            load_threshold: 4.0,
            load_cooldown_secs: 60,
        },
    );

    Ok(map)
}

/// Resolve the effective AlertConfig for a host (host override → global fallback)
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

/// Request body for upserting an alert config entry
#[derive(Debug, Deserialize)]
pub struct UpsertAlertRequest {
    pub metric_type: String,
    pub enabled: bool,
    pub threshold: f64,
    pub sustained_secs: i32,
    pub cooldown_secs: i32,
}

/// Upsert a global or per-host alert config row
pub async fn upsert_alert_config(
    pool: &PgPool,
    host_key: Option<&str>,
    req: &UpsertAlertRequest,
) -> Result<AlertConfigRow, sqlx::Error> {
    sqlx::query_as::<_, AlertConfigRow>(
        r#"
        INSERT INTO alert_configs (host_key, metric_type, enabled, threshold, sustained_secs, cooldown_secs, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, NOW())
        ON CONFLICT (host_key, metric_type)
        DO UPDATE SET
            enabled = EXCLUDED.enabled,
            threshold = EXCLUDED.threshold,
            sustained_secs = EXCLUDED.sustained_secs,
            cooldown_secs = EXCLUDED.cooldown_secs,
            updated_at = NOW()
        RETURNING *
        "#,
    )
    .bind(host_key)
    .bind(&req.metric_type)
    .bind(req.enabled)
    .bind(req.threshold)
    .bind(req.sustained_secs)
    .bind(req.cooldown_secs)
    .fetch_one(pool)
    .await
}

/// Delete all per-host alert config overrides (host reverts to global defaults)
pub async fn delete_host_configs(pool: &PgPool, host_key: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM alert_configs WHERE host_key = $1")
        .bind(host_key)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

fn row_to_rule(row: &AlertConfigRow) -> MetricAlertRule {
    MetricAlertRule {
        enabled: row.enabled,
        threshold: row.threshold,
        sustained_secs: row.sustained_secs as u64,
        cooldown_secs: row.cooldown_secs as u64,
    }
}

fn default_cpu_rule() -> MetricAlertRule {
    MetricAlertRule {
        enabled: true,
        threshold: 80.0,
        sustained_secs: 300,
        cooldown_secs: 60,
    }
}

fn default_memory_rule() -> MetricAlertRule {
    MetricAlertRule {
        enabled: true,
        threshold: 90.0,
        sustained_secs: 300,
        cooldown_secs: 60,
    }
}

fn default_disk_rule() -> MetricAlertRule {
    MetricAlertRule {
        enabled: true,
        threshold: 90.0,
        sustained_secs: 0,
        cooldown_secs: 300,
    }
}
