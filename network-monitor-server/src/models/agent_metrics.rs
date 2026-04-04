use serde::{Deserialize, Serialize};

/// Top-level struct for metric data sent by agents
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct AgentMetrics {
    pub hostname: String,
    pub timestamp: String,
    pub is_online: bool,
    pub system: SystemMetrics,
    /// Cumulative traffic totalled across physical interfaces (virtual/loopback already excluded by the agent)
    #[serde(default)]
    pub network: NetworkTotal,
    #[serde(default)]
    pub load_average: LoadAverage,
    /// Agent sends this field as "docker"; deserialized here as docker_containers
    #[serde(rename = "docker", default)]
    pub docker_containers: Vec<DockerContainer>,
    #[serde(default)]
    pub ports: Vec<PortStatus>,
    /// Agent binary version (e.g. "0.1.0"). Empty string for older agents without this field.
    #[serde(default)]
    pub agent_version: String,
}

/// System resource metrics (CPU, RAM, disk, processes, temperatures, GPUs)
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct SystemMetrics {
    pub cpu_usage_percent: f32,
    pub memory_total_mb: u64,
    pub memory_used_mb: u64,
    pub memory_usage_percent: f32,
    pub disks: Vec<DiskInfo>,
    #[serde(default)]
    pub processes: Vec<ProcessInfo>,
    #[serde(default)]
    pub temperatures: Vec<TemperatureInfo>,
    #[serde(default)]
    pub gpus: Vec<GpuInfo>,
}

/// Per-disk information
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub total_gb: f64,
    pub available_gb: f64,
    pub usage_percent: f32,
}

/// Top process by resource usage
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub memory_mb: u64,
}

/// Temperature sensor reading
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct TemperatureInfo {
    pub label: String,
    pub temperature_c: f32,
}

/// GPU device metrics (NVIDIA, Apple Silicon, or other backends)
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct GpuInfo {
    pub name: String,
    pub gpu_usage_percent: u32,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub temperature_c: u32,
    // New fields — appended at end for bincode compat with agent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_watts: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_mhz: Option<u32>,
}

/// Cumulative traffic totalled across physical interfaces only (virtual/loopback excluded by the agent).
///
/// Default impl: falls back to 0 if the agent is an older version or omits network data.
#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct NetworkTotal {
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
}

/// System load average (1-min, 5-min, 15-min)
#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct LoadAverage {
    pub one_min: f64,
    pub five_min: f64,
    pub fifteen_min: f64,
}

/// Docker container state
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct DockerContainer {
    pub container_name: String,
    pub image: String,
    pub state: String,  // "running", "exited", "dead", etc.
    pub status: String, // human-readable status string, e.g. "Up 2 hours"
}

/// Local port open/closed status
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct PortStatus {
    pub port: u16,
    pub is_open: bool,
}
