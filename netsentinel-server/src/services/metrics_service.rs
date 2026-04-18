use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use chrono::{SecondsFormat, Utc};

use crate::errors::AppError;
use crate::models::agent_metrics::{AgentMetrics, NetworkInterfaceInfo, NetworkTotal};
use crate::models::app_state::{
    AlertConfig, AlertMetricPoint, AppState, HostRecord, MetricAlertRule,
};
use crate::models::sse_payloads::{
    HostMetricsPayload, HostStatusPayload, NetworkInterfaceRate, NetworkRate,
};

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
    NetworkOverload {
        hostname: String,
        bytes_per_sec: f64,
        threshold: f64,
    },
    NetworkRecovery {
        hostname: String,
        bytes_per_sec: f64,
    },
    TemperatureOverload {
        hostname: String,
        sensor: String,
        threshold: f64,
        current: f32,
    },
    TemperatureRecovery {
        hostname: String,
        sensor: String,
        current: f32,
    },
    GpuOverload {
        hostname: String,
        gpu: String,
        threshold: f64,
        current: f32,
    },
    GpuRecovery {
        hostname: String,
        gpu: String,
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
            Self::NetworkOverload { .. } => "network_overload",
            Self::NetworkRecovery { .. } => "network_recovery",
            Self::TemperatureOverload { .. } => "temperature_overload",
            Self::TemperatureRecovery { .. } => "temperature_recovery",
            Self::GpuOverload { .. } => "gpu_overload",
            Self::GpuRecovery { .. } => "gpu_recovery",
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
            Self::NetworkOverload {
                hostname,
                bytes_per_sec,
                threshold,
            } => format!(
                "📡 **[Network Overload]** Host `{}` — aggregate network throughput is {:.1} MB/s (threshold {:.1} MB/s).",
                hostname,
                bytes_per_sec / 1_000_000.0,
                threshold / 1_000_000.0
            ),
            Self::NetworkRecovery {
                hostname,
                bytes_per_sec,
            } => format!(
                "✅ **[Network Recovery]** Host `{}` — network throughput has returned to normal. (current: {:.1} MB/s)",
                hostname,
                bytes_per_sec / 1_000_000.0
            ),
            Self::TemperatureOverload {
                hostname,
                sensor,
                threshold,
                current,
            } => format!(
                "🌡️ **[Temperature Overload]** Host `{}` — sensor `{}` reads {:.1}°C (threshold {:.1}°C).",
                hostname, sensor, current, threshold
            ),
            Self::TemperatureRecovery {
                hostname,
                sensor,
                current,
            } => format!(
                "✅ **[Temperature Recovery]** Host `{}` — sensor `{}` returned to {:.1}°C.",
                hostname, sensor, current
            ),
            Self::GpuOverload {
                hostname,
                gpu,
                threshold,
                current,
            } => format!(
                "🎮 **[GPU Overload]** Host `{}` — GPU `{}` usage is {:.1}% (threshold {:.1}%).",
                hostname, gpu, current, threshold
            ),
            Self::GpuRecovery {
                hostname,
                gpu,
                current,
            } => format!(
                "✅ **[GPU Recovery]** Host `{}` — GPU `{}` returned to {:.1}%.",
                hostname, gpu, current
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
    scrape_interval_secs: u64,
) -> Result<ProcessResult, AppError> {
    tracing::debug!(hostname = %metrics.hostname, is_online = %metrics.is_online, "Processing metrics (overview)");
    tracing::trace!(metrics = ?metrics, "Detailed metrics JSON data");

    let http_client = state.http_client.clone();
    let hostname = metrics.hostname.clone();

    // Allocate once outside the lock to avoid redundant .to_string()/.clone() inside it.
    let target_str = target.to_string();
    // Prefer the agent-provided timestamp when it parses as RFC 3339; fall
    // back to the server's own wall-clock otherwise. Agents now emit UTC
    // RFC 3339 (v0.3.3); older agents that still send a KST wall-clock
    // string silently fall through to the server fallback — the old contract
    // dropped the field entirely, so no behavior regresses.
    let server_ts = chrono::DateTime::parse_from_rfc3339(&metrics.timestamp)
        .map(|dt| {
            dt.with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Millis, true)
        })
        .unwrap_or_else(|_| Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true));

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

        // ── Compute per-second network throughput (aggregate + per-interface) ──
        let network_rate = compute_network_rate(&metrics.network, &mut record.network_prev);
        let network_interface_rates = compute_interface_rates(
            &metrics.network_interfaces,
            &mut record.network_interface_prev,
        );

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

        // ── Build metrics SSE payload ──
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
            cpu_cores: metrics.cpu_cores.clone(),
            network_interface_rates,
            disks: metrics.system.disks.clone(),
            temperatures: metrics.system.temperatures.clone(),
            docker_stats: metrics.docker_stats.clone(),
            timestamp: server_ts.clone(),
        };

        // ── Evaluate alert conditions (iterates alert_history only, no I/O) ──
        let alert_actions = collect_alerts(
            record,
            &hostname,
            alert_config,
            metrics,
            &metrics_payload.network_rate,
        );

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
        // Carry forward system info from existing status (populated by fetch_and_store_system_info)
        let prev_sys = state.last_known_status.read().ok().and_then(|lks| {
            lks.get(target).map(|s| {
                (
                    s.os_info.clone(),
                    s.cpu_model.clone(),
                    s.memory_total_mb,
                    s.boot_time,
                    s.ip_address.clone(),
                )
            })
        });
        let (os_info, cpu_model, memory_total_mb, boot_time, ip_address) =
            prev_sys.unwrap_or((None, None, None, None, None));

        Some(HostStatusPayload {
            host_key: metrics_payload.host_key.clone(),
            display_name: hostname.clone(),
            scrape_interval_secs,
            is_online: metrics.is_online,
            last_seen: server_ts,
            docker_containers: metrics.docker_containers.clone(),
            ports: metrics.ports.clone(),
            disks: metrics.system.disks.clone(),
            processes: metrics.system.processes.clone(),
            temperatures: metrics.system.temperatures.clone(),
            gpus: metrics.system.gpus.clone(),
            docker_stats: metrics.docker_stats.clone(),
            os_info,
            cpu_model,
            memory_total_mb,
            boot_time,
            ip_address,
        })
    } else {
        None
    };

    // Update alert state immediately (brief write lock) before spawning delivery
    if !alert_actions.is_empty() {
        let mut store = state.store.write().map_err(|e| {
            AppError::Internal(format!("Failed to acquire store write lock: {}", e))
        })?;
        if let Some(record) = store.hosts.get_mut(target) {
            update_alert_state_after_send(record, &alert_actions);
        }
    }

    // ── Fire-and-forget alert delivery (spawned, non-blocking) ──
    // Alert delivery can take hundreds of milliseconds — spawn it so it doesn't
    // block the scraper from processing the next host.
    if !alert_actions.is_empty() {
        let messages: Vec<(String, String)> = alert_actions
            .iter()
            .map(|a| (a.alert_type_str().to_string(), a.to_message()))
            .collect();
        let http_client = http_client.clone();
        let db_pool = state.db_pool.clone();
        let target_owned = target.to_string();
        tokio::spawn(async move {
            for (alert_type, message) in &messages {
                crate::services::alert_service::send_alert(&http_client, &db_pool, message).await;
                if let Err(e) = crate::repositories::alert_history_repo::insert_alert(
                    &db_pool,
                    &target_owned,
                    alert_type,
                    message,
                )
                .await
                {
                    tracing::error!(err = ?e, "⚠️ [AlertHistory] Failed to log alert");
                }
            }
        });
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

/// Compute bytes-per-second rate from cumulative counters (shared logic).
/// Returns (rx_rate, tx_rate). First call (no previous) returns (0, 0).
/// `saturating_sub` prevents underflow if counters reset (e.g. after reboot).
fn delta_rate(
    current_rx: u64,
    current_tx: u64,
    prev: Option<&(u64, u64, Instant)>,
    now: Instant,
) -> (f64, f64) {
    if let Some(&(prev_rx, prev_tx, prev_time)) = prev {
        let elapsed = now.duration_since(prev_time).as_secs_f64();
        if elapsed > 0.0 {
            return (
                current_rx.saturating_sub(prev_rx) as f64 / elapsed,
                current_tx.saturating_sub(prev_tx) as f64 / elapsed,
            );
        }
    }
    (0.0, 0.0)
}

/// Convert cumulative aggregate byte counters into per-second throughput.
fn compute_network_rate(
    network: &NetworkTotal,
    prev: &mut Option<(u64, u64, Instant)>,
) -> NetworkRate {
    let now = Instant::now();
    let (rx, tx) = delta_rate(
        network.total_rx_bytes,
        network.total_tx_bytes,
        prev.as_ref(),
        now,
    );
    *prev = Some((network.total_rx_bytes, network.total_tx_bytes, now));
    NetworkRate {
        rx_bytes_per_sec: rx,
        tx_bytes_per_sec: tx,
    }
}

/// Convert per-interface cumulative byte counters into per-second rates.
/// Prunes stale entries for interfaces no longer reported by the agent.
fn compute_interface_rates(
    interfaces: &[NetworkInterfaceInfo],
    prev_map: &mut std::collections::HashMap<String, (u64, u64, Instant)>,
) -> Vec<NetworkInterfaceRate> {
    let now = Instant::now();
    let rates: Vec<NetworkInterfaceRate> = interfaces
        .iter()
        .map(|iface| {
            let (rx, tx) = delta_rate(
                iface.rx_bytes,
                iface.tx_bytes,
                prev_map.get(&iface.name),
                now,
            );
            prev_map.insert(iface.name.clone(), (iface.rx_bytes, iface.tx_bytes, now));
            NetworkInterfaceRate {
                name: iface.name.clone(),
                rx_bytes_per_sec: rx,
                tx_bytes_per_sec: tx,
            }
        })
        .collect();
    // Prune stale entries (removed interfaces, e.g. Docker veth teardown)
    prev_map.retain(|name, _| interfaces.iter().any(|i| i.name == *name));
    rates
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
    network_rate: &NetworkRate,
) -> Vec<AlertAction> {
    let mut actions = Vec::new();

    collect_cpu_alerts(record, hostname, &alert_config.cpu, &mut actions);
    collect_memory_alerts(record, hostname, &alert_config.memory, &mut actions);
    collect_load_alerts(record, hostname, alert_config, metrics, &mut actions);
    collect_port_alerts(record, hostname, metrics, &mut actions);
    collect_disk_alerts(record, hostname, &alert_config.disk, metrics, &mut actions);
    collect_network_alerts(
        record,
        hostname,
        &alert_config.network,
        network_rate,
        &mut actions,
    );
    collect_temperature_alerts(
        record,
        hostname,
        &alert_config.temperature,
        metrics,
        &mut actions,
    );
    collect_gpu_alerts(record, hostname, &alert_config.gpu, metrics, &mut actions);

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

// ── Network throughput check ────────────────

fn collect_network_alerts(
    record: &HostRecord,
    hostname: &str,
    rule: &MetricAlertRule,
    rate: &NetworkRate,
    actions: &mut Vec<AlertAction>,
) {
    if !rule.enabled {
        return;
    }

    let aggregate = rate.rx_bytes_per_sec + rate.tx_bytes_per_sec;
    if aggregate > rule.threshold {
        if !record.alert_state.network_alerted
            && cooldown_elapsed(record.alert_state.last_network_alert, rule.cooldown_secs)
        {
            actions.push(AlertAction::NetworkOverload {
                hostname: hostname.to_string(),
                bytes_per_sec: aggregate,
                threshold: rule.threshold,
            });
        }
    } else if record.alert_state.network_alerted {
        actions.push(AlertAction::NetworkRecovery {
            hostname: hostname.to_string(),
            bytes_per_sec: aggregate,
        });
    }
}

// ── Temperature check (per sensor) ──────────

fn collect_temperature_alerts(
    record: &HostRecord,
    hostname: &str,
    rule: &MetricAlertRule,
    metrics: &AgentMetrics,
    actions: &mut Vec<AlertAction>,
) {
    if !rule.enabled {
        return;
    }

    for sensor in &metrics.system.temperatures {
        let label = &sensor.label;
        let current = sensor.temperature_c;
        let was_alerted = record
            .alert_state
            .temperature_alerted
            .get(label)
            .copied()
            .unwrap_or(false);

        if (current as f64) > rule.threshold {
            if !was_alerted
                && cooldown_elapsed(
                    record.alert_state.last_temperature_alert,
                    rule.cooldown_secs,
                )
            {
                actions.push(AlertAction::TemperatureOverload {
                    hostname: hostname.to_string(),
                    sensor: label.clone(),
                    threshold: rule.threshold,
                    current,
                });
            }
        } else if was_alerted {
            actions.push(AlertAction::TemperatureRecovery {
                hostname: hostname.to_string(),
                sensor: label.clone(),
                current,
            });
        }
    }
}

// ── GPU check (per device) ──────────────────

fn collect_gpu_alerts(
    record: &HostRecord,
    hostname: &str,
    rule: &MetricAlertRule,
    metrics: &AgentMetrics,
    actions: &mut Vec<AlertAction>,
) {
    if !rule.enabled {
        return;
    }

    for gpu in &metrics.system.gpus {
        let name = &gpu.name;
        let current = gpu.gpu_usage_percent as f32;
        let was_alerted = record
            .alert_state
            .gpu_alerted
            .get(name)
            .copied()
            .unwrap_or(false);

        if (current as f64) > rule.threshold {
            if !was_alerted
                && cooldown_elapsed(record.alert_state.last_gpu_alert, rule.cooldown_secs)
            {
                actions.push(AlertAction::GpuOverload {
                    hostname: hostname.to_string(),
                    gpu: name.clone(),
                    threshold: rule.threshold,
                    current,
                });
            }
        } else if was_alerted {
            actions.push(AlertAction::GpuRecovery {
                hostname: hostname.to_string(),
                gpu: name.clone(),
                current,
            });
        }
    }
}

// ──────────────────────────────────────────────
// Shared utilities
// ──────────────────────────────────────────────

/// Returns true if the cooldown period has elapsed. Cooldown duration is injected for flexibility.
fn cooldown_elapsed(last_alert: Option<Instant>, cooldown_secs: u64) -> bool {
    last_alert.is_none_or(|t| t.elapsed() >= Duration::from_secs(cooldown_secs))
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
            AlertAction::NetworkOverload { .. } => {
                record.alert_state.network_alerted = true;
                record.alert_state.last_network_alert = Some(now);
            }
            AlertAction::NetworkRecovery { .. } => {
                record.alert_state.network_alerted = false;
            }
            AlertAction::TemperatureOverload { sensor, .. } => {
                record
                    .alert_state
                    .temperature_alerted
                    .insert(sensor.clone(), true);
                record.alert_state.last_temperature_alert = Some(now);
            }
            AlertAction::TemperatureRecovery { sensor, .. } => {
                record.alert_state.temperature_alerted.remove(sensor);
            }
            AlertAction::GpuOverload { gpu, .. } => {
                record.alert_state.gpu_alerted.insert(gpu.clone(), true);
                record.alert_state.last_gpu_alert = Some(now);
            }
            AlertAction::GpuRecovery { gpu, .. } => {
                record.alert_state.gpu_alerted.remove(gpu);
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
            cpu_cores: vec![],
            network_interfaces: vec![],
            docker_stats: vec![],
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

    // ── compute_network_rate ────────────────────

    #[test]
    fn test_network_rate_first_call_returns_zero() {
        let net = NetworkTotal {
            total_rx_bytes: 1000,
            total_tx_bytes: 2000,
        };
        let mut prev = None;
        let rate = compute_network_rate(&net, &mut prev);
        assert_eq!(rate.rx_bytes_per_sec, 0.0);
        assert_eq!(rate.tx_bytes_per_sec, 0.0);
        assert!(prev.is_some());
    }

    #[test]
    fn test_network_rate_second_call_computes_delta() {
        let mut prev = Some((1000u64, 2000u64, Instant::now() - Duration::from_secs(1)));
        let net = NetworkTotal {
            total_rx_bytes: 2000,
            total_tx_bytes: 4000,
        };
        let rate = compute_network_rate(&net, &mut prev);
        // 1000 bytes in ~1 second
        assert!(rate.rx_bytes_per_sec > 900.0 && rate.rx_bytes_per_sec < 1100.0);
        assert!(rate.tx_bytes_per_sec > 1900.0 && rate.tx_bytes_per_sec < 2100.0);
    }

    #[test]
    fn test_network_rate_counter_reset_saturating() {
        // Simulates counter reset after reboot
        let mut prev = Some((5000u64, 5000u64, Instant::now() - Duration::from_secs(1)));
        let net = NetworkTotal {
            total_rx_bytes: 100,
            total_tx_bytes: 100,
        };
        let rate = compute_network_rate(&net, &mut prev);
        // saturating_sub: 100 - 5000 = 0
        assert_eq!(rate.rx_bytes_per_sec, 0.0);
        assert_eq!(rate.tx_bytes_per_sec, 0.0);
    }

    // ── compute_interface_rates ─────────────────

    #[test]
    fn test_interface_rates_first_call_returns_zero() {
        let interfaces = vec![NetworkInterfaceInfo {
            name: "eth0".to_string(),
            rx_bytes: 1000,
            tx_bytes: 2000,
        }];
        let mut prev_map = std::collections::HashMap::new();
        let rates = compute_interface_rates(&interfaces, &mut prev_map);
        assert_eq!(rates.len(), 1);
        assert_eq!(rates[0].name, "eth0");
        assert_eq!(rates[0].rx_bytes_per_sec, 0.0);
        assert!(prev_map.contains_key("eth0"));
    }

    #[test]
    fn test_interface_rates_delta_computation() {
        let mut prev_map = std::collections::HashMap::new();
        prev_map.insert(
            "eth0".to_string(),
            (1000u64, 2000u64, Instant::now() - Duration::from_secs(1)),
        );
        let interfaces = vec![NetworkInterfaceInfo {
            name: "eth0".to_string(),
            rx_bytes: 2000,
            tx_bytes: 4000,
        }];
        let rates = compute_interface_rates(&interfaces, &mut prev_map);
        assert!(rates[0].rx_bytes_per_sec > 900.0);
        assert!(rates[0].tx_bytes_per_sec > 1900.0);
    }

    // ── collect_disk_alerts ─────────────────────

    #[test]
    fn test_disk_alert_fires_above_threshold() {
        use crate::models::agent_metrics::DiskInfo;
        let record = make_record();
        let rule = MetricAlertRule {
            enabled: true,
            threshold: 90.0,
            sustained_secs: 0,
            cooldown_secs: 300,
        };
        let mut metrics = make_metrics(1.0, 10.0, vec![]);
        metrics.system.disks = vec![DiskInfo {
            name: "sda1".to_string(),
            mount_point: "/".to_string(),
            total_gb: 100.0,
            available_gb: 5.0,
            usage_percent: 95.0,
            read_bytes_per_sec: 0.0,
            write_bytes_per_sec: 0.0,
        }];
        let mut actions = Vec::new();
        collect_disk_alerts(&record, TEST_HOSTNAME, &rule, &metrics, &mut actions);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], AlertAction::DiskOverload { .. }));
    }

    #[test]
    fn test_disk_alert_silent_below_threshold() {
        use crate::models::agent_metrics::DiskInfo;
        let record = make_record();
        let rule = MetricAlertRule {
            enabled: true,
            threshold: 90.0,
            sustained_secs: 0,
            cooldown_secs: 300,
        };
        let mut metrics = make_metrics(1.0, 10.0, vec![]);
        metrics.system.disks = vec![DiskInfo {
            name: "sda1".to_string(),
            mount_point: "/".to_string(),
            total_gb: 100.0,
            available_gb: 50.0,
            usage_percent: 50.0,
            read_bytes_per_sec: 0.0,
            write_bytes_per_sec: 0.0,
        }];
        let mut actions = Vec::new();
        collect_disk_alerts(&record, TEST_HOSTNAME, &rule, &metrics, &mut actions);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_disk_alert_no_duplicate() {
        use crate::models::agent_metrics::DiskInfo;
        let mut record = make_record();
        record
            .alert_state
            .disk_alerted
            .insert("/".to_string(), true);
        let rule = MetricAlertRule {
            enabled: true,
            threshold: 90.0,
            sustained_secs: 0,
            cooldown_secs: 300,
        };
        let mut metrics = make_metrics(1.0, 10.0, vec![]);
        metrics.system.disks = vec![DiskInfo {
            name: "sda1".to_string(),
            mount_point: "/".to_string(),
            total_gb: 100.0,
            available_gb: 5.0,
            usage_percent: 95.0,
            read_bytes_per_sec: 0.0,
            write_bytes_per_sec: 0.0,
        }];
        let mut actions = Vec::new();
        collect_disk_alerts(&record, TEST_HOSTNAME, &rule, &metrics, &mut actions);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_disk_recovery_fires() {
        use crate::models::agent_metrics::DiskInfo;
        let mut record = make_record();
        record
            .alert_state
            .disk_alerted
            .insert("/".to_string(), true);
        let rule = MetricAlertRule {
            enabled: true,
            threshold: 90.0,
            sustained_secs: 0,
            cooldown_secs: 300,
        };
        let mut metrics = make_metrics(1.0, 10.0, vec![]);
        metrics.system.disks = vec![DiskInfo {
            name: "sda1".to_string(),
            mount_point: "/".to_string(),
            total_gb: 100.0,
            available_gb: 50.0,
            usage_percent: 50.0,
            read_bytes_per_sec: 0.0,
            write_bytes_per_sec: 0.0,
        }];
        let mut actions = Vec::new();
        collect_disk_alerts(&record, TEST_HOSTNAME, &rule, &metrics, &mut actions);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], AlertAction::DiskRecovery { .. }));
    }

    #[test]
    fn test_disk_alert_disabled_rule() {
        use crate::models::agent_metrics::DiskInfo;
        let record = make_record();
        let rule = MetricAlertRule {
            enabled: false,
            threshold: 90.0,
            sustained_secs: 0,
            cooldown_secs: 300,
        };
        let mut metrics = make_metrics(1.0, 10.0, vec![]);
        metrics.system.disks = vec![DiskInfo {
            name: "sda1".to_string(),
            mount_point: "/".to_string(),
            total_gb: 100.0,
            available_gb: 5.0,
            usage_percent: 95.0,
            read_bytes_per_sec: 0.0,
            write_bytes_per_sec: 0.0,
        }];
        let mut actions = Vec::new();
        collect_disk_alerts(&record, TEST_HOSTNAME, &rule, &metrics, &mut actions);
        assert!(actions.is_empty());
    }

    // ── Network / Temperature / GPU ──────────────

    fn enabled_rule(threshold: f64) -> MetricAlertRule {
        MetricAlertRule {
            enabled: true,
            threshold,
            sustained_secs: 0,
            cooldown_secs: 0,
        }
    }

    #[test]
    fn test_network_alert_fires_when_rate_exceeds_threshold() {
        let record = make_record();
        let rule = enabled_rule(100.0); // 100 B/s
        let rate = NetworkRate {
            rx_bytes_per_sec: 80.0,
            tx_bytes_per_sec: 80.0,
        };
        let mut actions = Vec::new();
        collect_network_alerts(&record, TEST_HOSTNAME, &rule, &rate, &mut actions);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], AlertAction::NetworkOverload { .. }));
    }

    #[test]
    fn test_network_alert_silent_below_threshold() {
        let record = make_record();
        let rule = enabled_rule(1_000_000.0);
        let rate = NetworkRate {
            rx_bytes_per_sec: 10.0,
            tx_bytes_per_sec: 10.0,
        };
        let mut actions = Vec::new();
        collect_network_alerts(&record, TEST_HOSTNAME, &rule, &rate, &mut actions);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_network_alert_recovery_fires_when_was_alerted() {
        let mut record = make_record();
        record.alert_state.network_alerted = true;
        let rule = enabled_rule(1_000_000.0);
        let rate = NetworkRate {
            rx_bytes_per_sec: 10.0,
            tx_bytes_per_sec: 10.0,
        };
        let mut actions = Vec::new();
        collect_network_alerts(&record, TEST_HOSTNAME, &rule, &rate, &mut actions);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], AlertAction::NetworkRecovery { .. }));
    }

    #[test]
    fn test_temperature_alert_fires_per_sensor() {
        use crate::models::agent_metrics::TemperatureInfo;
        let record = make_record();
        let rule = enabled_rule(80.0);
        let mut metrics = make_metrics(1.0, 10.0, vec![]);
        metrics.system.temperatures = vec![
            TemperatureInfo {
                label: "cpu".to_string(),
                temperature_c: 90.0,
            },
            TemperatureInfo {
                label: "gpu".to_string(),
                temperature_c: 50.0,
            },
        ];
        let mut actions = Vec::new();
        collect_temperature_alerts(&record, TEST_HOSTNAME, &rule, &metrics, &mut actions);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            AlertAction::TemperatureOverload { sensor, .. } => assert_eq!(sensor, "cpu"),
            _ => panic!("expected TemperatureOverload"),
        }
    }

    #[test]
    fn test_temperature_alert_disabled_rule() {
        use crate::models::agent_metrics::TemperatureInfo;
        let record = make_record();
        let rule = MetricAlertRule {
            enabled: false,
            ..enabled_rule(50.0)
        };
        let mut metrics = make_metrics(1.0, 10.0, vec![]);
        metrics.system.temperatures = vec![TemperatureInfo {
            label: "cpu".to_string(),
            temperature_c: 100.0,
        }];
        let mut actions = Vec::new();
        collect_temperature_alerts(&record, TEST_HOSTNAME, &rule, &metrics, &mut actions);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_temperature_alert_recovery_per_sensor() {
        use crate::models::agent_metrics::TemperatureInfo;
        let mut record = make_record();
        record
            .alert_state
            .temperature_alerted
            .insert("cpu".to_string(), true);
        let rule = enabled_rule(80.0);
        let mut metrics = make_metrics(1.0, 10.0, vec![]);
        metrics.system.temperatures = vec![TemperatureInfo {
            label: "cpu".to_string(),
            temperature_c: 40.0,
        }];
        let mut actions = Vec::new();
        collect_temperature_alerts(&record, TEST_HOSTNAME, &rule, &metrics, &mut actions);
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions[0],
            AlertAction::TemperatureRecovery { .. }
        ));
    }

    #[test]
    fn test_gpu_alert_fires_per_device() {
        use crate::models::agent_metrics::GpuInfo;
        let record = make_record();
        let rule = enabled_rule(90.0);
        let mut metrics = make_metrics(1.0, 10.0, vec![]);
        metrics.system.gpus = vec![GpuInfo {
            name: "RTX 4090".to_string(),
            gpu_usage_percent: 95,
            memory_used_mb: 0,
            memory_total_mb: 0,
            temperature_c: 0,
            power_watts: None,
            frequency_mhz: None,
        }];
        let mut actions = Vec::new();
        collect_gpu_alerts(&record, TEST_HOSTNAME, &rule, &metrics, &mut actions);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], AlertAction::GpuOverload { .. }));
    }

    #[test]
    fn test_gpu_alert_recovery_clears_state() {
        use crate::models::agent_metrics::GpuInfo;
        let mut record = make_record();
        record
            .alert_state
            .gpu_alerted
            .insert("RTX 4090".to_string(), true);
        let rule = enabled_rule(90.0);
        let mut metrics = make_metrics(1.0, 10.0, vec![]);
        metrics.system.gpus = vec![GpuInfo {
            name: "RTX 4090".to_string(),
            gpu_usage_percent: 40,
            memory_used_mb: 0,
            memory_total_mb: 0,
            temperature_c: 0,
            power_watts: None,
            frequency_mhz: None,
        }];
        let mut actions = Vec::new();
        collect_gpu_alerts(&record, TEST_HOSTNAME, &rule, &metrics, &mut actions);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], AlertAction::GpuRecovery { .. }));
    }
}
