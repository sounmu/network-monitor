use serde::{Deserialize, Serialize};

use crate::models::agent_metrics::{
    DiskInfo, DockerContainer, GpuInfo, PortStatus, ProcessInfo, TemperatureInfo,
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
    pub is_online: bool,
    pub last_seen: String,
    pub docker_containers: Vec<DockerContainer>,
    pub ports: Vec<PortStatus>,
    pub disks: Vec<DiskInfo>,
    pub processes: Vec<ProcessInfo>,
    pub temperatures: Vec<TemperatureInfo>,
    pub gpus: Vec<GpuInfo>,
}

/// Event variants delivered to SSE handlers via a `tokio::sync::broadcast` channel
#[derive(Clone, Debug)]
pub enum SseBroadcast {
    Metrics(HostMetricsPayload),
    Status(HostStatusPayload),
}
