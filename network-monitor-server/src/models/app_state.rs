use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::models::sse_payloads::{HostStatusPayload, SseBroadcast};
use crate::repositories::hosts_repo::HostRow;
use crate::repositories::metrics_repo::MetricsRow;
use tokio::sync::broadcast;

// ──────────────────────────────────────────────
// Application shared state
// ──────────────────────────────────────────────

/// Top-level state struct injected into the Axum router.
/// Fully DB-driven — no config.yaml dependency at runtime.
#[derive(Clone)]
pub struct AppState {
    /// In-memory store for per-host metric history and alert state
    pub store: SharedStore,
    /// Shared HTTP client reused for alert notifications and any future external API calls
    pub http_client: reqwest::Client,
    /// PostgreSQL connection pool
    pub db_pool: sqlx::PgPool,
    /// Global scrape interval in seconds (from env var or default 10)
    pub scrape_interval_secs: u64,
    /// SSE event broadcast channel sender
    pub sse_tx: broadcast::Sender<SseBroadcast>,
    /// Cache of the most recently sent per-host status payload
    pub last_known_status: Arc<RwLock<HashMap<String, HostStatusPayload>>>,
    /// TTL cache for long-range metric queries (avoids repeated DB scans for same range)
    pub metrics_query_cache: Arc<MetricsQueryCache>,
}

impl AppState {
    /// Pre-populate last_known_status from the hosts table on startup.
    /// Ensures SSE clients see all configured hosts immediately upon connection.
    pub fn pre_populate_status(&self, hosts: &[HostRow]) {
        let mut lks = self.last_known_status.write().expect("RwLock poisoned");
        for host in hosts {
            lks.entry(host.host_key.clone())
                .or_insert_with(|| HostStatusPayload {
                    host_key: host.host_key.clone(),
                    display_name: host.display_name.clone(),
                    is_online: false,
                    last_seen: String::new(),
                    docker_containers: vec![],
                    ports: vec![],
                    disks: vec![],
                    processes: vec![],
                    temperatures: vec![],
                    gpus: vec![],
                });
        }
    }
}

/// Thread-safe shared store type alias (RwLock-guarded)
pub type SharedStore = Arc<RwLock<MetricsStore>>;

// ──────────────────────────────────────────────
// Lightweight alert data point cache
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct AlertMetricPoint {
    pub received_at: Instant,
    pub cpu_usage_percent: f32,
    pub memory_usage_percent: f32,
}

// ──────────────────────────────────────────────
// Alert config runtime structs
// Loaded from the alert_configs DB table each scrape cycle
// ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MetricAlertRule {
    pub enabled: bool,
    pub threshold: f64,
    pub sustained_secs: u64,
    pub cooldown_secs: u64,
}

#[derive(Debug, Clone)]
pub struct AlertConfig {
    pub cpu: MetricAlertRule,
    pub memory: MetricAlertRule,
    pub disk: MetricAlertRule,
    pub load_threshold: f64,
    pub load_cooldown_secs: u64,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            cpu: MetricAlertRule {
                enabled: true,
                threshold: 80.0,
                sustained_secs: 5 * 60,
                cooldown_secs: 60,
            },
            memory: MetricAlertRule {
                enabled: true,
                threshold: 90.0,
                sustained_secs: 5 * 60,
                cooldown_secs: 60,
            },
            disk: MetricAlertRule {
                enabled: true,
                threshold: 90.0,
                sustained_secs: 0, // Disk alerts fire immediately (no sustained window)
                cooldown_secs: 300,
            },
            load_threshold: 4.0,
            load_cooldown_secs: 60,
        }
    }
}

/// Top-level in-memory metrics store
pub struct MetricsStore {
    pub hosts: HashMap<String, HostRecord>,
}

impl MetricsStore {
    pub fn new() -> Self {
        Self {
            hosts: HashMap::new(),
        }
    }
}

/// Per-host alert history, alert state, and SSE-related state
pub struct HostRecord {
    pub last_known_hostname: String,
    pub alert_history: VecDeque<AlertMetricPoint>,
    pub alert_state: AlertState,
    pub network_prev: Option<(u64, u64, Instant)>,
    pub prev_status_hash: Option<u64>,
    pub last_status_sent: Option<Instant>,
}

impl HostRecord {
    pub fn new(hostname: String) -> Self {
        Self {
            last_known_hostname: hostname,
            alert_history: VecDeque::new(),
            alert_state: AlertState::new(),
            network_prev: None,
            prev_status_hash: None,
            last_status_sent: None,
        }
    }

    pub fn push_alert_point(&mut self, point: AlertMetricPoint, retention: Duration) {
        self.alert_history.push_back(point);
        while let Some(front) = self.alert_history.front() {
            if front.received_at.elapsed() > retention {
                self.alert_history.pop_front();
            } else {
                break;
            }
        }
    }
}

pub struct AlertState {
    pub offline_alerted: bool,
    pub cpu_alerted: bool,
    pub memory_alerted: bool,
    pub load_alerted: bool,
    /// Per-mount-point disk alert state (keyed by mount_point string)
    pub disk_alerted: HashMap<String, bool>,
    pub last_offline_alert: Option<Instant>,
    pub last_recovery_alert: Option<Instant>,
    pub last_cpu_alert: Option<Instant>,
    pub last_memory_alert: Option<Instant>,
    pub last_load_alert: Option<Instant>,
    pub last_disk_alert: Option<Instant>,
    pub port_alerted: HashMap<u16, Instant>,
}

impl AlertState {
    pub fn new() -> Self {
        Self {
            offline_alerted: false,
            cpu_alerted: false,
            memory_alerted: false,
            load_alerted: false,
            disk_alerted: HashMap::new(),
            last_offline_alert: None,
            last_recovery_alert: None,
            last_cpu_alert: None,
            last_memory_alert: None,
            last_load_alert: None,
            last_disk_alert: None,
            port_alerted: HashMap::new(),
        }
    }
}

// ──────────────────────────────────────────────
// TTL cache for long-range metric queries
// ──────────────────────────────────────────────

struct CacheEntry {
    data: Vec<MetricsRow>,
    inserted_at: Instant,
}

/// Simple in-memory TTL cache for time-range metric queries.
///
/// Prevents repeated DB scans when multiple users view the same dashboard range.
/// Entries expire after `ttl` and are lazily evicted on the next `get` or periodic cleanup.
pub struct MetricsQueryCache {
    entries: RwLock<HashMap<String, CacheEntry>>,
    ttl: Duration,
}

impl MetricsQueryCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            ttl,
        }
    }

    /// Build a cache key from query parameters.
    /// Rounds timestamps to 5-minute boundaries so near-identical queries share a cache entry.
    pub fn make_key(host_key: &str, start_ts: i64, end_ts: i64) -> String {
        let start_rounded = start_ts / 300 * 300;
        let end_rounded = (end_ts + 299) / 300 * 300;
        format!("{}:{}:{}", host_key, start_rounded, end_rounded)
    }

    /// Get a cached result if it exists and hasn't expired.
    pub fn get(&self, key: &str) -> Option<Vec<MetricsRow>> {
        let entries = self.entries.read().ok()?;
        let entry = entries.get(key)?;
        if entry.inserted_at.elapsed() < self.ttl {
            Some(entry.data.clone())
        } else {
            None
        }
    }

    /// Insert a query result into the cache.
    pub fn insert(&self, key: String, data: Vec<MetricsRow>) {
        if let Ok(mut entries) = self.entries.write() {
            entries.insert(
                key,
                CacheEntry {
                    data,
                    inserted_at: Instant::now(),
                },
            );
        }
    }

    /// Remove expired entries. Called periodically from a background task.
    pub fn evict_expired(&self) {
        if let Ok(mut entries) = self.entries.write() {
            entries.retain(|_, entry| entry.inserted_at.elapsed() < self.ttl);
        }
    }
}
