use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::models::sse_payloads::{HostStatusPayload, SseBroadcast};
use crate::repositories::hosts_repo::HostRow;
use crate::repositories::metrics_repo::{ChartMetricsRow, MetricsRow};
use crate::services::hosts_snapshot::SharedHostsSnapshot;
use crate::services::monitors_snapshot::SharedMonitorsSnapshot;
use crate::services::sse_ticket::SseTicketStore;
use serde_json::Value;
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
    /// Database connection pool. NetSentinel is SQLite-only; the alias
    /// keeps downstream modules from importing sqlx internals directly.
    pub db_pool: crate::db::DbPool,
    /// Global scrape interval in seconds (from env var or default 10)
    pub scrape_interval_secs: u64,
    /// Configured sqlx pool size, used by fan-out handlers to avoid
    /// out-concurrencying the SQLite connection pool.
    pub max_db_connections: u32,
    /// SSE event broadcast channel sender
    pub sse_tx: broadcast::Sender<SseBroadcast>,
    /// Cache of the most recently sent per-host status payload.
    ///
    /// Uses `std::sync::RwLock` (not `tokio::sync::RwLock`) deliberately:
    /// lock scopes are micro-duration data shuffles with **no `.await` inside**,
    /// so the lower per-access overhead of std RwLock beats tokio's cooperative
    /// scheduling cost. Do not add `.await` calls inside lock scopes.
    /// Values are `Arc<HostStatusPayload>` so `build_initial_events`
    /// (SSE handshake + `Lagged` re-sync) can drain the map to a `Vec`
    /// of cheap reference-count bumps under the read lock, then serialize
    /// each payload **outside** the critical section. Writers either
    /// insert a freshly-built `Arc::new(...)` or swap in a new `Arc`
    /// via `Arc::make_mut` for in-place field updates.
    pub last_known_status: Arc<RwLock<HashMap<String, Arc<HostStatusPayload>>>>,
    /// TTL cache for full long-range metric queries (avoids repeated DB scans for same range)
    pub metrics_query_cache: Arc<MetricsQueryCache<MetricsRow>>,
    /// TTL cache for lightweight chart long-range queries.
    pub chart_metrics_query_cache: Arc<MetricsQueryCache<ChartMetricsRow>>,
    /// Per-IP login attempt rate limiter. Broad bucket that catches
    /// scatter-gun brute force attempts varying the username. Default 30 per
    /// 5 min — sized so a small NAT / Cloudflare-tunnel deployment with
    /// several concurrent dashboards does not lock itself out when one user
    /// mistypes a password. See the companion `login_user_rate_limiter`
    /// below for the per-username bucket that catches targeted attempts.
    pub login_rate_limiter: Arc<LoginRateLimiter>,
    /// Per-username login attempt rate limiter. Tighter bucket (default
    /// 10 / 5 min) that catches focused brute force against a single
    /// account. Keyed by the **supplied username**, so an attacker cannot
    /// evade the per-account limit by rotating IPs. Both limiters must
    /// admit the request; either tripping returns 429.
    pub login_user_rate_limiter: Arc<LoginRateLimiter>,
    /// Number of trusted reverse proxies in front of the server.
    /// When 0, X-Forwarded-For is ignored and the peer socket IP is used.
    /// When >0, the Nth IP from the right of X-Forwarded-For is used.
    pub trusted_proxy_count: usize,
    /// Unified "tokens before this instant are invalid" cache keyed by
    /// `user_id`. Fed by both password changes (`users.password_changed_at`)
    /// and explicit revocations (`users.tokens_revoked_at`). The stored
    /// value is the **later** of the two — see `services::auth` for the
    /// verification path.
    pub token_revocation_cutoffs: Arc<RwLock<HashMap<i32, i64>>>,
    /// Single-use opaque ticket store for the SSE handshake.
    /// See `services::sse_ticket` for rationale.
    pub sse_ticket_store: Arc<SseTicketStore>,
    /// Per-IP rate limiter for all API endpoints. More generous than the
    /// login limiter (which protects against brute-force). Prevents any
    /// single IP from overwhelming the server with rapid-fire requests.
    pub api_rate_limiter: Arc<LoginRateLimiter>,
    /// Tighter per-IP limiter for **unauthenticated** endpoints
    /// (`/api/auth/login|setup|status`, `/api/public/status`, `/api/health`).
    /// Without a separate bucket, abusive unauthenticated traffic would eat
    /// into the same budget the authenticated SPA uses for SWR polling +
    /// SSE retry, forcing the authenticated shell to return 429 while the
    /// abuse is ongoing.
    pub public_api_rate_limiter: Arc<LoginRateLimiter>,
    /// Global cap on concurrent SSE connections. Each `/api/stream` stream
    /// holds a `broadcast::Receiver`, a `last_known_status` snapshot, and
    /// an `auth_check` interval — unbounded growth turns one misbehaving
    /// client into a memory exhaustion vector. Controlled by
    /// `MAX_SSE_CONNECTIONS` env var.
    pub sse_connections: Arc<std::sync::atomic::AtomicUsize>,
    /// Upper bound the connection counter is compared against.
    pub max_sse_connections: usize,
    /// Cached view of the `hosts` + `alert_configs` tables used by the
    /// scraper hot path. See `services::hosts_snapshot` for the refresh
    /// protocol (invalidation on mutation handlers + 60 s background tick).
    /// This replaced per-scrape `SELECT * FROM hosts` + `SELECT * FROM alert_configs`
    /// round-trips (Top-10 review finding #10).
    pub hosts_snapshot: SharedHostsSnapshot,
    /// Cached view of the enabled HTTP / Ping monitor sets used by
    /// `monitor_scraper`. Replaces the per-sweep
    /// `SELECT … FROM http_monitors WHERE enabled = 1` + ping equivalent
    /// (Top-10 review #9). Refreshed synchronously on monitor mutation
    /// handlers and every 60 s as a backstop.
    pub monitors_snapshot: SharedMonitorsSnapshot,
}

impl AppState {
    /// Pre-populate last_known_status from the hosts table on startup.
    /// Ensures SSE clients see all configured hosts immediately upon connection.
    pub fn pre_populate_status(&self, hosts: &[HostRow]) {
        let mut lks = self.last_known_status.write().unwrap_or_else(|e| {
            tracing::warn!("⚠️ [Status] RwLock poisoned during pre_populate_status, recovering");
            e.into_inner()
        });
        for host in hosts {
            lks.entry(host.host_key.clone()).or_insert_with(|| {
                Arc::new(HostStatusPayload {
                    host_key: host.host_key.clone(),
                    display_name: host.display_name.clone(),
                    scrape_interval_secs: u64::try_from(host.scrape_interval_secs)
                        .ok()
                        .filter(|secs| *secs > 0)
                        .unwrap_or(self.scrape_interval_secs),
                    is_online: false,
                    last_seen: String::new(),
                    docker_containers: vec![],
                    ports: vec![],
                    disks: vec![],
                    processes: vec![],
                    temperatures: vec![],
                    gpus: vec![],
                    docker_stats: vec![],
                    os_info: host.os_info.clone(),
                    cpu_model: host.cpu_model.clone(),
                    memory_total_mb: host.memory_total_mb,
                    boot_time: host.boot_time,
                    ip_address: host.ip_address.clone(),
                })
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

#[derive(Debug, Clone, Copy)]
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
    /// Load-average rule loaded from alert_configs (metric_type='load').
    /// When present, this takes precedence over `load_threshold` / `load_cooldown_secs`
    /// below, which are carried forward for back-compat with the per-host `hosts.load_threshold`
    /// column.
    pub load: MetricAlertRule,
    /// Aggregate network throughput rule — threshold is bytes/sec across all physical NICs.
    pub network: MetricAlertRule,
    /// Temperature rule — applied to every sensor in the scrape payload.
    pub temperature: MetricAlertRule,
    /// GPU usage rule — applied to every GPU device.
    pub gpu: MetricAlertRule,
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
            load: MetricAlertRule {
                enabled: false,
                threshold: 4.0,
                sustained_secs: 5 * 60,
                cooldown_secs: 300,
            },
            network: MetricAlertRule {
                enabled: false,
                threshold: 500_000_000.0, // 500 MB/s aggregate
                sustained_secs: 5 * 60,
                cooldown_secs: 600,
            },
            temperature: MetricAlertRule {
                enabled: false,
                threshold: 85.0, // °C
                sustained_secs: 2 * 60,
                cooldown_secs: 600,
            },
            gpu: MetricAlertRule {
                enabled: false,
                threshold: 90.0,
                sustained_secs: 5 * 60,
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
    /// Per-interface previous byte counters for rate calculation
    pub network_interface_prev: HashMap<String, (u64, u64, Instant)>,
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
            network_interface_prev: HashMap::new(),
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
    pub network_alerted: bool,
    /// Per-mount-point disk alert state (keyed by mount_point string)
    pub disk_alerted: HashMap<String, bool>,
    /// Per-sensor temperature alert state (keyed by sensor label)
    pub temperature_alerted: HashMap<String, bool>,
    /// Per-GPU alert state (keyed by GPU name or index)
    pub gpu_alerted: HashMap<String, bool>,
    pub last_offline_alert: Option<Instant>,
    pub last_recovery_alert: Option<Instant>,
    pub last_cpu_alert: Option<Instant>,
    pub last_memory_alert: Option<Instant>,
    pub last_load_alert: Option<Instant>,
    pub last_disk_alert: Option<Instant>,
    pub last_network_alert: Option<Instant>,
    pub last_temperature_alert: Option<Instant>,
    pub last_gpu_alert: Option<Instant>,
    pub port_alerted: HashMap<u16, Instant>,
}

impl AlertState {
    pub fn new() -> Self {
        Self {
            offline_alerted: false,
            cpu_alerted: false,
            memory_alerted: false,
            load_alerted: false,
            network_alerted: false,
            disk_alerted: HashMap::new(),
            temperature_alerted: HashMap::new(),
            gpu_alerted: HashMap::new(),
            last_offline_alert: None,
            last_recovery_alert: None,
            last_cpu_alert: None,
            last_memory_alert: None,
            last_load_alert: None,
            last_disk_alert: None,
            last_network_alert: None,
            last_temperature_alert: None,
            last_gpu_alert: None,
            port_alerted: HashMap::new(),
        }
    }
}

// ──────────────────────────────────────────────
// TTL cache for long-range metric queries
// ──────────────────────────────────────────────

struct CacheEntry<T> {
    data: Arc<Vec<T>>,
    inserted_at: Instant,
    weight_bytes: usize,
}

pub trait CacheWeight {
    fn cache_weight_bytes(&self) -> usize;
}

/// Internal mutable state held under the cache's single `RwLock`.
///
/// Pulling `entries` and `total_bytes` into one struct guarantees the
/// two stay in lock-step: every code path that mutates the map also has
/// a `&mut` to the byte counter, so a future contributor cannot
/// accidentally update one without the other (the previous design held
/// each in a separate `RwLock` and silently desynced when the second
/// `.write()` returned `None`).
struct CacheInner<T> {
    entries: HashMap<String, CacheEntry<T>>,
    total_bytes: usize,
}

impl<T> CacheInner<T> {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            total_bytes: 0,
        }
    }

    fn remove_entry(&mut self, key: &str) -> Option<CacheEntry<T>> {
        let entry = self.entries.remove(key)?;
        self.total_bytes = self.total_bytes.saturating_sub(entry.weight_bytes);
        Some(entry)
    }

    fn prune_expired(&mut self, ttl: Duration) {
        let now = Instant::now();
        let total = &mut self.total_bytes;
        self.entries.retain(|_, entry| {
            let keep = now.duration_since(entry.inserted_at) < ttl;
            if !keep {
                *total = total.saturating_sub(entry.weight_bytes);
            }
            keep
        });
    }

    fn remove_with_prefix(&mut self, prefix: &str) {
        let total = &mut self.total_bytes;
        self.entries.retain(|key, entry| {
            let keep = !key.starts_with(prefix);
            if !keep {
                *total = total.saturating_sub(entry.weight_bytes);
            }
            keep
        });
    }
}

/// Simple in-memory TTL cache for rollup/wide time-range metric queries.
///
/// Prevents repeated DB scans when multiple users view the same dashboard range.
/// Entries expire after `ttl` and are lazily evicted on the next `get` or periodic cleanup.
///
/// Bounded by both `max_entries` and `max_bytes` to cap worst-case memory.
/// v0.3.0 grew the per-sample payload (per-core CPU, per-interface network,
/// per-container docker_stats JSON) 3–5×, so count-only caps can still pin
/// multi-MB Vecs. On insert, expired entries are purged first, then
/// oldest-inserted entries are evicted until both caps fit.
pub struct MetricsQueryCache<T> {
    inner: RwLock<CacheInner<T>>,
    ttl: Duration,
    max_entries: usize,
    max_bytes: usize,
}

/// Build a cache key from query parameters.
///
/// Rounds timestamps so near-identical dashboard queries collapse onto a
/// shared cache entry. Keys for ranges ≤ `raw_boundary_secs` use 10 s
/// buckets, ranges within 14 d use 60 s buckets, and wide re-aggregation
/// (> 14 d) uses 300 s buckets. The full-metrics endpoints pass a 6 h
/// raw boundary; the chart endpoint passes 1 h.
pub fn metrics_cache_key(
    host_key: &str,
    start_ts: i64,
    end_ts: i64,
    raw_boundary_secs: i64,
) -> String {
    const ROLLUP_BOUNDARY_SECS: i64 = 14 * 24 * 3600;

    let range = (end_ts - start_ts).max(0);
    let bucket: i64 = if range <= raw_boundary_secs {
        10
    } else if range <= ROLLUP_BOUNDARY_SECS {
        60
    } else {
        300
    };
    let start_rounded = start_ts.div_euclid(bucket) * bucket;
    let end_rounded = (end_ts + bucket - 1).div_euclid(bucket) * bucket;
    format!("{host_key}:{start_rounded}:{end_rounded}")
}

/// Whether a `[start, end]` range should be cached at all. Ranges
/// inside the raw window are excluded because live dashboards already
/// get SWR dedup + SSE live samples and the indexed read is cheap.
pub fn should_cache_metrics_range(start_ts: i64, end_ts: i64, raw_boundary_secs: i64) -> bool {
    (end_ts - start_ts).max(0) > raw_boundary_secs
}

impl<T> MetricsQueryCache<T>
where
    T: CacheWeight,
{
    pub fn new(ttl: Duration, max_entries: usize, max_bytes: usize) -> Self {
        Self {
            inner: RwLock::new(CacheInner::new()),
            ttl,
            max_entries: max_entries.max(1),
            max_bytes: max_bytes.max(1),
        }
    }

    /// Get a cached result if it exists and hasn't expired.
    /// Returns an Arc-wrapped Vec for cheap cloning (atomic ref-count increment only).
    pub fn get(&self, key: &str) -> Option<Arc<Vec<T>>> {
        let inner = self.inner.read().ok()?;
        let entry = inner.entries.get(key)?;
        if entry.inserted_at.elapsed() < self.ttl {
            Some(Arc::clone(&entry.data))
        } else {
            None
        }
    }

    /// Insert a query result into the cache and return the Arc-wrapped data.
    /// Avoids the caller needing to clone the Vec before insertion.
    ///
    /// Enforces `max_entries` and `max_bytes` by first draining expired
    /// rows, then evicting oldest-inserted entries until both caps fit.
    /// Oversized single payloads (`weight > max_bytes`) bypass the cache —
    /// they are still returned to the caller, just not retained.
    pub fn insert(&self, key: String, data: Vec<T>) -> Arc<Vec<T>> {
        let weight_bytes = estimate_vec_weight(&data);
        let arc = Arc::new(data);
        if weight_bytes > self.max_bytes {
            // Bumped from `debug` so an unexpectedly large response that
            // silently bypasses the cache surfaces in default ops logs.
            // Frequent emissions here are the operator's signal to raise
            // METRICS_CACHE_MAX_BYTES or narrow the query window.
            tracing::warn!(
                key = %key,
                weight_bytes,
                max_bytes = self.max_bytes,
                "Skipping oversized metrics query cache entry"
            );
            return arc;
        }

        if let Ok(mut inner) = self.inner.write() {
            inner.prune_expired(self.ttl);
            inner.remove_entry(&key);

            while inner.entries.len() >= self.max_entries
                || inner.total_bytes.saturating_add(weight_bytes) > self.max_bytes
            {
                let Some(oldest_key) = inner
                    .entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.inserted_at)
                    .map(|(k, _)| k.clone())
                else {
                    break;
                };
                inner.remove_entry(&oldest_key);
            }

            inner.entries.insert(
                key,
                CacheEntry {
                    data: Arc::clone(&arc),
                    inserted_at: Instant::now(),
                    weight_bytes,
                },
            );
            inner.total_bytes = inner.total_bytes.saturating_add(weight_bytes);
        }
        arc
    }

    /// Remove every cached query for a host. Cache keys are
    /// `{host_key}:{rounded_start}:{rounded_end}`, so a prefix match is enough.
    pub fn remove_host(&self, host_key: &str) {
        let prefix = format!("{host_key}:");
        if let Ok(mut inner) = self.inner.write() {
            inner.remove_with_prefix(&prefix);
        }
    }

    /// Current number of entries. Only called from tests today; exposed on
    /// the public API so ops can wire it into `/metrics` later without having
    /// to re-plumb visibility.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.read().map(|i| i.entries.len()).unwrap_or(0)
    }

    #[cfg(test)]
    pub fn total_bytes(&self) -> usize {
        self.inner.read().map(|i| i.total_bytes).unwrap_or(0)
    }

    /// Remove expired entries. Called periodically from a background task.
    pub fn evict_expired(&self) {
        if let Ok(mut inner) = self.inner.write() {
            inner.prune_expired(self.ttl);
        }
    }
}

fn estimate_vec_weight<T: CacheWeight>(data: &[T]) -> usize {
    std::mem::size_of_val(data)
        + data
            .iter()
            .map(CacheWeight::cache_weight_bytes)
            .sum::<usize>()
}

fn value_weight(value: &Option<Value>) -> usize {
    match value {
        Some(Value::Null) | None => 0,
        Some(inner) => value_weight_inner(inner),
    }
}

fn value_weight_inner(value: &Value) -> usize {
    match value {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 8,
        Value::String(s) => s.len(),
        Value::Array(items) => {
            std::mem::size_of_val(items.as_slice())
                + items.iter().map(value_weight_inner).sum::<usize>()
        }
        // serde_json::Map (BTreeMap-like) has ~24 B per node on top of the
        // key + value bytes — negligible per entry but compounds on the
        // hot per-core / per-interface JSON we cache.
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| std::mem::size_of::<(String, Value)>() + k.len() + value_weight_inner(v))
            .sum(),
    }
}

impl CacheWeight for MetricsRow {
    fn cache_weight_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.host_key.len()
            + self.display_name.len()
            + value_weight(&self.networks)
            + value_weight(&self.docker_containers)
            + value_weight(&self.ports)
            + value_weight(&self.disks)
            + value_weight(&self.processes)
            + value_weight(&self.temperatures)
            + value_weight(&self.gpus)
            + value_weight(&self.cpu_cores)
            + value_weight(&self.network_interfaces)
            + value_weight(&self.docker_stats)
    }
}

impl CacheWeight for ChartMetricsRow {
    fn cache_weight_bytes(&self) -> usize {
        // `size_of_val(slice)` on each Vec captures the per-element
        // fixed-size component (e.g. `f32`/`f64` fields, mount counts
        // for ChartDiskInfo). The per-iter `+= s.len()` then layers on
        // the heap-tail of each `String`. Without `size_of_val` the
        // weight tracker undercounted Vec contents by `len * size_of<T>`,
        // which on a 30-day chart with dozens of containers compounded
        // to multiple MB of unaccounted RSS.
        std::mem::size_of::<Self>()
            + self.host_key.len()
            + self.display_name.len()
            + std::mem::size_of_val(self.disks.as_slice())
            + self
                .disks
                .iter()
                .map(|d| d.name.len() + d.mount_point.len())
                .sum::<usize>()
            + std::mem::size_of_val(self.temperatures.as_slice())
            + self
                .temperatures
                .iter()
                .map(|t| t.label.len())
                .sum::<usize>()
            + std::mem::size_of_val(self.docker_stats.as_slice())
            + self
                .docker_stats
                .iter()
                .map(|s| s.container_name.len())
                .sum::<usize>()
    }
}

// ──────────────────────────────────────────────
// Login rate limiter (per-IP sliding window)
// ──────────────────────────────────────────────

/// Simple per-IP rate limiter for login attempts.
/// Allows `max_attempts` within `window` duration per IP address.
pub struct LoginRateLimiter {
    attempts: RwLock<HashMap<String, VecDeque<Instant>>>,
    max_attempts: usize,
    window: Duration,
}

impl LoginRateLimiter {
    pub fn new(max_attempts: usize, window: Duration) -> Self {
        Self {
            attempts: RwLock::new(HashMap::new()),
            max_attempts,
            window,
        }
    }

    /// Check if a login attempt from the given IP is allowed.
    /// Returns `Ok(())` if allowed, `Err` with remaining seconds if rate-limited.
    pub fn check(&self, ip: &str) -> Result<(), u64> {
        let mut map = match self.attempts.write() {
            Ok(m) => m,
            Err(_) => {
                tracing::error!(
                    limiter = std::any::type_name::<Self>(),
                    "Rate limiter lock poisoned; failing closed"
                );
                return Err(self.window.as_secs().max(1));
            }
        };
        let now = Instant::now();
        let entry = map.entry(ip.to_string()).or_insert_with(VecDeque::new);

        // Remove expired attempts
        while let Some(front) = entry.front() {
            if now.duration_since(*front) > self.window {
                entry.pop_front();
            } else {
                break;
            }
        }

        if entry.len() >= self.max_attempts {
            // Safety: len() >= max_attempts (>= 1), so the deque is non-empty.
            let oldest = entry
                .front()
                .expect("deque non-empty (guarded by len check)");
            let retry_after = self.window.as_secs() - now.duration_since(*oldest).as_secs();
            return Err(retry_after.max(1));
        }

        entry.push_back(now);
        Ok(())
    }

    /// Remove entries whose all timestamps have expired.
    /// Call periodically from a background task to prevent unbounded HashMap growth.
    pub fn evict_stale(&self) {
        if let Ok(mut map) = self.attempts.write() {
            let now = Instant::now();
            map.retain(|_, deque| {
                // Drain expired timestamps from the front
                while let Some(front) = deque.front() {
                    if now.duration_since(*front) > self.window {
                        deque.pop_front();
                    } else {
                        break;
                    }
                }
                !deque.is_empty()
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl CacheWeight for usize {
        fn cache_weight_bytes(&self) -> usize {
            *self
        }
    }

    #[test]
    fn rate_limiter_allows_within_limit() {
        let limiter = LoginRateLimiter::new(3, Duration::from_secs(60));
        assert!(limiter.check("10.0.0.1").is_ok());
        assert!(limiter.check("10.0.0.1").is_ok());
        assert!(limiter.check("10.0.0.1").is_ok());
    }

    #[test]
    fn rate_limiter_blocks_when_exceeded() {
        let limiter = LoginRateLimiter::new(2, Duration::from_secs(60));
        assert!(limiter.check("10.0.0.1").is_ok());
        assert!(limiter.check("10.0.0.1").is_ok());
        let result = limiter.check("10.0.0.1");
        assert!(result.is_err(), "Third attempt should be rejected");
        let retry_after = result.unwrap_err();
        assert!(
            retry_after >= 1,
            "retry_after should be at least 1 second, got {retry_after}"
        );
    }

    #[test]
    fn rate_limiter_isolates_ips() {
        let limiter = LoginRateLimiter::new(1, Duration::from_secs(60));
        assert!(limiter.check("10.0.0.1").is_ok());
        // Different IP should still be allowed
        assert!(limiter.check("10.0.0.2").is_ok());
        // First IP is now blocked
        assert!(limiter.check("10.0.0.1").is_err());
    }

    #[test]
    fn rate_limiter_fails_closed_on_poison() {
        let limiter = Arc::new(LoginRateLimiter::new(10, Duration::from_secs(60)));
        let limiter_for_thread = Arc::clone(&limiter);

        let _ = std::thread::spawn(move || {
            let _guard = limiter_for_thread.attempts.write().unwrap();
            panic!("poison rate limiter lock");
        })
        .join();

        let result = limiter.check("10.0.0.1");
        assert!(result.is_err(), "poisoned limiter must reject requests");
        assert_eq!(result.unwrap_err(), 60);
    }

    #[test]
    fn rate_limiter_expired_attempts_cleaned_up() {
        // Use a tiny window so attempts expire almost immediately
        let limiter = LoginRateLimiter::new(1, Duration::from_millis(1));
        assert!(limiter.check("10.0.0.1").is_ok());
        // Wait for the window to expire
        std::thread::sleep(Duration::from_millis(5));
        // Should be allowed again because the old attempt expired
        assert!(
            limiter.check("10.0.0.1").is_ok(),
            "Expired attempts should be cleaned up, allowing new ones"
        );
    }

    #[test]
    fn metrics_query_cache_enforces_max_entries() {
        // Long TTL so entries never expire — this test exclusively exercises
        // the capacity-based eviction path.
        let cache = MetricsQueryCache::<usize>::new(Duration::from_secs(600), 3, 1024 * 1024);
        for i in 0..10 {
            cache.insert(format!("k{i}"), vec![]);
        }
        assert_eq!(
            cache.len(),
            3,
            "cache must stay at max_entries under flood insert"
        );
        // The most recent three keys are the ones that survive (oldest-first eviction).
        for i in 7..10 {
            assert!(cache.get(&format!("k{i}")).is_some());
        }
        for i in 0..7 {
            assert!(cache.get(&format!("k{i}")).is_none());
        }
    }

    #[test]
    fn make_key_picks_different_bucket_per_tier() {
        // Pin the dynamic-granularity contract: the three tiers
        // (≤ 6 h / ≤ 14 d / > 14 d) emit distinguishable keys for the same
        // absolute start. Without this, the fixed 300 s bucket regression
        // silently resurfaces — live dashboards would see stale data again
        // (see the comment on `metrics_cache_key`).
        let start: i64 = 1_700_000_000;
        let raw_boundary: i64 = 6 * 3600;
        let k_live = metrics_cache_key("h", start, start + 5 * 60, raw_boundary);
        let k_rollup = metrics_cache_key("h", start, start + 12 * 3600, raw_boundary);
        let k_wide = metrics_cache_key("h", start, start + 30 * 86400, raw_boundary);
        assert_ne!(k_live, k_rollup, "live vs rollup must not collide");
        assert_ne!(k_rollup, k_wide, "rollup vs wide must not collide");

        // Live tier advances one bucket after a 10 s shift (frontend's own
        // live rounding granularity in `api.ts`). Pin the boundary so a
        // regression to a coarser server bucket is caught immediately.
        let k_live_next = metrics_cache_key("h", start + 10, start + 10 + 5 * 60, raw_boundary);
        assert_ne!(
            k_live, k_live_next,
            "10 s shift on live range must cross a bucket boundary"
        );
    }

    #[test]
    fn should_cache_range_excludes_raw_window_only() {
        // The full-metrics endpoint refuses to cache anything ≤ 6 h.
        let full_boundary: i64 = 6 * 3600;
        assert!(!should_cache_metrics_range(0, full_boundary, full_boundary));
        assert!(should_cache_metrics_range(
            0,
            full_boundary + 1,
            full_boundary
        ));

        // The chart endpoint passes its own ~1 h boundary (62 min, see
        // `metrics_repo::CHART_RAW_BOUNDARY_SECS`). This test only pins
        // the boundary semantics (`<= boundary` does not cache, `>` does);
        // the literal value mirrors the constant for readability.
        let chart_boundary: i64 = 62 * 60;
        assert!(!should_cache_metrics_range(
            0,
            chart_boundary,
            chart_boundary
        ));
        assert!(should_cache_metrics_range(
            0,
            chart_boundary + 1,
            chart_boundary
        ));
    }

    #[test]
    fn metrics_query_cache_eviction_prefers_expired_over_fresh() {
        // Short TTL; insert two entries, wait past TTL, insert more up to the
        // cap. Expired entries should be purged first, leaving the fresh ones.
        let cache = MetricsQueryCache::<usize>::new(Duration::from_millis(10), 3, 1024 * 1024);
        cache.insert("old1".into(), vec![]);
        cache.insert("old2".into(), vec![]);
        std::thread::sleep(Duration::from_millis(20));
        cache.insert("fresh1".into(), vec![]);
        cache.insert("fresh2".into(), vec![]);
        cache.insert("fresh3".into(), vec![]);
        assert!(cache.get("old1").is_none());
        assert!(cache.get("old2").is_none());
        assert!(cache.get("fresh1").is_some());
        assert!(cache.get("fresh2").is_some());
        assert!(cache.get("fresh3").is_some());
    }

    #[test]
    fn metrics_query_cache_enforces_byte_budget() {
        let cache = MetricsQueryCache::new(Duration::from_secs(600), 10, 256);
        cache.insert("large1".into(), vec![200usize]);
        cache.insert("large2".into(), vec![200usize]);

        assert!(cache.get("large1").is_none());
        assert!(cache.get("large2").is_some());
        assert!(cache.total_bytes() <= 256 + std::mem::size_of::<usize>());
    }

    #[test]
    fn metrics_query_cache_skips_single_entry_over_byte_budget() {
        let cache = MetricsQueryCache::new(Duration::from_secs(600), 10, 256);
        let returned = cache.insert("oversized".into(), vec![300usize]);

        assert_eq!(*returned, vec![300usize]);
        assert!(cache.get("oversized").is_none());
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.total_bytes(), 0);
    }

    #[test]
    fn metrics_query_cache_removes_entries_by_host_key() {
        let cache = MetricsQueryCache::new(Duration::from_secs(600), 10, 1024 * 1024);
        cache.insert("h1:9101:100:200".into(), vec![1usize]);
        cache.insert("h1:9101:200:300".into(), vec![1usize]);
        cache.insert("h2:9101:100:200".into(), vec![1usize]);

        cache.remove_host("h1:9101");

        assert!(cache.get("h1:9101:100:200").is_none());
        assert!(cache.get("h1:9101:200:300").is_none());
        assert!(cache.get("h2:9101:100:200").is_some());
        assert_eq!(cache.len(), 1);
    }
}
