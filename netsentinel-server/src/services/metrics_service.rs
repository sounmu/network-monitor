use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use chrono::{SecondsFormat, Utc};

use crate::errors::AppError;
use crate::models::agent_metrics::{AgentMetrics, NetworkTotal};
use crate::models::app_state::{
    AlertConfig, AlertMetricPoint, AppState, HostRecord, MetricAlertRule,
};
use crate::models::sse_payloads::{HostMetricsPayload, HostStatusPayload, NetworkRate};

/// How long to retain in-memory metric history (10 minutes)
const HISTORY_RETENTION_SECS: u64 = 10 * 60;
/// Minimum interval between periodic forced status SSE broadcasts (2 minutes).
/// Used by both `process_metrics` (online path) and `handle_down` (offline path).
pub const STATUS_PERIODIC_INTERVAL_SECS: u64 = 120;

/// Type-safe alert result enum.
/// Uses pattern matching instead of string matching for state transitions,
/// preventing silent bugs caused by message format changes.
pub enum AlertAction {
    CpuOverload {
        hostname: String,
        sustained_mins: u64,
        threshold: f64,
        current: f32,
    },
    CpuRecovery {
        hostname: String,
        current: f32,
    },
    MemoryOverload {
        hostname: String,
        sustained_mins: u64,
        threshold: f64,
        current: f32,
    },
    MemoryRecovery {
        hostname: String,
        current: f32,
    },
    LoadOverload {
        hostname: String,
        load: f64,
        threshold: f64,
    },
    LoadRecovery {
        hostname: String,
        load: f64,
    },
    PortDown {
        hostname: String,
        port: u16,
    },
    PortRecovery {
        hostname: String,
        port: u16,
    },
    DiskOverload {
        hostname: String,
        mount_point: String,
        threshold: f64,
        current: f32,
    },
    DiskRecovery {
        hostname: String,
        mount_point: String,
        current: f32,
    },
}

impl AlertAction {
    /// Returns a short string identifier for this alert type (used for DB logging).
    pub fn alert_type_str(&self) -> &'static str {
        match self {
            Self::CpuOverload { .. } => "cpu_overload",
            Self::CpuRecovery { .. } => "cpu_recovery",
            Self::MemoryOverload { .. } => "memory_overload",
            Self::MemoryRecovery { .. } => "memory_recovery",
            Self::LoadOverload { .. } => "load_overload",
            Self::LoadRecovery { .. } => "load_recovery",
            Self::PortDown { .. } => "port_down",
            Self::PortRecovery { .. } => "port_recovery",
            Self::DiskOverload { .. } => "disk_overload",
            Self::DiskRecovery { .. } => "disk_recovery",
        }
    }

    /// Formats a Discord notification message for this alert action.
    pub fn to_message(&self) -> String {
        match self {
            Self::CpuOverload {
                hostname,
                sustained_mins,
                threshold,
                current,
            } => format!(
                "🔥 **[CPU Overload]** Host `{}` — CPU usage has been above {:.1}% for the past {} minute(s). (current: {:.1}%)",
                hostname, threshold, sustained_mins, current
            ),
            Self::CpuRecovery { hostname, current } => format!(
                "✅ **[CPU Recovery]** Host `{}` — CPU usage has returned to normal. (current: {:.1}%)",
                hostname, current
            ),
            Self::MemoryOverload {
                hostname,
                sustained_mins,
                threshold,
                current,
            } => format!(
                "🔥 **[Memory Overload]** Host `{}` — Memory usage has been above {:.1}% for the past {} minute(s). (current: {:.1}%)",
                hostname, threshold, sustained_mins, current
            ),
            Self::MemoryRecovery { hostname, current } => format!(
                "✅ **[Memory Recovery]** Host `{}` — Memory usage has returned to normal. (current: {:.1}%)",
                hostname, current
            ),
            Self::LoadOverload {
                hostname,
                load,
                threshold,
            } => format!(
                "⚡ **[High Load]** Host `{}` — Load Average (1 min) is {:.2}, exceeding threshold {:.1}!",
                hostname, load, threshold
            ),
            Self::LoadRecovery { hostname, load } => format!(
                "✅ **[Load Recovery]** Host `{}` — Load Average (1 min) has returned to normal at {:.2}.",
                hostname, load
            ),
            Self::PortDown { hostname, port } => format!(
                "🚫 **[Port Down]** Host `{}` — port `{}` is not responding (closed).",
                hostname, port
            ),
            Self::PortRecovery { hostname, port } => format!(
                "✅ **[Port Recovery]** Host `{}` — port `{}` is open again.",
                hostname, port
            ),
            Self::DiskOverload {
                hostname,
                mount_point,
                threshold,
                current,
            } => format!(
                "💾 **[Disk Full]** Host `{}` — disk `{}` usage is {:.1}%, exceeding threshold {:.1}%!",
                hostname, mount_point, current, threshold
            ),
            Self::DiskRecovery {
                hostname,
                mount_point,
                current,
            } => format!(
                "✅ **[Disk Recovery]** Host `{}` — disk `{}` usage has returned to normal. (current: {:.1}%)",
                hostname, mount_point, current
            ),
        }
    }
}

/// Return value of `process_metrics`
pub struct ProcessResult {
    pub log_msg: String,
    /// Payload for `event: metrics` — produced every scrape cycle
    pub metrics_payload: HostMetricsPayload,
    /// Payload for `event: status` — only `Some` when Docker/port state changed,
    /// on the first scrape, or after the 2-minute periodic interval
    pub status_payload: Option<HostStatusPayload>,
}

/// Core business logic for processing scraped metric data.
///
/// Lock minimization strategy:
/// 1) Under write lock: only lightweight AlertMetricPoint push + SSE payload assembly
/// 2) Alert evaluation iterates only alert_history (Copy type) — no heap allocations
/// 3) Discord I/O always happens after the lock is released
#[tracing::instrument(skip(metrics, state, alert_config))]
pub async fn process_metrics(
    metrics: &AgentMetrics,
    target: &str,
    state: &AppState,
    alert_config: &AlertConfig,
) -> Result<ProcessResult, AppError> {
    tracing::debug!(hostname = %metrics.hostname, is_online = %metrics.is_online, "Processing metrics (overview)");
    tracing::trace!(metrics = ?metrics, "Detailed metrics JSON data");

    let http_client = state.http_client.clone();
    let hostname = metrics.hostname.clone();

    // Allocate once outside the lock to avoid redundant .to_string()/.clone() inside it.
    let target_str = target.to_string();
    let server_ts = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

    // ── Lock region: only lightweight data manipulation ──
    // AlertMetricPoint is a Copy type (~20 B), so pushing inside the lock is trivially cheap.
    // Vec clones for HostStatusPayload are deferred until after the lock is released.
    let (alert_actions, history_count, metrics_payload, needs_status) = {
        let mut store = state.store.write().map_err(|e| {
            AppError::Internal(format!("Failed to acquire store write lock: {}", e))
        })?;

        let record = store
            .hosts
            .entry(target_str.clone())
            .or_insert_with(|| HostRecord::new(hostname.clone()));

        record.last_known_hostname.clone_from(&hostname);

        // ── Push lightweight metric point + evict stale entries ──
        let point = AlertMetricPoint {
            received_at: Instant::now(),
            cpu_usage_percent: metrics.system.cpu_usage_percent,
            memory_usage_percent: metrics.system.memory_usage_percent,
        };
        record.push_alert_point(point, Duration::from_secs(HISTORY_RETENTION_SECS));
        let history_count = record.alert_history.len();

        // ── Compute per-second network throughput ──
        let network_rate = compute_network_rate(&metrics.network, &mut record.network_prev);

        // ── Determine if status SSE payload is needed (decision only, no allocation) ──
        let new_hash = compute_status_hash(
            &metrics.docker_containers,
            &metrics.ports,
            &metrics.system.disks,
        );
        let now = Instant::now();
        let periodic_elapsed = record
            .last_status_sent
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(STATUS_PERIODIC_INTERVAL_SECS));
        let hash_changed = record.prev_status_hash != Some(new_hash);

        let needs_status = hash_changed || periodic_elapsed;
        if needs_status {
            record.prev_status_hash = Some(new_hash);
            record.last_status_sent = Some(now);
        }

        // ── Build metrics SSE payload (only scalar copies, no heap allocation) ──
        let metrics_payload = HostMetricsPayload {
            host_key: target_str,
            display_name: hostname.clone(),
            is_online: metrics.is_online,
            cpu_usage_percent: metrics.system.cpu_usage_percent,
            memory_usage_percent: metrics.system.memory_usage_percent,
            load_1min: metrics.load_average.one_min,
            load_5min: metrics.load_average.five_min,
            load_15min: metrics.load_average.fifteen_min,
            network_rate,
            timestamp: server_ts.clone(),
        };

        // ── Evaluate alert conditions (iterates alert_history only, no I/O) ──
        let alert_actions = collect_alerts(record, &hostname, alert_config, metrics);

        tracing::info!(
            target = %target,
            hostname = %hostname,
            count = history_count,
            "📊 [Store] Recorded metrics"
        );

        (alert_actions, history_count, metrics_payload, needs_status)
        // ← RwLockWriteGuard is dropped here, releasing the lock immediately
    };

    // ── Build status payload OUTSIDE the lock (Vec clones happen here, no contention) ──
    let status_payload = if needs_status {
        Some(HostStatusPayload {
            host_key: metrics_payload.host_key.clone(),
            display_name: hostname.clone(),
            is_online: metrics.is_online,
            last_seen: server_ts,
            docker_containers: metrics.docker_containers.clone(),
            ports: metrics.ports.clone(),
            disks: metrics.system.disks.clone(),
            processes: metrics.system.processes.clone(),
            temperatures: metrics.system.temperatures.clone(),
            gpus: metrics.system.gpus.clone(),
        })
    } else {
        None
    };

    // ── Send alerts outside the lock (async) ──
    // Alert delivery can take hundreds of milliseconds — must run after the lock is released.
    for action in &alert_actions {
        let message = action.to_message();
        crate::services::alert_service::send_alert(&http_client, &state.db_pool, &message).await;

        // Log to alert_history (best-effort, don't block on failure)
        if let Err(e) = crate::repositories::alert_history_repo::insert_alert(
            &state.db_pool,
            target,
            action.alert_type_str(),
            &message,
        )
        .await
        {
            tracing::error!(err = ?e, "⚠️ [AlertHistory] Failed to log alert");
        }
    }

    // Update alert state after sending (brief write lock)
    if !alert_actions.is_empty() {
        let mut store = state.store.write().map_err(|e| {
            AppError::Internal(format!("Failed to acquire store write lock: {}", e))
        })?;
        if let Some(record) = store.hosts.get_mut(target) {
            update_alert_state_after_send(record, &alert_actions);
        }
    }

    // DB persistence is deferred — the caller (scraper) collects ProcessResults
    // and batch-inserts all metrics in a single query per scrape cycle.

    Ok(ProcessResult {
        log_msg: format!(
            "Data from {} ({}) processed successfully (history: {})",
            metrics.hostname, target, history_count
        ),
        metrics_payload,
        status_payload,
    })
}

// ──────────────────────────────────────────────
// Network throughput calculation
// ──────────────────────────────────────────────

/// Convert cumulative byte counters reported by the agent into per-second throughput.
///
/// The agent already excludes virtual/loopback interfaces, so the server only needs a delta calculation.
/// - First call (no previous value): reports rate=0; accurate values start from the next cycle.
/// - saturating_sub: prevents underflow if counters reset (e.g. after a reboot).
fn compute_network_rate(
    network: &NetworkTotal,
    prev: &mut Option<(u64, u64, Instant)>,
) -> NetworkRate {
    let now = Instant::now();
    let rate = if let Some((prev_rx, prev_tx, prev_time)) = *prev {
        let elapsed = now.duration_since(prev_time).as_secs_f64();
        if elapsed > 0.0 {
            NetworkRate {
                rx_bytes_per_sec: network.total_rx_bytes.saturating_sub(prev_rx) as f64 / elapsed,
                tx_bytes_per_sec: network.total_tx_bytes.saturating_sub(prev_tx) as f64 / elapsed,
            }
        } else {
            NetworkRate::default()
        }
    } else {
        NetworkRate::default()
    };
    *prev = Some((network.total_rx_bytes, network.total_tx_bytes, now));
    rate
}

// ──────────────────────────────────────────────
// Status change detection
// ──────────────────────────────────────────────

/// Compute a hash of Docker container, port, and disk states.
/// Only fields that indicate a state change are included (name, state, port/open, disk usage rounded to 1%).
fn compute_status_hash(
    containers: &[crate::models::agent_metrics::DockerContainer],
    ports: &[crate::models::agent_metrics::PortStatus],
    disks: &[crate::models::agent_metrics::DiskInfo],
) -> u64 {
    let mut hasher = DefaultHasher::new();
    for c in containers {
        c.container_name.hash(&mut hasher);
        c.state.hash(&mut hasher);
    }
    for p in ports {
        p.port.hash(&mut hasher);
        p.is_open.hash(&mut hasher);
    }
    for d in disks {
        d.mount_point.hash(&mut hasher);
        // Round to 1% to avoid excessive SSE broadcasts from minor fluctuations
        (d.usage_percent as u32).hash(&mut hasher);
    }
    hasher.finish()
}

// ──────────────────────────────────────────────
// Alert collection (called inside the lock, no I/O)
// AlertConfig is passed as a parameter so thresholds and cooldowns can be injected dynamically.
// ──────────────────────────────────────────────

fn collect_alerts(
    record: &HostRecord,
    hostname: &str,
    alert_config: &AlertConfig,
    metrics: &AgentMetrics,
) -> Vec<AlertAction> {
    let mut actions = Vec::new();

    collect_cpu_alerts(record, hostname, &alert_config.cpu, &mut actions);
    collect_memory_alerts(record, hostname, &alert_config.memory, &mut actions);
    collect_load_alerts(record, hostname, alert_config, metrics, &mut actions);
    collect_port_alerts(record, hostname, metrics, &mut actions);
    collect_disk_alerts(record, hostname, &alert_config.disk, metrics, &mut actions);

    actions
}

// ── CPU overload check ───────────────────────

fn collect_cpu_alerts(
    record: &HostRecord,
    hostname: &str,
    rule: &MetricAlertRule,
    actions: &mut Vec<AlertAction>,
) {
    if !rule.enabled || record.alert_history.is_empty() {
        return;
    }

    let sustained = Duration::from_secs(rule.sustained_secs);
    let recent_points: Vec<_> = record
        .alert_history
        .iter()
        .rev()
        .take_while(|p| p.received_at.elapsed() <= sustained)
        .collect();

    if recent_points.len() < 2 {
        return;
    }

    let all_high = recent_points
        .iter()
        .all(|p| p.cpu_usage_percent as f64 > rule.threshold);
    let latest_cpu = record
        .alert_history
        .back()
        .map(|p| p.cpu_usage_percent)
        .unwrap_or(0.0);

    if all_high {
        if !record.alert_state.cpu_alerted
            && cooldown_elapsed(record.alert_state.last_cpu_alert, rule.cooldown_secs)
        {
            actions.push(AlertAction::CpuOverload {
                hostname: hostname.to_string(),
                sustained_mins: rule.sustained_secs / 60,
                threshold: rule.threshold,
                current: latest_cpu,
            });
        }
    } else if record.alert_state.cpu_alerted {
        actions.push(AlertAction::CpuRecovery {
            hostname: hostname.to_string(),
            current: latest_cpu,
        });
    }
}

// ── RAM overload check ───────────────────────

fn collect_memory_alerts(
    record: &HostRecord,
    hostname: &str,
    rule: &MetricAlertRule,
    actions: &mut Vec<AlertAction>,
) {
    if !rule.enabled || record.alert_history.is_empty() {
        return;
    }

    let sustained = Duration::from_secs(rule.sustained_secs);
    let recent_points: Vec<_> = record
        .alert_history
        .iter()
        .rev()
        .take_while(|p| p.received_at.elapsed() <= sustained)
        .collect();

    if recent_points.len() < 2 {
        return;
    }

    let all_high = recent_points
        .iter()
        .all(|p| p.memory_usage_percent as f64 > rule.threshold);
    let latest_mem = record
        .alert_history
        .back()
        .map(|p| p.memory_usage_percent)
        .unwrap_or(0.0);

    if all_high {
        if !record.alert_state.memory_alerted
            && cooldown_elapsed(record.alert_state.last_memory_alert, rule.cooldown_secs)
        {
            actions.push(AlertAction::MemoryOverload {
                hostname: hostname.to_string(),
                sustained_mins: rule.sustained_secs / 60,
                threshold: rule.threshold,
                current: latest_mem,
            });
        }
    } else if record.alert_state.memory_alerted {
        actions.push(AlertAction::MemoryRecovery {
            hostname: hostname.to_string(),
            current: latest_mem,
        });
    }
}

// ── Load average overload check ──────────────

fn collect_load_alerts(
    record: &HostRecord,
    hostname: &str,
    alert_config: &AlertConfig,
    metrics: &AgentMetrics,
    actions: &mut Vec<AlertAction>,
) {
    let load_1min = metrics.load_average.one_min;
    let threshold = alert_config.load_threshold;

    if load_1min > threshold {
        if !record.alert_state.load_alerted
            && cooldown_elapsed(
                record.alert_state.last_load_alert,
                alert_config.load_cooldown_secs,
            )
        {
            actions.push(AlertAction::LoadOverload {
                hostname: hostname.to_string(),
                load: load_1min,
                threshold,
            });
        }
    } else if record.alert_state.load_alerted {
        actions.push(AlertAction::LoadRecovery {
            hostname: hostname.to_string(),
            load: load_1min,
        });
    }
}

// ── Port state check ─────────────────────────

fn collect_port_alerts(
    record: &HostRecord,
    hostname: &str,
    metrics: &AgentMetrics,
    actions: &mut Vec<AlertAction>,
) {
    for port_status in &metrics.ports {
        let port = port_status.port;

        if !port_status.is_open {
            if !record.alert_state.port_alerted.contains_key(&port) {
                actions.push(AlertAction::PortDown {
                    hostname: hostname.to_string(),
                    port,
                });
            }
        } else if record.alert_state.port_alerted.contains_key(&port) {
            actions.push(AlertAction::PortRecovery {
                hostname: hostname.to_string(),
                port,
            });
        }
    }
}

// ── Disk overload check ─────────────────────

fn collect_disk_alerts(
    record: &HostRecord,
    hostname: &str,
    rule: &MetricAlertRule,
    metrics: &AgentMetrics,
    actions: &mut Vec<AlertAction>,
) {
    if !rule.enabled {
        return;
    }

    for disk in &metrics.system.disks {
        let mount = &disk.mount_point;
        let usage = disk.usage_percent;
        let was_alerted = record
            .alert_state
            .disk_alerted
            .get(mount)
            .copied()
            .unwrap_or(false);

        if (usage as f64) > rule.threshold {
            if !was_alerted
                && cooldown_elapsed(record.alert_state.last_disk_alert, rule.cooldown_secs)
            {
                actions.push(AlertAction::DiskOverload {
                    hostname: hostname.to_string(),
                    mount_point: mount.clone(),
                    threshold: rule.threshold,
                    current: usage,
                });
            }
        } else if was_alerted {
            actions.push(AlertAction::DiskRecovery {
                hostname: hostname.to_string(),
                mount_point: mount.clone(),
                current: usage,
            });
        }
    }
}

// ──────────────────────────────────────────────
// Shared utilities
// ──────────────────────────────────────────────

/// Returns true if the cooldown period has elapsed. Cooldown duration is injected for flexibility.
fn cooldown_elapsed(last_alert: Option<Instant>, cooldown_secs: u64) -> bool {
    let cooldown = Duration::from_secs(cooldown_secs);
    match last_alert {
        None => true,
        Some(t) => t.elapsed() >= cooldown,
    }
}

fn update_alert_state_after_send(record: &mut HostRecord, actions: &[AlertAction]) {
    let now = Instant::now();
    for action in actions {
        match action {
            AlertAction::CpuOverload { .. } => {
                record.alert_state.cpu_alerted = true;
                record.alert_state.last_cpu_alert = Some(now);
            }
            AlertAction::CpuRecovery { .. } => {
                record.alert_state.cpu_alerted = false;
            }
            AlertAction::MemoryOverload { .. } => {
                record.alert_state.memory_alerted = true;
                record.alert_state.last_memory_alert = Some(now);
            }
            AlertAction::MemoryRecovery { .. } => {
                record.alert_state.memory_alerted = false;
            }
            AlertAction::LoadOverload { .. } => {
                record.alert_state.load_alerted = true;
                record.alert_state.last_load_alert = Some(now);
            }
            AlertAction::LoadRecovery { .. } => {
                record.alert_state.load_alerted = false;
            }
            AlertAction::PortDown { port, .. } => {
                record.alert_state.port_alerted.insert(*port, now);
            }
            AlertAction::PortRecovery { port, .. } => {
                record.alert_state.port_alerted.remove(port);
            }
            AlertAction::DiskOverload { mount_point, .. } => {
                record
                    .alert_state
                    .disk_alerted
                    .insert(mount_point.clone(), true);
                record.alert_state.last_disk_alert = Some(now);
            }
            AlertAction::DiskRecovery { mount_point, .. } => {
                record.alert_state.disk_alerted.remove(mount_point);
            }
        }
    }
}

// ──────────────────────────────────────────────
// Unit tests
// ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::agent_metrics::{
        AgentMetrics, LoadAverage, NetworkTotal, PortStatus, SystemMetrics,
    };
    use crate::models::app_state::{AlertConfig, AlertMetricPoint, HostRecord, MetricAlertRule};

    const TEST_HOSTNAME: &str = "test-host";

    fn make_metrics(load: f64, cpu: f32, ports: Vec<PortStatus>) -> AgentMetrics {
        AgentMetrics {
            hostname: TEST_HOSTNAME.to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            is_online: true,
            system: SystemMetrics {
                cpu_usage_percent: cpu,
                memory_total_mb: 8000,
                memory_used_mb: 4000,
                memory_usage_percent: 50.0,
                disks: vec![],
                processes: vec![],
                temperatures: vec![],
                gpus: vec![],
            },
            network: NetworkTotal {
                total_rx_bytes: 0,
                total_tx_bytes: 0,
            },
            load_average: LoadAverage {
                one_min: load,
                five_min: 0.0,
                fifteen_min: 0.0,
            },
            docker_containers: vec![],
            ports,
            agent_version: "0.1.0".to_string(),
        }
    }

    fn make_record() -> HostRecord {
        HostRecord::new(TEST_HOSTNAME.to_string())
    }

    fn make_record_with_cpu_history(cpu: f32, count: usize, alerted: bool) -> HostRecord {
        let mut record = make_record();
        record.alert_state.cpu_alerted = alerted;
        for _ in 0..count {
            record.alert_history.push_back(AlertMetricPoint {
                received_at: Instant::now(),
                cpu_usage_percent: cpu,
                memory_usage_percent: 50.0,
            });
        }
        record
    }

    fn make_record_with_memory_history(mem: f32, count: usize, alerted: bool) -> HostRecord {
        let mut record = make_record();
        record.alert_state.memory_alerted = alerted;
        for _ in 0..count {
            record.alert_history.push_back(AlertMetricPoint {
                received_at: Instant::now(),
                cpu_usage_percent: 30.0,
                memory_usage_percent: mem,
            });
        }
        record
    }

    fn default_cpu_rule() -> MetricAlertRule {
        MetricAlertRule {
            enabled: true,
            threshold: 80.0,
            sustained_secs: 5 * 60,
            cooldown_secs: 60,
        }
    }

    fn default_memory_rule() -> MetricAlertRule {
        MetricAlertRule {
            enabled: true,
            threshold: 90.0,
            sustained_secs: 5 * 60,
            cooldown_secs: 60,
        }
    }

    fn default_alert_config() -> AlertConfig {
        AlertConfig::default()
    }

    // ── cooldown_elapsed ─────────────────────────

    #[test]
    fn test_cooldown_elapsed_never_alerted_is_always_ready() {
        assert!(cooldown_elapsed(None, 60));
    }

    #[test]
    fn test_cooldown_elapsed_just_sent_is_not_ready() {
        let just_now = Instant::now();
        assert!(!cooldown_elapsed(Some(just_now), 60));
    }

    // ── compute_status_hash ───────────────────────

    #[test]
    fn test_status_hash_same_input_same_hash() {
        use crate::models::agent_metrics::{DockerContainer, PortStatus};
        let containers = vec![DockerContainer {
            container_name: "nginx".to_string(),
            image: "nginx:latest".to_string(),
            state: "running".to_string(),
            status: "Up 2 hours".to_string(),
        }];
        let ports = vec![PortStatus {
            port: 80,
            is_open: true,
        }];
        let h1 = compute_status_hash(&containers, &ports, &[]);
        let h2 = compute_status_hash(&containers, &ports, &[]);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_status_hash_different_state_different_hash() {
        use crate::models::agent_metrics::{DockerContainer, PortStatus};
        let running = vec![DockerContainer {
            container_name: "nginx".to_string(),
            image: "nginx:latest".to_string(),
            state: "running".to_string(),
            status: "Up 2 hours".to_string(),
        }];
        let exited = vec![DockerContainer {
            container_name: "nginx".to_string(),
            image: "nginx:latest".to_string(),
            state: "exited".to_string(),
            status: "Exited (1) 5 minutes ago".to_string(),
        }];
        let ports = vec![PortStatus {
            port: 80,
            is_open: true,
        }];
        assert_ne!(
            compute_status_hash(&running, &ports, &[]),
            compute_status_hash(&exited, &ports, &[])
        );
    }

    // ── collect_load_alerts ──────────────────────

    #[test]
    fn test_load_alert_fires_above_threshold() {
        let record = make_record();
        let metrics = make_metrics(5.0, 10.0, vec![]);
        let config = default_alert_config();
        let mut actions = Vec::new();
        collect_load_alerts(&record, TEST_HOSTNAME, &config, &metrics, &mut actions);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], AlertAction::LoadOverload { .. }));
    }

    #[test]
    fn test_load_alert_silent_below_threshold() {
        let record = make_record();
        let metrics = make_metrics(1.0, 10.0, vec![]);
        let config = default_alert_config();
        let mut actions = Vec::new();
        collect_load_alerts(&record, TEST_HOSTNAME, &config, &metrics, &mut actions);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_load_alert_no_duplicate_when_already_alerted() {
        let mut record = make_record();
        record.alert_state.load_alerted = true;
        let metrics = make_metrics(5.0, 10.0, vec![]);
        let config = default_alert_config();
        let mut actions = Vec::new();
        collect_load_alerts(&record, TEST_HOSTNAME, &config, &metrics, &mut actions);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_load_alert_recovery_fires_when_was_alerted() {
        let mut record = make_record();
        record.alert_state.load_alerted = true;
        let metrics = make_metrics(1.0, 10.0, vec![]);
        let config = default_alert_config();
        let mut actions = Vec::new();
        collect_load_alerts(&record, TEST_HOSTNAME, &config, &metrics, &mut actions);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], AlertAction::LoadRecovery { .. }));
    }

    // ── collect_cpu_alerts ───────────────────────

    #[test]
    fn test_cpu_alert_fires_when_sustained_high() {
        let record = make_record_with_cpu_history(90.0, 3, false);
        let rule = default_cpu_rule();
        let mut actions = Vec::new();
        collect_cpu_alerts(&record, TEST_HOSTNAME, &rule, &mut actions);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], AlertAction::CpuOverload { .. }));
    }

    #[test]
    fn test_cpu_alert_silent_when_cpu_normal() {
        let record = make_record_with_cpu_history(50.0, 3, false);
        let rule = default_cpu_rule();
        let mut actions = Vec::new();
        collect_cpu_alerts(&record, TEST_HOSTNAME, &rule, &mut actions);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_cpu_alert_no_duplicate_when_already_alerted() {
        let record = make_record_with_cpu_history(90.0, 3, true);
        let rule = default_cpu_rule();
        let mut actions = Vec::new();
        collect_cpu_alerts(&record, TEST_HOSTNAME, &rule, &mut actions);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_cpu_recovery_fires_when_was_alerted() {
        let record = make_record_with_cpu_history(50.0, 3, true);
        let rule = default_cpu_rule();
        let mut actions = Vec::new();
        collect_cpu_alerts(&record, TEST_HOSTNAME, &rule, &mut actions);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], AlertAction::CpuRecovery { .. }));
    }

    #[test]
    fn test_cpu_alert_silent_with_too_few_history() {
        let record = make_record_with_cpu_history(90.0, 1, false);
        let rule = default_cpu_rule();
        let mut actions = Vec::new();
        collect_cpu_alerts(&record, TEST_HOSTNAME, &rule, &mut actions);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_cpu_alert_silent_with_empty_history() {
        let record = make_record();
        let rule = default_cpu_rule();
        let mut actions = Vec::new();
        collect_cpu_alerts(&record, TEST_HOSTNAME, &rule, &mut actions);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_cpu_alert_respects_disabled_rule() {
        let record = make_record_with_cpu_history(90.0, 3, false);
        let mut rule = default_cpu_rule();
        rule.enabled = false;
        let mut actions = Vec::new();
        collect_cpu_alerts(&record, TEST_HOSTNAME, &rule, &mut actions);
        assert!(
            actions.is_empty(),
            "A disabled rule must not generate alerts"
        );
    }

    // ── collect_memory_alerts ────────────────────

    #[test]
    fn test_memory_alert_fires_when_sustained_high() {
        let record = make_record_with_memory_history(95.0, 3, false);
        let rule = default_memory_rule();
        let mut actions = Vec::new();
        collect_memory_alerts(&record, TEST_HOSTNAME, &rule, &mut actions);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], AlertAction::MemoryOverload { .. }));
    }

    #[test]
    fn test_memory_alert_silent_when_normal() {
        let record = make_record_with_memory_history(50.0, 3, false);
        let rule = default_memory_rule();
        let mut actions = Vec::new();
        collect_memory_alerts(&record, TEST_HOSTNAME, &rule, &mut actions);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_memory_recovery_fires_when_was_alerted() {
        let record = make_record_with_memory_history(50.0, 3, true);
        let rule = default_memory_rule();
        let mut actions = Vec::new();
        collect_memory_alerts(&record, TEST_HOSTNAME, &rule, &mut actions);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], AlertAction::MemoryRecovery { .. }));
    }

    #[test]
    fn test_memory_alert_respects_disabled_rule() {
        let record = make_record_with_memory_history(95.0, 3, false);
        let mut rule = default_memory_rule();
        rule.enabled = false;
        let mut actions = Vec::new();
        collect_memory_alerts(&record, TEST_HOSTNAME, &rule, &mut actions);
        assert!(
            actions.is_empty(),
            "A disabled rule must not generate alerts"
        );
    }

    // ── collect_port_alerts ──────────────────────

    #[test]
    fn test_port_down_fires_first_time() {
        let record = make_record();
        let metrics = make_metrics(
            1.0,
            10.0,
            vec![PortStatus {
                port: 80,
                is_open: false,
            }],
        );
        let mut actions = Vec::new();
        collect_port_alerts(&record, TEST_HOSTNAME, &metrics, &mut actions);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], AlertAction::PortDown { port: 80, .. }));
    }

    #[test]
    fn test_port_down_no_duplicate_alert() {
        let mut record = make_record();
        record.alert_state.port_alerted.insert(80, Instant::now());
        let metrics = make_metrics(
            1.0,
            10.0,
            vec![PortStatus {
                port: 80,
                is_open: false,
            }],
        );
        let mut actions = Vec::new();
        collect_port_alerts(&record, TEST_HOSTNAME, &metrics, &mut actions);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_port_recovery_fires_when_was_down() {
        let mut record = make_record();
        record.alert_state.port_alerted.insert(8080, Instant::now());
        let metrics = make_metrics(
            1.0,
            10.0,
            vec![PortStatus {
                port: 8080,
                is_open: true,
            }],
        );
        let mut actions = Vec::new();
        collect_port_alerts(&record, TEST_HOSTNAME, &metrics, &mut actions);
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions[0],
            AlertAction::PortRecovery { port: 8080, .. }
        ));
    }

    #[test]
    fn test_port_open_silent_when_always_open() {
        let record = make_record();
        let metrics = make_metrics(
            1.0,
            10.0,
            vec![PortStatus {
                port: 443,
                is_open: true,
            }],
        );
        let mut actions = Vec::new();
        collect_port_alerts(&record, TEST_HOSTNAME, &metrics, &mut actions);
        assert!(actions.is_empty());
    }

    // ── update_alert_state_after_send ────────────

    #[test]
    fn test_state_update_cpu_overload() {
        let mut record = make_record();
        let actions = vec![AlertAction::CpuOverload {
            hostname: "test".to_string(),
            sustained_mins: 5,
            threshold: 80.0,
            current: 90.0,
        }];
        update_alert_state_after_send(&mut record, &actions);
        assert!(record.alert_state.cpu_alerted);
        assert!(record.alert_state.last_cpu_alert.is_some());
    }

    #[test]
    fn test_state_update_cpu_recovery() {
        let mut record = make_record();
        record.alert_state.cpu_alerted = true;
        let actions = vec![AlertAction::CpuRecovery {
            hostname: "test".to_string(),
            current: 50.0,
        }];
        update_alert_state_after_send(&mut record, &actions);
        assert!(!record.alert_state.cpu_alerted);
    }

    #[test]
    fn test_state_update_memory_overload() {
        let mut record = make_record();
        let actions = vec![AlertAction::MemoryOverload {
            hostname: "test".to_string(),
            sustained_mins: 5,
            threshold: 90.0,
            current: 95.0,
        }];
        update_alert_state_after_send(&mut record, &actions);
        assert!(record.alert_state.memory_alerted);
        assert!(record.alert_state.last_memory_alert.is_some());
    }

    #[test]
    fn test_state_update_memory_recovery() {
        let mut record = make_record();
        record.alert_state.memory_alerted = true;
        let actions = vec![AlertAction::MemoryRecovery {
            hostname: "test".to_string(),
            current: 50.0,
        }];
        update_alert_state_after_send(&mut record, &actions);
        assert!(!record.alert_state.memory_alerted);
    }

    #[test]
    fn test_state_update_port_down_inserts_entry() {
        let mut record = make_record();
        let actions = vec![AlertAction::PortDown {
            hostname: "test-host".to_string(),
            port: 80,
        }];
        update_alert_state_after_send(&mut record, &actions);
        assert!(record.alert_state.port_alerted.contains_key(&80));
    }

    #[test]
    fn test_state_update_port_recovery_removes_entry() {
        let mut record = make_record();
        record.alert_state.port_alerted.insert(443, Instant::now());
        let actions = vec![AlertAction::PortRecovery {
            hostname: "test-host".to_string(),
            port: 443,
        }];
        update_alert_state_after_send(&mut record, &actions);
        assert!(!record.alert_state.port_alerted.contains_key(&443));
    }
}
