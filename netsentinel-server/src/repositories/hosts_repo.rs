use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// Row struct for the `hosts` table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct HostRow {
    pub host_key: String,
    pub display_name: String,
    pub scrape_interval_secs: i32,
    pub load_threshold: f64,
    pub ports: Vec<i32>,
    pub containers: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    // Static system info (populated by /system-info agent endpoint)
    pub os_info: Option<String>,
    pub cpu_model: Option<String>,
    pub memory_total_mb: Option<i64>,
    pub boot_time: Option<i64>,
    pub ip_address: Option<String>,
    pub system_info_updated_at: Option<DateTime<Utc>>,
}

/// Fetch all hosts ordered by host_key
pub async fn list_hosts(pool: &PgPool) -> Result<Vec<HostRow>, sqlx::Error> {
    sqlx::query_as::<_, HostRow>("SELECT * FROM hosts ORDER BY host_key")
        .fetch_all(pool)
        .await
}

/// Fetch a single host by host_key
pub async fn get_host(pool: &PgPool, host_key: &str) -> Result<Option<HostRow>, sqlx::Error> {
    sqlx::query_as::<_, HostRow>("SELECT * FROM hosts WHERE host_key = $1")
        .bind(host_key)
        .fetch_optional(pool)
        .await
}

/// Request body for creating a new host
#[derive(Debug, Deserialize)]
pub struct CreateHostRequest {
    pub host_key: String,
    pub display_name: String,
    #[serde(default = "default_scrape_interval")]
    pub scrape_interval_secs: i32,
    #[serde(default = "default_load_threshold")]
    pub load_threshold: f64,
    #[serde(default = "default_ports")]
    pub ports: Vec<i32>,
    #[serde(default)]
    pub containers: Vec<String>,
}

fn default_scrape_interval() -> i32 {
    10
}
fn default_load_threshold() -> f64 {
    4.0
}
fn default_ports() -> Vec<i32> {
    vec![80, 443]
}

/// Request body for updating an existing host
#[derive(Debug, Deserialize)]
pub struct UpdateHostRequest {
    pub display_name: Option<String>,
    pub scrape_interval_secs: Option<i32>,
    pub load_threshold: Option<f64>,
    pub ports: Option<Vec<i32>>,
    pub containers: Option<Vec<String>>,
}

/// Insert a new host record
pub async fn create_host(pool: &PgPool, req: &CreateHostRequest) -> Result<HostRow, sqlx::Error> {
    sqlx::query_as::<_, HostRow>(
        r#"
        INSERT INTO hosts (host_key, display_name, scrape_interval_secs, load_threshold, ports, containers)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING *
        "#,
    )
    .bind(&req.host_key)
    .bind(&req.display_name)
    .bind(req.scrape_interval_secs)
    .bind(req.load_threshold)
    .bind(&req.ports)
    .bind(&req.containers)
    .fetch_one(pool)
    .await
}

/// Update host config fields (COALESCE — only provided fields are changed)
pub async fn update_host(
    pool: &PgPool,
    host_key: &str,
    req: &UpdateHostRequest,
) -> Result<Option<HostRow>, sqlx::Error> {
    sqlx::query_as::<_, HostRow>(
        r#"
        UPDATE hosts SET
            display_name = COALESCE($2, display_name),
            scrape_interval_secs = COALESCE($3, scrape_interval_secs),
            load_threshold = COALESCE($4, load_threshold),
            ports = COALESCE($5, ports),
            containers = COALESCE($6, containers),
            updated_at = NOW()
        WHERE host_key = $1
        RETURNING *
        "#,
    )
    .bind(host_key)
    .bind(&req.display_name)
    .bind(req.scrape_interval_secs)
    .bind(req.load_threshold)
    .bind(&req.ports)
    .bind(&req.containers)
    .fetch_optional(pool)
    .await
}

/// Delete a host record; returns true if a row was removed
pub async fn delete_host(pool: &PgPool, host_key: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM hosts WHERE host_key = $1")
        .bind(host_key)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Update static system info fields fetched from the agent's /system-info endpoint.
pub async fn update_system_info(
    pool: &PgPool,
    host_key: &str,
    os_info: &str,
    cpu_model: &str,
    memory_total_mb: i64,
    boot_time: i64,
    ip_address: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE hosts SET
            os_info = $2,
            cpu_model = $3,
            memory_total_mb = $4,
            boot_time = $5,
            ip_address = $6,
            system_info_updated_at = NOW(),
            updated_at = NOW()
        WHERE host_key = $1
        "#,
    )
    .bind(host_key)
    .bind(os_info)
    .bind(cpu_model)
    .bind(memory_total_mb)
    .bind(boot_time)
    .bind(ip_address)
    .execute(pool)
    .await?;
    Ok(())
}

/// Auto-register a host on first metric receipt.
/// Only updates display_name if it was empty/null — prevents a compromised agent
/// from overwriting an admin-set name (SS-15).
pub async fn ensure_host_registered(
    pool: &PgPool,
    host_key: &str,
    display_name: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO hosts (host_key, display_name)
        VALUES ($1, $2)
        ON CONFLICT (host_key) DO UPDATE SET
            display_name = CASE
                WHEN hosts.display_name = '' OR hosts.display_name IS NULL
                THEN EXCLUDED.display_name
                ELSE hosts.display_name
            END,
            updated_at = NOW()
        "#,
    )
    .bind(host_key)
    .bind(display_name)
    .execute(pool)
    .await?;
    Ok(())
}
