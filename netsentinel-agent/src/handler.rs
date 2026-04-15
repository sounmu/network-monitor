//! Request handlers for `GET /metrics` and `GET /system-info`.

use axum::Json;
use axum::extract::Query;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use chrono_tz::Asia::Seoul;
use sysinfo::System;

use crate::docker_cache::{DockerCache, DockerStatsCache, read_docker_cache, read_docker_stats};
use crate::models::{AgentMetrics, MetricsQuery, SystemInfoResponse, SystemMetrics};
use crate::ports::{collect_ports, parse_comma_separated_ports};
use crate::sysinfo_collector::collect_sysinfo;

const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[tracing::instrument(skip(docker_cache, docker_stats_cache, query))]
pub(crate) async fn metrics_handler(
    hostname: String,
    docker_cache: DockerCache,
    docker_stats_cache: DockerStatsCache,
    query: Query<MetricsQuery>,
) -> Response {
    // Ports are managed server-side and sent via query param (capped at 100 to prevent DoS)
    let mut monitor_ports = query
        .ports
        .as_ref()
        .map(|p| parse_comma_separated_ports(p))
        .unwrap_or_default();
    monitor_ports.truncate(100);

    let target_containers = query
        .containers
        .as_ref()
        .map(|c| c.split(',').map(|s| s.trim().to_string()).collect());

    // Run sysinfo (which includes a 200 ms blocking sleep for CPU delta), port checks,
    // and Docker cache read in parallel. Docker state is served from the in-memory
    // cache (no HTTP I/O), but including it in the join hides any read-lock contention
    // with the background Docker event listener behind the sysinfo sleep.
    let (sys_result, port_statuses, docker_containers, docker_stats) = tokio::join!(
        collect_sysinfo(),
        collect_ports(monitor_ports),
        read_docker_cache(&docker_cache, target_containers),
        read_docker_stats(&docker_stats_cache),
    );

    let timestamp = Utc::now()
        .with_timezone(&Seoul)
        .format("%Y-%m-%d %H:%M:%S %Z")
        .to_string();

    tracing::info!(
        cpu = %format!("{:.1}%", sys_result.cpu_usage),
        ram = %format!("{}/{} MB", sys_result.memory_used_mb, sys_result.memory_total_mb),
        load = %format!("{:.2}/{:.2}/{:.2}", sys_result.load_average.one_min, sys_result.load_average.five_min, sys_result.load_average.fifteen_min),
        docker_count = docker_containers.len(),
        open_ports = port_statuses.iter().filter(|p| p.is_open).count(),
        "Scraped metrics"
    );

    let metrics = AgentMetrics {
        hostname,
        timestamp,
        is_online: true,
        system: SystemMetrics {
            cpu_usage_percent: sys_result.cpu_usage,
            memory_total_mb: sys_result.memory_total_mb,
            memory_used_mb: sys_result.memory_used_mb,
            memory_usage_percent: sys_result.memory_usage_percent,
            disks: sys_result.disks,
            processes: sys_result.processes,
            temperatures: sys_result.temperatures,
            gpus: sys_result.gpus,
        },
        network: sys_result.network,
        load_average: sys_result.load_average,
        docker: docker_containers,
        ports: port_statuses,
        agent_version: AGENT_VERSION.to_string(),
        cpu_cores: sys_result.cpu_cores,
        network_interfaces: sys_result.network_interfaces,
        docker_stats,
    };

    // bincode binary serialisation: ~40–70% smaller payload than JSON, near-zero-copy parsing
    // speed. Both agent and server are Rust, so serde-based binary format field-order
    // compatibility is guaranteed.
    let bytes = match bincode::serialize(&metrics) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(err = ?e, "❌ [Metrics] bincode serialization failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "serialization error").into_response();
        }
    };

    ([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response()
}

/// GET /system-info — static system information (OS, CPU model, IP, boot time, total RAM).
/// Called infrequently (on reconnection + every 24h) so JSON is fine.
#[tracing::instrument]
pub(crate) async fn system_info_handler() -> Json<SystemInfoResponse> {
    let info = tokio::task::spawn_blocking(|| {
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        sys.refresh_memory();

        let os = System::long_os_version().unwrap_or_else(|| "Unknown".to_string());
        let cpu_model = sys
            .cpus()
            .first()
            .map(|c| c.brand().to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        let memory_total_mb = sys.total_memory() / 1024 / 1024;
        let boot_time = System::boot_time();
        let ip_address = get_primary_ip();

        SystemInfoResponse {
            os,
            cpu_model,
            memory_total_mb,
            boot_time,
            ip_address,
        }
    })
    .await
    .unwrap_or_else(|e| {
        tracing::error!(err = ?e, "❌ [SystemInfo] spawn_blocking panicked");
        SystemInfoResponse {
            os: "Unknown".to_string(),
            cpu_model: "Unknown".to_string(),
            memory_total_mb: 0,
            boot_time: 0,
            ip_address: "Unknown".to_string(),
        }
    });

    Json(info)
}

/// Determine the primary IP address by creating a UDP socket aimed at a
/// public address. No data is actually sent — the OS routing table selects
/// the source interface, giving us the default outbound IP.
fn get_primary_ip() -> String {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .ok()
        .and_then(|s| {
            s.connect("8.8.8.8:80").ok()?;
            s.local_addr().ok()
        })
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}
