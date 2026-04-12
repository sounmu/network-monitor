//! Wire-format data structures for `/metrics` responses.
//!
//! These structs are serialised with bincode (see the agent ↔ server protocol).
//! Field order matters — new fields MUST be appended at the end with
//! `#[serde(default)]` on the server side for backward compatibility.

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub(crate) struct MetricsQuery {
    pub ports: Option<String>,
    pub containers: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct AgentMetrics {
    pub hostname: String,
    pub timestamp: String,
    pub is_online: bool,
    pub system: SystemMetrics,
    /// Aggregate traffic across physical interfaces (virtual/loopback excluded)
    pub network: NetworkTotal,
    pub load_average: LoadAverage,
    pub docker: Vec<DockerContainer>,
    pub ports: Vec<PortStatus>,
    /// Agent binary version (from Cargo.toml at build time)
    pub agent_version: String,
}

#[derive(Serialize)]
pub(crate) struct SystemMetrics {
    pub cpu_usage_percent: f32,
    pub memory_total_mb: u64,
    pub memory_used_mb: u64,
    pub memory_usage_percent: f32,
    pub disks: Vec<DiskInfo>,
    pub processes: Vec<ProcessInfo>,
    pub temperatures: Vec<TemperatureInfo>,
    pub gpus: Vec<GpuInfo>,
}

#[derive(Serialize)]
pub(crate) struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub total_gb: f64,
    pub available_gb: f64,
    pub usage_percent: f32,
}

#[derive(Serialize)]
pub(crate) struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub memory_mb: u64,
}

#[derive(Serialize)]
pub(crate) struct TemperatureInfo {
    pub label: String,
    pub temperature_c: f32,
}

#[derive(Serialize)]
pub(crate) struct GpuInfo {
    pub name: String,
    pub gpu_usage_percent: u32,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub temperature_c: u32,
    // New fields — appended at end for bincode compat with server
    pub power_watts: Option<f32>,
    pub frequency_mhz: Option<u32>,
}

/// Physical-interface traffic totals (cumulative bytes after agent-side filtering).
///
/// Sent as a single aggregate struct rather than a per-interface array because:
/// - It reduces the payload size sent to the server.
/// - The server needs no duplicate filtering logic.
/// - Monitoring aggregate throughput is sufficient for this use case.
#[derive(Serialize, Default)]
pub(crate) struct NetworkTotal {
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
}

#[derive(Serialize)]
pub(crate) struct LoadAverage {
    pub one_min: f64,
    pub five_min: f64,
    pub fifteen_min: f64,
}

#[derive(Serialize, Clone)]
pub(crate) struct DockerContainer {
    pub container_name: String,
    pub image: String,
    pub state: String,
    pub status: String,
}

#[derive(Serialize)]
pub(crate) struct PortStatus {
    pub port: u16,
    pub is_open: bool,
}

/// Intermediate bundle returned by `sysinfo_collector::collect_sysinfo`.
/// Separated from `AgentMetrics` so the handler can assemble the final
/// response from multiple parallel sources.
pub(crate) struct SysinfoResult {
    pub cpu_usage: f32,
    pub memory_total_mb: u64,
    pub memory_used_mb: u64,
    pub memory_usage_percent: f32,
    pub disks: Vec<DiskInfo>,
    pub processes: Vec<ProcessInfo>,
    pub temperatures: Vec<TemperatureInfo>,
    pub gpus: Vec<GpuInfo>,
    pub network: NetworkTotal,
    pub load_average: LoadAverage,
}
