use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::models::agent_metrics::{
    DiskInfo, DockerContainer, DockerContainerStats, GpuInfo, PortStatus, ProcessInfo,
    TemperatureInfo,
};

/// Network throughput per second — computed server-side as a delta of cumulative byte counters.
///
/// Stored as a single aggregate value instead of a per-interface array:
/// - The agent already sums physical interfaces before sending, so per-interface breakdown is unnecessary.
/// - Reduces SSE payload size.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NetworkRate {
    pub rx_bytes_per_sec: f64,
    pub tx_bytes_per_sec: f64,
    /// Cumulative counters mirrored from the agent's NetworkTotal so live
    /// SSE rows match the REST MetricsRow.networks shape.
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
}

/// Per-interface network throughput (bytes/sec), computed server-side as a delta.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NetworkInterfaceRate {
    pub name: String,
    pub rx_bytes_per_sec: f64,
    pub tx_bytes_per_sec: f64,
}

/// `event: metrics` payload — dynamic data (CPU, memory, network rate, etc.) sent every scrape cycle
#[derive(Serialize, Clone, Debug)]
pub struct HostMetricsPayload {
    /// Target-URL-based unique identifier — prevents collisions when multiple agents share the same hostname
    pub host_key: String,
    /// Agent-reported hostname — used for UI display only
    pub display_name: String,
    pub is_online: bool,
    pub cpu_usage_percent: f32,
    pub memory_usage_percent: f32,
    pub load_1min: f64,
    pub load_5min: f64,
    pub load_15min: f64,
    /// Aggregate throughput across all physical interfaces (bytes/sec)
    pub network_rate: NetworkRate,
    /// Per-core CPU usage percentages
    pub cpu_cores: Vec<f32>,
    /// Per-interface throughput (bytes/sec)
    pub network_interface_rates: Vec<NetworkInterfaceRate>,
    /// Per-disk usage + I/O throughput (sent every cycle for real-time charts)
    pub disks: Vec<DiskInfo>,
    /// Temperature sensor readings
    pub temperatures: Vec<TemperatureInfo>,
    /// Per-container resource usage (CPU%, memory)
    pub docker_stats: Vec<DockerContainerStats>,
    pub timestamp: String,
}

/// `event: status` payload — semi-static data (Docker containers, port states, etc.)
/// Sent immediately on client connection and re-sent on state change or periodically.
#[derive(Serialize, Clone, Debug)]
pub struct HostStatusPayload {
    /// Target-URL-based unique identifier — prevents hostname collisions
    pub host_key: String,
    /// Agent-reported hostname — used for UI display only
    pub display_name: String,
    /// Effective scrape cadence for this host (seconds).
    pub scrape_interval_secs: u64,
    pub is_online: bool,
    pub last_seen: String,
    pub docker_containers: Vec<DockerContainer>,
    pub ports: Vec<PortStatus>,
    pub disks: Vec<DiskInfo>,
    pub processes: Vec<ProcessInfo>,
    pub temperatures: Vec<TemperatureInfo>,
    pub gpus: Vec<GpuInfo>,
    pub docker_stats: Vec<DockerContainerStats>,
    // ── Static system info (fetched on reconnection + every 24h) ──
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_info: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_total_mb: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boot_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_address: Option<String>,
}

/// Event variants delivered to SSE handlers via a `tokio::sync::broadcast` channel.
///
/// Payloads are wrapped in `Arc` so the `broadcast::Sender` can hand each
/// subscriber a cheap reference-count bump instead of a full `HostMetricsPayload`
/// / `HostStatusPayload` clone per receiver. `HostStatusPayload` alone carries
/// five sizeable `Vec`s (docker containers, ports, disks, processes, etc.) —
/// with N connected SSE clients the pre-Arc shape was O(N × payload) allocation
/// per scrape tick.
#[derive(Clone, Debug)]
pub enum SseBroadcast {
    Metrics(Arc<HostMetricsPayload>),
    Status(Arc<HostStatusPayload>),
}
