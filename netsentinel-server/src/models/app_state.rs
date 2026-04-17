use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::models::sse_payloads::{HostStatusPayload, SseBroadcast};
use crate::repositories::hosts_repo::HostRow;
use crate::repositories::metrics_repo::MetricsRow;
use crate::services::hosts_snapshot::SharedHostsSnapshot;
use crate::services::sse_ticket::SseTicketStore;
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
    /// Cache of the most recently sent per-host status payload.
    ///
    /// Uses `std::sync::RwLock` (not `tokio::sync::RwLock`) deliberately:
    /// lock scopes are micro-duration data shuffles with **no `.await` inside**,
    /// so the lower per-access overhead of std RwLock beats tokio's cooperative
    /// scheduling cost. Do not add `.await` calls inside lock scopes.
    pub last_known_status: Arc<RwLock<HashMap<String, HostStatusPayload>>>,
    /// TTL cache for long-range metric queries (avoids repeated DB scans for same range)
    pub metrics_query_cache: Arc<MetricsQueryCache>,
    /// Per-IP login attempt rate limiter
    pub login_rate_limiter: Arc<LoginRateLimiter>,
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
    /// Cached view of the `hosts` + `alert_configs` tables used by the
    /// scraper hot path. See `services::hosts_snapshot` for the refresh
    /// protocol (invalidation on mutation handlers + 60 s background tick).
    /// This replaced per-scrape `SELECT * FROM hosts` + `SELECT * FROM alert_configs`
    /// round-trips (Top-10 review finding #10).
    pub hosts_snapshot: SharedHostsSnapshot,
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
                    docker_stats: vec![],
                    os_info: host.os_info.clone(),
                    cpu_model: host.cpu_model.clone(),
                    memory_total_mb: host.memory_total_mb,
                    boot_time: host.boot_time,
                    ip_address: host.ip_address.clone(),
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
    data: Arc<Vec<MetricsRow>>,
    inserted_at: Instant,
}

/// Simple in-memory TTL cache for time-range metric queries.
///
/// Prevents repeated DB scans when multiple users view the same dashboard range.
/// Entries expire after `ttl` and are lazily evicted on the next `get` or periodic cleanup.
///
/// Bounded by `max_entries` to cap worst-case memory: v0.3.0 grew the per-sample
/// payload (per-core CPU, per-interface network, per-container docker_stats JSONB)
/// 3–5×, so a previously-cheap unbounded cache now pins multi-MB Vecs and could
/// trivially hit hundreds of MB under concurrent dashboard load within one TTL
/// window. On insert, oldest-inserted entries are evicted first once the cap is hit.
pub struct MetricsQueryCache {
    entries: RwLock<HashMap<String, CacheEntry>>,
    ttl: Duration,
    max_entries: usize,
}

impl MetricsQueryCache {
    pub fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            ttl,
            max_entries: max_entries.max(1),
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
    /// Returns an Arc-wrapped Vec for cheap cloning (atomic ref-count increment only).
    pub fn get(&self, key: &str) -> Option<Arc<Vec<MetricsRow>>> {
        let entries = self.entries.read().ok()?;
        let entry = entries.get(key)?;
        if entry.inserted_at.elapsed() < self.ttl {
            Some(Arc::clone(&entry.data))
        } else {
            None
        }
    }

    /// Insert a query result into the cache and return the Arc-wrapped data.
    /// Avoids the caller needing to clone the Vec before insertion.
    ///
    /// Enforces `max_entries` by first draining expired rows, then — if still at
    /// capacity — evicting the single oldest-inserted entry. O(n) scan only
    /// fires when over capacity, so the common path stays cheap.
    pub fn insert(&self, key: String, data: Vec<MetricsRow>) -> Arc<Vec<MetricsRow>> {
        let arc = Arc::new(data);
        if let Ok(mut entries) = self.entries.write() {
            if entries.len() >= self.max_entries {
                let now = Instant::now();
                entries.retain(|_, entry| now.duration_since(entry.inserted_at) < self.ttl);
            }
            if entries.len() >= self.max_entries
                && let Some(oldest_key) = entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.inserted_at)
                    .map(|(k, _)| k.clone())
            {
                entries.remove(&oldest_key);
            }
            entries.insert(
                key,
                CacheEntry {
                    data: Arc::clone(&arc),
                    inserted_at: Instant::now(),
                },
            );
        }
        arc
    }

    /// Current number of entries. Only called from tests today; exposed on
    /// the public API so ops can wire it into `/metrics` later without having
    /// to re-plumb visibility.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.read().map(|e| e.len()).unwrap_or(0)
    }

    /// Remove expired entries. Called periodically from a background task.
    pub fn evict_expired(&self) {
        if let Ok(mut entries) = self.entries.write() {
            entries.retain(|_, entry| entry.inserted_at.elapsed() < self.ttl);
        }
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
            Err(_) => return Ok(()), // On lock poisoning, allow the attempt
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
        let cache = MetricsQueryCache::new(Duration::from_secs(600), 3);
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
    fn metrics_query_cache_eviction_prefers_expired_over_fresh() {
        // Short TTL; insert two entries, wait past TTL, insert more up to the
        // cap. Expired entries should be purged first, leaving the fresh ones.
        let cache = MetricsQueryCache::new(Duration::from_millis(10), 3);
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
}
