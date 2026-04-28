use bincode::Options as _;
use serde::{Deserialize, Serialize};

const MAX_AGENT_PAYLOAD_BYTES: u64 = 10 * 1024 * 1024;

fn bincode_options() -> impl bincode::Options {
    bincode::DefaultOptions::new()
        .with_limit(MAX_AGENT_PAYLOAD_BYTES)
        .with_fixint_encoding()
        .allow_trailing_bytes()
}

/// Static system information returned by the agent's `GET /system-info` endpoint.
/// Fetched on reconnection and every 24 hours.
#[derive(Deserialize, Debug, Clone)]
pub struct SystemInfoResponse {
    pub os: String,
    pub cpu_model: String,
    pub memory_total_mb: u64,
    pub boot_time: u64,
    pub ip_address: String,
}

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
    /// Per-core CPU usage percentages (index = core index)
    #[serde(default)]
    pub cpu_cores: Vec<f32>,
    /// Per-interface network traffic (physical interfaces only)
    #[serde(default)]
    pub network_interfaces: Vec<NetworkInterfaceInfo>,
    /// Per-container resource metrics
    #[serde(default)]
    pub docker_stats: Vec<DockerContainerStats>,
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

/// Per-disk information (capacity + I/O throughput)
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub total_gb: f64,
    pub available_gb: f64,
    pub usage_percent: f32,
    #[serde(default)]
    pub read_bytes_per_sec: f64,
    #[serde(default)]
    pub write_bytes_per_sec: f64,
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

/// Cumulative traffic totals + bandwidth across physical interfaces.
///
/// `total_*_bytes` are cumulative kernel counters — useful for alerting
/// on daily totals or computing long-window averages. `*_bytes_per_sec`
/// is the instantaneous rate as measured *by the agent* between its
/// previous and current scrape — matches how `DiskInfo.read_bytes_per_sec`
/// already works so the "Network Bandwidth" graph is a true rate, not
/// a counter the frontend has to differentiate.
///
/// Rate fields are optional on the wire. `rate_fields_present` is server-only
/// metadata set by `deserialize_agent_metrics()` so a real 0 B/s from a new
/// agent is distinguishable from "old agent omitted the rate fields".
#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct NetworkTotal {
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
    #[serde(default)]
    pub rx_bytes_per_sec: f64,
    #[serde(default)]
    pub tx_bytes_per_sec: f64,
    #[serde(skip)]
    pub rate_fields_present: bool,
}

#[derive(Default, Deserialize, Serialize)]
struct LegacyNetworkTotal {
    total_rx_bytes: u64,
    total_tx_bytes: u64,
}

#[derive(Deserialize, Serialize)]
struct LegacyAgentMetrics {
    hostname: String,
    timestamp: String,
    is_online: bool,
    system: SystemMetrics,
    #[serde(default)]
    network: LegacyNetworkTotal,
    #[serde(default)]
    load_average: LoadAverage,
    #[serde(rename = "docker", default)]
    docker_containers: Vec<DockerContainer>,
    #[serde(default)]
    ports: Vec<PortStatus>,
    #[serde(default)]
    agent_version: String,
    #[serde(default)]
    cpu_cores: Vec<f32>,
    #[serde(default)]
    network_interfaces: Vec<NetworkInterfaceInfo>,
    #[serde(default)]
    docker_stats: Vec<DockerContainerStats>,
}

impl From<LegacyNetworkTotal> for NetworkTotal {
    fn from(network: LegacyNetworkTotal) -> Self {
        Self {
            total_rx_bytes: network.total_rx_bytes,
            total_tx_bytes: network.total_tx_bytes,
            rx_bytes_per_sec: 0.0,
            tx_bytes_per_sec: 0.0,
            rate_fields_present: false,
        }
    }
}

impl From<LegacyAgentMetrics> for AgentMetrics {
    fn from(metrics: LegacyAgentMetrics) -> Self {
        Self {
            hostname: metrics.hostname,
            timestamp: metrics.timestamp,
            is_online: metrics.is_online,
            system: metrics.system,
            network: metrics.network.into(),
            load_average: metrics.load_average,
            docker_containers: metrics.docker_containers,
            ports: metrics.ports,
            agent_version: metrics.agent_version,
            cpu_cores: metrics.cpu_cores,
            network_interfaces: metrics.network_interfaces,
            docker_stats: metrics.docker_stats,
        }
    }
}

/// Decode the bincode agent payload while preserving one-way compatibility:
/// old agents that emitted only cumulative network counters still work with
/// new servers, while new-agent rate fields are marked as present even when
/// the actual rate is 0 B/s.
pub fn deserialize_agent_metrics(bytes: &[u8]) -> Result<AgentMetrics, bincode::Error> {
    if bytes.len() > MAX_AGENT_PAYLOAD_BYTES as usize {
        return Err(Box::new(bincode::ErrorKind::SizeLimit));
    }

    match bincode_options().deserialize::<AgentMetrics>(bytes) {
        Ok(mut metrics) => {
            metrics.network.rate_fields_present = true;
            Ok(metrics)
        }
        Err(new_err) => match bincode_options().deserialize::<LegacyAgentMetrics>(bytes) {
            Ok(metrics) => {
                crate::services::metrics_service::record_legacy_fallback_used();
                Ok(metrics.into())
            }
            Err(_) => Err(new_err),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system_metrics() -> SystemMetrics {
        SystemMetrics {
            cpu_usage_percent: 12.5,
            memory_total_mb: 8192,
            memory_used_mb: 4096,
            memory_usage_percent: 50.0,
            disks: vec![],
            processes: vec![],
            temperatures: vec![],
            gpus: vec![],
        }
    }

    #[test]
    fn deserialize_agent_metrics_accepts_legacy_network_payload() {
        let legacy = LegacyAgentMetrics {
            hostname: "legacy-box".into(),
            timestamp: "2026-04-21T00:00:00Z".into(),
            is_online: true,
            system: system_metrics(),
            network: LegacyNetworkTotal {
                total_rx_bytes: 100,
                total_tx_bytes: 200,
            },
            load_average: LoadAverage {
                one_min: 1.0,
                five_min: 2.0,
                fifteen_min: 3.0,
            },
            docker_containers: vec![],
            ports: vec![],
            agent_version: "0.4.0".into(),
            cpu_cores: vec![12.5],
            network_interfaces: vec![],
            docker_stats: vec![],
        };

        let bytes = bincode_options().serialize(&legacy).unwrap();
        let decoded = deserialize_agent_metrics(&bytes).unwrap();

        assert_eq!(decoded.network.total_rx_bytes, 100);
        assert_eq!(decoded.network.total_tx_bytes, 200);
        assert_eq!(decoded.network.rx_bytes_per_sec, 0.0);
        assert_eq!(decoded.network.tx_bytes_per_sec, 0.0);
        assert!(!decoded.network.rate_fields_present);
        assert_eq!(decoded.load_average.one_min, 1.0);
        assert_eq!(decoded.agent_version, "0.4.0");
    }

    #[test]
    fn deserialize_agent_metrics_marks_new_zero_rate_as_present() {
        let metrics = AgentMetrics {
            hostname: "new-box".into(),
            timestamp: "2026-04-21T00:00:00Z".into(),
            is_online: true,
            system: system_metrics(),
            network: NetworkTotal {
                total_rx_bytes: 100,
                total_tx_bytes: 200,
                rx_bytes_per_sec: 0.0,
                tx_bytes_per_sec: 0.0,
                rate_fields_present: false,
            },
            load_average: LoadAverage {
                one_min: 1.0,
                five_min: 2.0,
                fifteen_min: 3.0,
            },
            docker_containers: vec![],
            ports: vec![],
            agent_version: "0.5.0".into(),
            cpu_cores: vec![12.5],
            network_interfaces: vec![],
            docker_stats: vec![],
        };

        let bytes = bincode_options().serialize(&metrics).unwrap();
        let decoded = deserialize_agent_metrics(&bytes).unwrap();

        assert_eq!(decoded.network.rx_bytes_per_sec, 0.0);
        assert_eq!(decoded.network.tx_bytes_per_sec, 0.0);
        assert!(decoded.network.rate_fields_present);
        assert_eq!(decoded.load_average.fifteen_min, 3.0);
        assert_eq!(decoded.agent_version, "0.5.0");
    }

    #[test]
    fn deserialize_agent_metrics_rejects_oversized_payload() {
        let bytes = vec![0_u8; (MAX_AGENT_PAYLOAD_BYTES as usize) + 1];
        assert!(deserialize_agent_metrics(&bytes).is_err());
    }
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

/// Per-interface network traffic (cumulative bytes)
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct NetworkInterfaceInfo {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// Per-container resource usage snapshot
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct DockerContainerStats {
    pub container_name: String,
    pub cpu_percent: f32,
    pub memory_usage_mb: u64,
    pub memory_limit_mb: u64,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
}

/// Local port open/closed status
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct PortStatus {
    pub port: u16,
    pub is_open: bool,
}
