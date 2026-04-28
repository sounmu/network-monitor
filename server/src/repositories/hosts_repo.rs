use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::db::DbPool;

/// Row struct for the `hosts` table.
///
/// `ports` and `containers` are persisted as JSON text columns in SQLite
/// and parsed at the boundary via `HostRowRaw` — the Rust type stays
/// `Vec<i32>` / `Vec<String>` so consumers are unaffected.
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

// `ports` / `containers` are stored as JSON text (schema: TEXT default
// `'[]'`). `HostRowRaw` holds the raw `String` fields and decodes them
// into `Vec<T>` via `TryFrom` at the boundary.

#[derive(sqlx::FromRow)]
struct HostRowRaw {
    host_key: String,
    display_name: String,
    scrape_interval_secs: i32,
    load_threshold: f64,
    ports: String,
    containers: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    os_info: Option<String>,
    cpu_model: Option<String>,
    memory_total_mb: Option<i64>,
    boot_time: Option<i64>,
    ip_address: Option<String>,
    system_info_updated_at: Option<DateTime<Utc>>,
}

impl TryFrom<HostRowRaw> for HostRow {
    type Error = sqlx::Error;

    fn try_from(raw: HostRowRaw) -> Result<Self, Self::Error> {
        let ports: Vec<i32> =
            serde_json::from_str(&raw.ports).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        let containers: Vec<String> =
            serde_json::from_str(&raw.containers).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        Ok(Self {
            host_key: raw.host_key,
            display_name: raw.display_name,
            scrape_interval_secs: raw.scrape_interval_secs,
            load_threshold: raw.load_threshold,
            ports,
            containers,
            created_at: raw.created_at,
            updated_at: raw.updated_at,
            os_info: raw.os_info,
            cpu_model: raw.cpu_model,
            memory_total_mb: raw.memory_total_mb,
            boot_time: raw.boot_time,
            ip_address: raw.ip_address,
            system_info_updated_at: raw.system_info_updated_at,
        })
    }
}

const HOST_COLUMNS: &str = "host_key, display_name, scrape_interval_secs, load_threshold, \
                            ports, containers, created_at, updated_at, \
                            os_info, cpu_model, memory_total_mb, boot_time, ip_address, \
                            system_info_updated_at";

pub async fn list_hosts(pool: &DbPool) -> Result<Vec<HostRow>, sqlx::Error> {
    let sql = format!("SELECT {HOST_COLUMNS} FROM hosts ORDER BY host_key");
    let raws = sqlx::query_as::<_, HostRowRaw>(&sql)
        .fetch_all(pool)
        .await?;
    raws.into_iter().map(HostRow::try_from).collect()
}

pub async fn get_host(pool: &DbPool, host_key: &str) -> Result<Option<HostRow>, sqlx::Error> {
    let sql = format!("SELECT {HOST_COLUMNS} FROM hosts WHERE host_key = ?1");
    let raw = sqlx::query_as::<_, HostRowRaw>(&sql)
        .bind(host_key)
        .fetch_optional(pool)
        .await?;
    raw.map(HostRow::try_from).transpose()
}

pub async fn create_host(pool: &DbPool, req: &CreateHostRequest) -> Result<HostRow, sqlx::Error> {
    let ports_text = serde_json::to_string(&req.ports).expect("Vec<i32> always serialises");
    let containers_text =
        serde_json::to_string(&req.containers).expect("Vec<String> always serialises");

    let sql = format!(
        r#"
        INSERT INTO hosts (host_key, display_name, scrape_interval_secs, load_threshold, ports, containers)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        RETURNING {HOST_COLUMNS}
        "#
    );
    let raw = sqlx::query_as::<_, HostRowRaw>(&sql)
        .bind(&req.host_key)
        .bind(&req.display_name)
        .bind(req.scrape_interval_secs)
        .bind(req.load_threshold)
        .bind(&ports_text)
        .bind(&containers_text)
        .fetch_one(pool)
        .await?;
    HostRow::try_from(raw)
}

pub async fn update_host(
    pool: &DbPool,
    host_key: &str,
    req: &UpdateHostRequest,
) -> Result<Option<HostRow>, sqlx::Error> {
    // Bind NULL for absent fields so COALESCE keeps the existing value.
    let ports_text = req
        .ports
        .as_ref()
        .map(|v| serde_json::to_string(v).expect("Vec<i32> always serialises"));
    let containers_text = req
        .containers
        .as_ref()
        .map(|v| serde_json::to_string(v).expect("Vec<String> always serialises"));

    let sql = format!(
        r#"
        UPDATE hosts SET
            display_name         = COALESCE(?2, display_name),
            scrape_interval_secs = COALESCE(?3, scrape_interval_secs),
            load_threshold       = COALESCE(?4, load_threshold),
            ports                = COALESCE(?5, ports),
            containers           = COALESCE(?6, containers),
            updated_at           = strftime('%s','now')
        WHERE host_key = ?1
        RETURNING {HOST_COLUMNS}
        "#
    );
    let raw = sqlx::query_as::<_, HostRowRaw>(&sql)
        .bind(host_key)
        .bind(&req.display_name)
        .bind(req.scrape_interval_secs)
        .bind(req.load_threshold)
        .bind(ports_text)
        .bind(containers_text)
        .fetch_optional(pool)
        .await?;
    raw.map(HostRow::try_from).transpose()
}

pub async fn delete_host(pool: &DbPool, host_key: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM hosts WHERE host_key = ?1")
        .bind(host_key)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn update_system_info(
    pool: &DbPool,
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
            os_info                = ?2,
            cpu_model              = ?3,
            memory_total_mb        = ?4,
            boot_time              = ?5,
            ip_address             = ?6,
            system_info_updated_at = strftime('%s','now'),
            updated_at             = strftime('%s','now')
        WHERE host_key = ?1
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

    fn sample_create_req() -> CreateHostRequest {
        CreateHostRequest {
            host_key: "192.168.1.10:9101".into(),
            display_name: "homeserver".into(),
            scrape_interval_secs: 10,
            load_threshold: 4.0,
            ports: vec![80, 443, 8080],
            containers: vec!["nginx".into(), "postgres".into()],
        }
    }

    #[tokio::test]
    async fn roundtrip_ports_and_containers_as_json() {
        let pool = fresh_pool().await;
        let created = create_host(&pool, &sample_create_req()).await.unwrap();

        assert_eq!(created.ports, vec![80, 443, 8080]);
        assert_eq!(
            created.containers,
            vec!["nginx".to_string(), "postgres".to_string()]
        );

        let fetched = get_host(&pool, "192.168.1.10:9101").await.unwrap().unwrap();
        assert_eq!(fetched.ports, created.ports);
        assert_eq!(fetched.containers, created.containers);
    }

    #[tokio::test]
    async fn update_host_coalesce_leaves_absent_fields() {
        let pool = fresh_pool().await;
        create_host(&pool, &sample_create_req()).await.unwrap();

        let updated = update_host(
            &pool,
            "192.168.1.10:9101",
            &UpdateHostRequest {
                display_name: None,
                scrape_interval_secs: Some(30),
                load_threshold: None,
                ports: Some(vec![22]),
                containers: None,
            },
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(updated.display_name, "homeserver"); // untouched
        assert_eq!(updated.scrape_interval_secs, 30); // new
        assert_eq!(updated.load_threshold, 4.0); // untouched
        assert_eq!(updated.ports, vec![22]); // new
        assert_eq!(
            updated.containers,
            vec!["nginx".to_string(), "postgres".to_string()]
        ); // untouched

        assert!(delete_host(&pool, "192.168.1.10:9101").await.unwrap());
        assert!(
            get_host(&pool, "192.168.1.10:9101")
                .await
                .unwrap()
                .is_none()
        );
    }
}
