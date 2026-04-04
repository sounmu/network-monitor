mod gpu;
mod logger;

use anyhow::Context;
use axum::Router;
use axum::{
    extract::{Query, Request},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use bollard::Docker;
use bollard::models::EventMessage;
use bollard::query_parameters::{EventsOptions, ListContainersOptions};
use chrono::Utc;
use chrono_tz::Asia::Seoul;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::RwLock;
use sysinfo::{Components, Disks, Networks, System};
use tokio::net::TcpListener;

// ──────────────────────────────────────────────
// JWT middleware
// ──────────────────────────────────────────────
#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    exp: usize,
}

static DECODING_KEY: OnceLock<DecodingKey> = OnceLock::new();

async fn auth_middleware(req: Request, next: Next) -> Result<Response, StatusCode> {
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|val| val.to_str().ok())
        .filter(|s| s.starts_with("Bearer "));

    let token = match auth_header {
        Some(s) => &s[7..],
        None => return Err(StatusCode::UNAUTHORIZED),
    };

    let key = DECODING_KEY.get().expect("DECODING_KEY not initialized");
    let validation = Validation::new(Algorithm::HS256);
    // jsonwebtoken::Validation::new(Algorithm::HS256) automatically validates the `exp` claim.

    match decode::<Claims>(token, key, &validation) {
        Ok(_) => Ok(next.run(req).await),
        Err(e) => {
            tracing::warn!(err = ?e, "⚠️ [Auth] JWT validation failed");
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

// ──────────────────────────────────────────────
// Utilities
// ──────────────────────────────────────────────

/// Parse a comma-separated port string into `Vec<u16>`.
/// Invalid values (out-of-range, non-numeric) are silently ignored.
fn parse_comma_separated_ports(input: &str) -> Vec<u16> {
    input
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect()
}

// ──────────────────────────────────────────────
// Response and query structs
// ──────────────────────────────────────────────

#[derive(Deserialize)]
pub struct MetricsQuery {
    pub ports: Option<String>,
    pub containers: Option<String>,
}

#[derive(Serialize)]
struct AgentMetrics {
    hostname: String,
    timestamp: String,
    is_online: bool,
    system: SystemMetrics,
    /// Aggregate traffic across physical interfaces (virtual/loopback excluded)
    network: NetworkTotal,
    load_average: LoadAverage,
    docker: Vec<DockerContainer>,
    ports: Vec<PortStatus>,
}

#[derive(Serialize)]
struct SystemMetrics {
    cpu_usage_percent: f32,
    memory_total_mb: u64,
    memory_used_mb: u64,
    memory_usage_percent: f32,
    disks: Vec<DiskInfo>,
    processes: Vec<ProcessInfo>,
    temperatures: Vec<TemperatureInfo>,
    gpus: Vec<GpuInfo>,
}

#[derive(Serialize)]
struct DiskInfo {
    name: String,
    mount_point: String,
    total_gb: f64,
    available_gb: f64,
    usage_percent: f32,
}

#[derive(Serialize)]
struct ProcessInfo {
    pid: u32,
    name: String,
    cpu_usage: f32,
    memory_mb: u64,
}

#[derive(Serialize)]
struct TemperatureInfo {
    label: String,
    temperature_c: f32,
}

#[derive(Serialize)]
struct GpuInfo {
    name: String,
    gpu_usage_percent: u32,
    memory_used_mb: u64,
    memory_total_mb: u64,
    temperature_c: u32,
    // New fields — appended at end for bincode compat with server
    power_watts: Option<f32>,
    frequency_mhz: Option<u32>,
}

/// Physical-interface traffic totals (cumulative bytes after agent-side filtering).
///
/// Sent as a single aggregate struct rather than a per-interface array because:
/// - It reduces the JSON payload size sent to the server.
/// - The server needs no duplicate filtering logic.
/// - Monitoring aggregate throughput is sufficient for this use case.
#[derive(Serialize, Default)]
struct NetworkTotal {
    total_rx_bytes: u64,
    total_tx_bytes: u64,
}

#[derive(Serialize)]
struct LoadAverage {
    one_min: f64,
    five_min: f64,
    fifteen_min: f64,
}

#[derive(Serialize, Clone)]
struct DockerContainer {
    container_name: String,
    image: String,
    state: String,
    status: String,
}

/// Event-driven in-memory Docker container cache.
/// Arc<RwLock<...>> ensures safe concurrent access between the metrics handler and the background event listener.
/// - Reads (metric collection): multiple requests can hold a Read Lock simultaneously.
/// - Writes (event processing): Write Lock acquired only on container lifecycle events — minimal contention.
type DockerCache = Arc<RwLock<Vec<DockerContainer>>>;

#[derive(Serialize)]
struct PortStatus {
    port: u16,
    is_open: bool,
}

// ──────────────────────────────────────────────
// sysinfo collection (includes CPU delta, blocking)
// ──────────────────────────────────────────────

struct SysinfoResult {
    cpu_usage: f32,
    memory_total_mb: u64,
    memory_used_mb: u64,
    memory_usage_percent: f32,
    disks: Vec<DiskInfo>,
    processes: Vec<ProcessInfo>,
    temperatures: Vec<TemperatureInfo>,
    gpus: Vec<GpuInfo>,
    network: NetworkTotal,
    load_average: LoadAverage,
}

/// Virtual/dummy interface prefix list.
///
/// Filtering on the agent side because:
/// - It reduces the JSON payload sent to the server (there can be many veth/docker interfaces).
/// - The server does not need to maintain a separate filter list.
const FILTERED_PREFIXES: &[&str] = &[
    "lo",     // loopback (lo, lo0, lo1)
    "docker", // Docker default bridge
    "br-",    // container user-defined bridge
    "veth",   // per-container virtual ethernet
    "utun",   // userspace tunnel (VPN, etc.)
    "awdl",   // Apple Wireless Direct Link (AirDrop)
    "llw",    // Low-latency WLAN (iPhone tethering)
    "gif",    // Generic Tunnel Interface
    "stf",    // IPv6-in-IPv4 (6to4) tunnel
    "anpi",   // Apple Network Proxy Interface
    "ap",     // Apple internal wireless AP
];

fn is_physical_interface(name: &str) -> bool {
    !FILTERED_PREFIXES.iter().any(|p| name.starts_with(p))
}

#[tracing::instrument]
async fn collect_sysinfo() -> SysinfoResult {
    // GPU collection runs on its own blocking thread — its ~200ms sampling window
    // overlaps with the CPU/process delta sleeps, hiding the latency entirely.
    let gpu_handle = tokio::task::spawn_blocking(gpu::collect_gpu_info);

    // sysinfo collection runs on a separate blocking thread.
    let sys_handle = tokio::task::spawn_blocking(|| {
        let mut sys = System::new();

        // Three-phase CPU delta measurement:
        // macOS requires three refresh_processes calls to produce non-zero cpu_usage().
        // Phase 1: populate process list, Phase 2: establish baseline, Phase 3: compute delta.
        sys.refresh_cpu_usage();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        std::thread::sleep(Duration::from_millis(100));

        sys.refresh_cpu_usage();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        std::thread::sleep(Duration::from_millis(100));

        sys.refresh_cpu_usage();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        sys.refresh_memory();

        let cpu_usage = sys.global_cpu_usage();
        let memory_total_mb = sys.total_memory() / 1024 / 1024;
        let memory_used_mb = sys.used_memory() / 1024 / 1024;
        let memory_usage_percent = if sys.total_memory() > 0 {
            (sys.used_memory() as f64 / sys.total_memory() as f64 * 100.0) as f32
        } else {
            0.0
        };

        // Disks
        let disks_raw = Disks::new_with_refreshed_list();
        let disks = disks_raw
            .iter()
            .map(|disk| {
                let total_bytes = disk.total_space();
                let available_bytes = disk.available_space();
                let used_bytes = total_bytes.saturating_sub(available_bytes);
                DiskInfo {
                    name: disk.name().to_string_lossy().into_owned(),
                    mount_point: disk.mount_point().to_string_lossy().into_owned(),
                    total_gb: total_bytes as f64 / 1_073_741_824.0,
                    available_gb: available_bytes as f64 / 1_073_741_824.0,
                    usage_percent: if total_bytes > 0 {
                        (used_bytes as f64 / total_bytes as f64 * 100.0) as f32
                    } else {
                        0.0
                    },
                }
            })
            .collect();

        // Aggregate physical interface traffic.
        let nets = Networks::new_with_refreshed_list();
        let network = nets
            .iter()
            .filter(|(name, _)| is_physical_interface(name))
            .fold(NetworkTotal::default(), |mut acc, (_, data)| {
                acc.total_rx_bytes += data.total_received();
                acc.total_tx_bytes += data.total_transmitted();
                acc
            });

        // Load average (Linux/macOS — returns 0.0 on Windows)
        let la = System::load_average();
        let load_average = LoadAverage {
            one_min: la.one,
            five_min: la.five,
            fifteen_min: la.fifteen,
        };

        // Top 10 processes by CPU usage (already refreshed three times above for accurate delta)
        let mut process_list: Vec<ProcessInfo> = sys
            .processes()
            .values()
            .map(|p| ProcessInfo {
                pid: p.pid().as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                cpu_usage: p.cpu_usage(),
                memory_mb: p.memory() / 1024 / 1024,
            })
            .collect();
        process_list.sort_by(|a, b| b.cpu_usage.partial_cmp(&a.cpu_usage).unwrap_or(std::cmp::Ordering::Equal));
        process_list.truncate(10);

        // Temperature sensors
        let components = Components::new_with_refreshed_list();
        let temperatures: Vec<TemperatureInfo> = components
            .iter()
            .filter_map(|c| {
                let temp = c.temperature()?;
                if temp.is_finite() {
                    Some(TemperatureInfo {
                        label: c.label().to_string(),
                        temperature_c: temp,
                    })
                } else {
                    None
                }
            })
            .collect();

        (cpu_usage, memory_total_mb, memory_used_mb, memory_usage_percent,
         disks, process_list, temperatures, network, load_average)
    });

    // Await both blocking tasks concurrently — GPU sampling overlaps with CPU delta sleeps.
    let (gpu_result, sys_result) = tokio::join!(gpu_handle, sys_handle);
    let gpus = gpu_result.expect("spawn_blocking panicked in collect_gpu_info");
    let (cpu_usage, memory_total_mb, memory_used_mb, memory_usage_percent,
         disks, processes, temperatures, network, load_average) =
        sys_result.expect("spawn_blocking panicked in collect_sysinfo");

    SysinfoResult {
        cpu_usage,
        memory_total_mb,
        memory_used_mb,
        memory_usage_percent,
        disks,
        processes,
        temperatures,
        gpus,
        network,
        load_average,
    }
}

// ──────────────────────────────────────────────
// Event-driven Docker in-memory cache
// ──────────────────────────────────────────────

/// Performs a one-time full container list fetch at agent startup to seed the cache.
/// Subsequent updates are incremental via the Docker Events stream — no need to call list_containers again.
#[tracing::instrument(skip(docker))]
async fn initial_docker_load(docker: &Docker) -> Vec<DockerContainer> {
    let options = ListContainersOptions {
        all: true,
        filters: None,
        ..Default::default()
    };

    match docker.list_containers(Some(options)).await {
        Ok(containers) => {
            let result: Vec<DockerContainer> = containers
                .into_iter()
                .map(|c| {
                    let name = c
                        .names
                        .unwrap_or_default()
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "unknown".to_string())
                        .trim_start_matches('/')
                        .to_string();

                    DockerContainer {
                        container_name: name,
                        image: c.image.unwrap_or_else(|| "unknown".to_string()),
                        state: c
                            .state
                            .map(|s| format!("{:?}", s).to_lowercase())
                            .unwrap_or_else(|| "unknown".to_string()),
                        status: c.status.unwrap_or_else(|| "unknown".to_string()),
                    }
                })
                .collect();
            tracing::info!(count = result.len(), "Docker cache initialized");
            result
        }
        Err(e) => {
            tracing::error!(err = ?e, "⚠️  [Docker] Initial container load failed");
            vec![]
        }
    }
}

/// Subscribes to the Docker Events API and applies incremental cache updates on container lifecycle changes.
/// Instead of polling the full container list every 15 seconds, I/O only happens when an event fires —
/// significantly reducing Docker daemon load and network I/O overhead.
async fn docker_event_listener(cache: DockerCache) {
    loop {
        let docker = match Docker::connect_with_local_defaults() {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(err = ?e, "⚠️  [Docker Events] Connection failed, retrying in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        // On reconnect, reload the full state to catch any events missed while the stream was down.
        let refreshed = initial_docker_load(&docker).await;
        {
            let mut containers = cache.write().await;
            *containers = refreshed;
        }

        // Filter to container events only — avoids receiving image/network/volume noise.
        let mut filters = HashMap::new();
        filters.insert("type".to_string(), vec!["container".to_string()]);

        let options = EventsOptions {
            since: None,
            until: None,
            filters: Some(filters),
        };

        let mut stream = docker.events(Some(options));
        tracing::info!("🐳 [Docker Events] Listening for container lifecycle events");

        while let Some(event_result) = stream.next().await {
            match event_result {
                Ok(event) => handle_docker_event(&docker, &cache, event).await,
                Err(e) => {
                    tracing::error!(err = ?e, "⚠️  [Docker Events] Stream error, reconnecting...");
                    break;
                }
            }
        }

        tracing::warn!("⚠️  [Docker Events] Stream ended, reconnecting in 5s");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// Process a single Docker event and update only the affected container in the cache.
/// Calls the inspect API once for start/create events only; stop/die/pause update state in-cache
/// to minimise Docker daemon API calls.
async fn handle_docker_event(docker: &Docker, cache: &DockerCache, event: EventMessage) {
    let action = match event.action.as_deref() {
        Some(a) => a,
        None => return,
    };

    let actor = match &event.actor {
        Some(a) => a,
        None => return,
    };

    let container_id = match &actor.id {
        Some(id) => id.as_str(),
        None => return,
    };

    let container_name = actor
        .attributes
        .as_ref()
        .and_then(|attrs| attrs.get("name"))
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());

    match action {
        // Container started/resumed: fetch full state via inspect and update the cache.
        // inspect is necessary here because a newly started container may not be in the cache yet.
        "start" | "unpause" => {
            if let Ok(info) = docker.inspect_container(container_id, None).await {
                let image = info
                    .config
                    .as_ref()
                    .and_then(|c| c.image.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                let state = info
                    .state
                    .as_ref()
                    .and_then(|s| s.status.as_ref())
                    .map(|s| format!("{:?}", s).to_lowercase())
                    .unwrap_or_else(|| "unknown".to_string());

                let updated = DockerContainer {
                    container_name: container_name.clone(),
                    image,
                    state,
                    status: "Running".to_string(),
                };

                let mut containers = cache.write().await;
                if let Some(c) = containers
                    .iter_mut()
                    .find(|c| c.container_name == container_name)
                {
                    *c = updated;
                } else {
                    containers.push(updated);
                }
            }
        }
        // Container stopped/exited: update state in-cache only, no extra API call needed (saves I/O).
        "stop" | "die" => {
            let mut containers = cache.write().await;
            if let Some(c) = containers
                .iter_mut()
                .find(|c| c.container_name == container_name)
            {
                c.state = "exited".to_string();
                c.status = "Exited".to_string();
            }
        }
        // Container paused: update state string only.
        "pause" => {
            let mut containers = cache.write().await;
            if let Some(c) = containers
                .iter_mut()
                .find(|c| c.container_name == container_name)
            {
                c.state = "paused".to_string();
                c.status = "Paused".to_string();
            }
        }
        // New container created: fetch full info via inspect and add to cache.
        "create" => {
            if let Ok(info) = docker.inspect_container(container_id, None).await {
                let image = info
                    .config
                    .as_ref()
                    .and_then(|c| c.image.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                let state = info
                    .state
                    .as_ref()
                    .and_then(|s| s.status.as_ref())
                    .map(|s| format!("{:?}", s).to_lowercase())
                    .unwrap_or_else(|| "created".to_string());

                let new_container = DockerContainer {
                    container_name: container_name.clone(),
                    image,
                    state,
                    status: "Created".to_string(),
                };

                let mut containers = cache.write().await;
                if !containers
                    .iter()
                    .any(|c| c.container_name == container_name)
                {
                    containers.push(new_container);
                }
            }
        }
        // Container destroyed: remove from cache.
        "destroy" => {
            let mut containers = cache.write().await;
            containers.retain(|c| c.container_name != container_name);
        }
        _ => return, // Ignore other events (rename, exec, etc.)
    }

    tracing::debug!(
        action = action,
        container = %container_name,
        "🐳 Docker event processed"
    );
}

/// Return Docker containers from the cache at metric collection time.
/// Only a Read Lock is acquired, so multiple concurrent requests can read simultaneously
/// with zero HTTP I/O to the Docker daemon — eliminates latency from polling.
async fn read_docker_cache(
    cache: &DockerCache,
    target_containers: Option<Vec<String>>,
) -> Vec<DockerContainer> {
    let containers = cache.read().await;

    match target_containers {
        Some(targets) => {
            let mut result: Vec<DockerContainer> = containers
                .iter()
                .filter(|c| targets.iter().any(|t| c.image.contains(t)))
                .cloned()
                .collect();

            // If a target image is not in the cache, add a placeholder with "Missing" state.
            for t in &targets {
                if !result.iter().any(|c| c.image.contains(t)) {
                    result.push(DockerContainer {
                        container_name: format!("Missing ({})", t),
                        image: t.clone(),
                        state: "off".to_string(),
                        status: "Not Found".to_string(),
                    });
                }
            }
            result
        }
        None => containers.clone(),
    }
}

// ──────────────────────────────────────────────
// Port state check
// ──────────────────────────────────────────────

#[tracing::instrument]
async fn collect_ports(ports: Vec<u16>) -> Vec<PortStatus> {
    tokio::task::spawn_blocking(move || {
        ports
            .into_iter()
            .map(|port| {
                let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
                let is_open = TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_ok();
                PortStatus { port, is_open }
            })
            .collect()
    })
    .await
    .expect("spawn_blocking panicked in collect_ports")
}

// ──────────────────────────────────────────────
// GET /metrics handler
// ──────────────────────────────────────────────

#[tracing::instrument(skip(docker_cache, query))]
async fn metrics_handler(
    hostname: String,
    docker_cache: DockerCache,
    query: Query<MetricsQuery>,
) -> impl IntoResponse {
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

    // Run sysinfo (which includes a 200 ms blocking sleep for CPU delta) and port checks in parallel.
    let (sys_result, port_statuses) = tokio::join!(
        collect_sysinfo(),
        collect_ports(monitor_ports),
    );

    // Docker state is served instantly from the in-memory cache — no HTTP I/O.
    let docker_containers = read_docker_cache(&docker_cache, target_containers).await;

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
    };

    // bincode binary serialisation: ~40–70% smaller payload than JSON, near-zero-copy parsing speed.
    // Both agent and server are Rust, so serde-based binary format field-order compatibility is guaranteed.
    let bytes = bincode::serialize(&metrics)
        .expect("AgentMetrics bincode serialization should never fail");

    (
        [(header::CONTENT_TYPE, "application/octet-stream")],
        bytes,
    )
}

// ──────────────────────────────────────────────
// Main
// ──────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let _guard = logger::init_tracing();
    tracing::info!("Starting network-monitor-agent...");

    let port: u16 = std::env::var("AGENT_PORT")
        .unwrap_or_else(|_| "9100".to_string())
        .parse()
        .context("AGENT_PORT is not a valid port number (1–65535)")?;

    let jwt_secret = std::env::var("JWT_SECRET")
        .context("JWT_SECRET environment variable is not set. Please check your .env file.")?;
    DECODING_KEY.set(DecodingKey::from_secret(jwt_secret.as_bytes())).ok();

    let hostname = System::host_name().unwrap_or_else(|| "unknown".to_string());

    tracing::info!(hostname = %hostname, "Node configuration");

    // Initialise the Docker in-memory cache with a one-time full container list fetch at startup.
    let docker_cache: DockerCache = Arc::new(RwLock::new(
        match Docker::connect_with_local_defaults() {
            Ok(docker) => initial_docker_load(&docker).await,
            Err(e) => {
                tracing::warn!(err = ?e, "⚠️  [Docker] Initial connection failed, cache starts empty");
                vec![]
            }
        },
    ));

    // Spawn the Docker Events API listener as a background task.
    // Incrementally updates the cache only when container start/stop/die/pause/unpause/create/destroy
    // events fire — far cheaper than periodic polling.
    tokio::spawn(docker_event_listener(docker_cache.clone()));

    let app = Router::new().route(
        "/metrics",
        get({
            let hostname = hostname.clone();
            let cache = docker_cache.clone();
            move |query: Query<MetricsQuery>| async move {
                metrics_handler(hostname.clone(), cache.clone(), query).await
            }
        }),
    ).layer(middleware::from_fn(auth_middleware));

    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind to port {} — is it already in use?", port))?;

    tracing::info!("Agent exporter running on http://{}", addr);
    tracing::info!("Scrape endpoint: GET http://{}/metrics", addr);

    axum::serve(listener, app)
        .await
        .context("Agent server encountered a fatal error")?;

    Ok(())
}

// ──────────────────────────────────────────────
// Unit tests
// ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};

    // ── Port parsing ─────────────────────────────

    #[test]
    fn test_port_parsing_filters_invalid_values() {
        assert_eq!(
            parse_comma_separated_ports("80,443,invalid,8080"),
            vec![80, 443, 8080],
            "Invalid ports should be silently removed"
        );
    }

    #[test]
    fn test_port_parsing_trims_whitespace() {
        assert_eq!(parse_comma_separated_ports(" 80 , 443 , 3000 "), vec![80, 443, 3000]);
    }

    #[test]
    fn test_port_parsing_empty_string_returns_empty() {
        assert!(parse_comma_separated_ports("").is_empty());
    }

    #[test]
    fn test_port_parsing_rejects_out_of_range() {
        // Values exceeding u16::MAX (65535) fail to parse and are dropped.
        assert_eq!(parse_comma_separated_ports("80,65536,443"), vec![80, 443]);
    }

    // ── Container name normalisation ─────────────
    // The Docker API prepends '/' to container names ("/my-app" → "my-app").

    #[test]
    fn test_container_name_strips_leading_slash() {
        let raw = "/my-container";
        let name = raw.trim_start_matches('/').to_string();
        assert_eq!(name, "my-container");
    }

    #[test]
    fn test_container_name_without_slash_unchanged() {
        let raw = "my-container";
        let name = raw.trim_start_matches('/').to_string();
        assert_eq!(name, "my-container");
    }

    // ── JWT token validation ─────────────────────

    fn test_validation() -> Validation {
        let mut v = Validation::new(Algorithm::HS256);
        v.validate_exp = false;
        v
    }

    #[test]
    fn test_valid_jwt_decodes_successfully() {
        let secret = b"test-agent-secret";
        let token = encode(
            &Header::new(Algorithm::HS256),
            &Claims { exp: usize::MAX },
            &EncodingKey::from_secret(secret),
        )
        .expect("Token creation failed");
        let result = decode::<Claims>(&token, &DecodingKey::from_secret(secret), &test_validation());
        assert!(result.is_ok(), "Should succeed with the correct secret");
    }

    #[test]
    fn test_jwt_with_wrong_secret_is_rejected() {
        let token = encode(
            &Header::new(Algorithm::HS256),
            &Claims { exp: usize::MAX },
            &EncodingKey::from_secret(b"correct-secret"),
        )
        .expect("Token creation failed");
        let result = decode::<Claims>(&token, &DecodingKey::from_secret(b"wrong-secret"), &test_validation());
        assert!(result.is_err(), "Should fail with the wrong secret");
    }

    #[test]
    fn test_malformed_token_is_rejected() {
        let result = decode::<Claims>(
            "this.is.not.a.real.jwt",
            &DecodingKey::from_secret(b"any-secret"),
            &Validation::new(Algorithm::HS256),
        );
        assert!(result.is_err(), "Malformed token must be rejected");
    }
}
