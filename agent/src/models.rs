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
    /// Per-core CPU usage percentages (index = core index)
    pub cpu_cores: Vec<f32>,
    /// Per-interface network traffic (physical interfaces only)
    pub network_interfaces: Vec<NetworkInterfaceInfo>,
    /// Per-container resource metrics (CPU, memory, network)
    pub docker_stats: Vec<DockerContainerStats>,
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

#[derive(Serialize, Clone)]
pub(crate) struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub total_gb: f64,
    pub available_gb: f64,
    pub usage_percent: f32,
    pub read_bytes_per_sec: f64,
    pub write_bytes_per_sec: f64,
}

#[derive(Serialize, Clone)]
pub(crate) struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub memory_mb: u64,
}

#[derive(Serialize, Clone)]
pub(crate) struct TemperatureInfo {
    pub label: String,
    pub temperature_c: f32,
}

#[derive(Serialize, Clone)]
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

/// Physical-interface traffic totals + bandwidth (after agent-side filtering).
///
/// Sent as a single aggregate struct rather than a per-interface array because:
/// - It reduces the payload size sent to the server.
/// - The server needs no duplicate filtering logic.
/// - Monitoring aggregate throughput is sufficient for this use case.
///
/// `total_*_bytes` are cumulative counters; `*_bytes_per_sec` is the rate
/// between the previous and current collection cycles — computed here for
/// symmetry with `DiskInfo.read_bytes_per_sec` so "Network Bandwidth" in
/// the UI is an actual rate, not a counter the frontend has to differentiate.
/// Rate fields are appended at the end of the struct. New servers decode both
/// the old 2-field shape and this 4-field shape; older servers must be upgraded
/// before using agents that emit these fields.
#[derive(Serialize, Default, Clone)]
pub(crate) struct NetworkTotal {
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
    pub rx_bytes_per_sec: f64,
    pub tx_bytes_per_sec: f64,
}

#[derive(Serialize, Clone)]
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

/// Per-interface network traffic (cumulative bytes).
#[derive(Serialize, Clone)]
pub(crate) struct NetworkInterfaceInfo {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// Per-container resource usage snapshot.
#[derive(Serialize, Clone)]
pub(crate) struct DockerContainerStats {
    pub container_name: String,
    pub cpu_percent: f32,
    pub memory_usage_mb: u64,
    pub memory_limit_mb: u64,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
}

#[derive(Serialize)]
pub(crate) struct PortStatus {
    pub port: u16,
    pub is_open: bool,
}

/// Static system information returned by `GET /system-info`.
/// Fetched on reconnection and every 24 hours — not included in the
/// high-frequency `/metrics` bincode payload.
#[derive(Serialize)]
pub(crate) struct SystemInfoResponse {
    pub os: String,
    pub cpu_model: String,
    pub memory_total_mb: u64,
    /// System boot time as Unix timestamp (seconds since epoch)
    pub boot_time: u64,
    /// Primary IP address of the host
    pub ip_address: String,
}

/// Intermediate bundle returned by `sysinfo_collector::collect_sysinfo`.
/// Separated from `AgentMetrics` so the handler can assemble the final
/// response from multiple parallel sources.
#[derive(Clone)]
pub(crate) struct SysinfoResult {
    pub cpu_usage: f32,
    pub cpu_cores: Vec<f32>,
    pub memory_total_mb: u64,
    pub memory_used_mb: u64,
    pub memory_usage_percent: f32,
    pub disks: Vec<DiskInfo>,
    pub processes: Vec<ProcessInfo>,
    pub temperatures: Vec<TemperatureInfo>,
    pub gpus: Vec<GpuInfo>,
    pub network: NetworkTotal,
    pub network_interfaces: Vec<NetworkInterfaceInfo>,
    pub load_average: LoadAverage,
}

#[cfg(test)]
mod tests {
    use super::*;
    use bincode::Options as _;

    fn bincode_options() -> impl bincode::Options {
        bincode::DefaultOptions::new()
            .with_limit(10 * 1024 * 1024)
            .with_fixint_encoding()
            .allow_trailing_bytes()
    }

    #[test]
    fn metrics_query_empty_defaults() {
        // Simulate an empty query string (no ports, no containers)
        let query: MetricsQuery = serde_json::from_str("{}").unwrap();
        assert!(query.ports.is_none());
        assert!(query.containers.is_none());
    }

    #[test]
    fn metrics_query_with_ports_and_containers() {
        let query: MetricsQuery =
            serde_json::from_str(r#"{"ports":"80,443","containers":"nginx,redis"}"#).unwrap();
        assert_eq!(query.ports.as_deref(), Some("80,443"));
        assert_eq!(query.containers.as_deref(), Some("nginx,redis"));
    }

    #[test]
    fn metrics_query_with_only_ports() {
        let query: MetricsQuery = serde_json::from_str(r#"{"ports":"8080"}"#).unwrap();
        assert_eq!(query.ports.as_deref(), Some("8080"));
        assert!(query.containers.is_none());
    }

    #[test]
    fn network_total_default() {
        let net = NetworkTotal::default();
        assert_eq!(net.total_rx_bytes, 0);
        assert_eq!(net.total_tx_bytes, 0);
        assert_eq!(net.rx_bytes_per_sec, 0.0);
        assert_eq!(net.tx_bytes_per_sec, 0.0);
    }

    #[test]
    fn network_total_serializes() {
        let net = NetworkTotal {
            total_rx_bytes: 1024,
            total_tx_bytes: 2048,
            rx_bytes_per_sec: 128.0,
            tx_bytes_per_sec: 256.0,
        };
        let json = serde_json::to_value(&net).unwrap();
        assert_eq!(json["total_rx_bytes"], 1024);
        assert_eq!(json["total_tx_bytes"], 2048);
        assert_eq!(json["rx_bytes_per_sec"], 128.0);
        assert_eq!(json["tx_bytes_per_sec"], 256.0);
    }

    #[test]
    fn port_status_serializes() {
        let port = PortStatus {
            port: 443,
            is_open: true,
        };
        let json = serde_json::to_value(&port).unwrap();
        assert_eq!(json["port"], 443);
        assert_eq!(json["is_open"], true);
    }

    #[test]
    fn docker_container_clone() {
        let container = DockerContainer {
            container_name: "nginx".into(),
            image: "nginx:latest".into(),
            state: "running".into(),
            status: "Up 2 hours".into(),
        };
        let cloned = container.clone();
        assert_eq!(cloned.container_name, "nginx");
        assert_eq!(cloned.image, "nginx:latest");
    }

    #[test]
    fn disk_info_serializes() {
        let disk = DiskInfo {
            name: "sda1".into(),
            mount_point: "/".into(),
            total_gb: 500.0,
            available_gb: 200.0,
            usage_percent: 60.0,
            read_bytes_per_sec: 1024.0,
            write_bytes_per_sec: 512.0,
        };
        let json = serde_json::to_value(&disk).unwrap();
        assert_eq!(json["name"], "sda1");
        assert_eq!(json["mount_point"], "/");
        assert_eq!(json["usage_percent"], 60.0);
    }

    #[test]
    fn gpu_info_optional_fields() {
        let gpu = GpuInfo {
            name: "RTX 4090".into(),
            gpu_usage_percent: 85,
            memory_used_mb: 8192,
            memory_total_mb: 24576,
            temperature_c: 72,
            power_watts: Some(350.0),
            frequency_mhz: None,
        };
        let json = serde_json::to_value(&gpu).unwrap();
        assert_eq!(json["power_watts"], 350.0);
        assert!(json["frequency_mhz"].is_null());
    }

    #[test]
    fn agent_metrics_bincode_round_trip() {
        let metrics = AgentMetrics {
            hostname: "test-host".into(),
            timestamp: "2026-04-12T00:00:00Z".into(),
            is_online: true,
            system: SystemMetrics {
                cpu_usage_percent: 45.5,
                memory_total_mb: 16384,
                memory_used_mb: 8192,
                memory_usage_percent: 50.0,
                disks: vec![],
                processes: vec![],
                temperatures: vec![],
                gpus: vec![],
            },
            network: NetworkTotal {
                total_rx_bytes: 1_000_000,
                total_tx_bytes: 500_000,
                rx_bytes_per_sec: 0.0,
                tx_bytes_per_sec: 0.0,
            },
            load_average: LoadAverage {
                one_min: 1.5,
                five_min: 1.2,
                fifteen_min: 0.9,
            },
            docker: vec![],
            ports: vec![PortStatus {
                port: 80,
                is_open: true,
            }],
            agent_version: "1.0.0".into(),
            cpu_cores: vec![45.5, 30.0],
            network_interfaces: vec![],
            docker_stats: vec![],
        };
        let encoded = bincode_options().serialize(&metrics).unwrap();
        assert!(!encoded.is_empty());
    }
}
