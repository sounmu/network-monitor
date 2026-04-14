//! Request handler for `GET /metrics`.

use axum::extract::Query;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use chrono_tz::Asia::Seoul;

use crate::docker_cache::{DockerCache, read_docker_cache};
use crate::models::{AgentMetrics, MetricsQuery, SystemMetrics};
use crate::ports::{collect_ports, parse_comma_separated_ports};
use crate::sysinfo_collector::collect_sysinfo;

#[tracing::instrument(skip(docker_cache, query))]
pub(crate) async fn metrics_handler(
    hostname: String,
    docker_cache: DockerCache,
    query: Query<MetricsQuery>,
) -> Response {
    // Ports are managed server-side and sent via query param
    let monitor_ports = query
        .ports
        .as_ref()
        .map(|p| parse_comma_separated_ports(p))
        .unwrap_or_default();

    let target_containers = query
        .containers
        .as_ref()
        .map(|c| c.split(',').map(|s| s.trim().to_string()).collect());

    // Run sysinfo (which includes a 200 ms blocking sleep for CPU delta), port checks,
    // and Docker cache read in parallel. Docker state is served from the in-memory
    // cache (no HTTP I/O), but including it in the join hides any read-lock contention
    // with the background Docker event listener behind the sysinfo sleep.
    let (sys_result, port_statuses, docker_containers) = tokio::join!(
        collect_sysinfo(),
        collect_ports(monitor_ports),
        read_docker_cache(&docker_cache, target_containers),
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
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
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
