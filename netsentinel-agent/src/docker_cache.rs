//! Event-driven in-memory Docker container cache.
//!
//! An `Arc<RwLock<Vec<DockerContainer>>>` is seeded once at startup and
//! incrementally updated from the Docker Events API, so the `/metrics`
//! handler can respond without making any live Docker API calls.

use crate::models::{DockerContainer, DockerContainerStats};
use bollard::Docker;
use bollard::models::EventMessage;
use bollard::query_parameters::{EventsOptions, ListContainersOptions, StatsOptions};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Shared cache type.
///
/// - Reads (metric collection): multiple requests can hold a read lock concurrently.
/// - Writes (event processing): taken only on container lifecycle events — minimal contention.
pub(crate) type DockerCache = Arc<RwLock<Vec<DockerContainer>>>;

/// Shared container resource stats cache, keyed by container name.
pub(crate) type DockerStatsCache = Arc<RwLock<HashMap<String, DockerContainerStats>>>;

/// Performs a one-time full container list fetch at agent startup to seed the cache.
/// Subsequent updates are incremental via the Docker Events stream — no need to call
/// list_containers again.
#[tracing::instrument(skip(docker))]
pub(crate) async fn initial_docker_load(docker: &Docker) -> Vec<DockerContainer> {
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

/// Subscribes to the Docker Events API and applies incremental cache updates on container
/// lifecycle changes. Instead of polling the full container list every 15 seconds, I/O only
/// happens when an event fires — significantly reducing Docker daemon load and network I/O.
pub(crate) async fn docker_event_listener(cache: DockerCache) {
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
/// Calls the inspect API once for start/create events only; stop/die/pause update state
/// in-cache to minimise Docker daemon API calls.
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
        // Container stopped/exited: update state in-cache only, no extra API call needed.
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
/// Only a read lock is acquired, so multiple concurrent requests can read simultaneously
/// with zero HTTP I/O to the Docker daemon.
pub(crate) async fn read_docker_cache(
    cache: &DockerCache,
    target_containers: Option<Vec<String>>,
) -> Vec<DockerContainer> {
    let containers = cache.read().await;

    match target_containers {
        Some(targets) => {
            // Track which targets have been matched using a HashSet
            let mut matched: std::collections::HashSet<&str> =
                std::collections::HashSet::with_capacity(targets.len());
            let mut result: Vec<DockerContainer> = Vec::with_capacity(targets.len());

            for c in containers.iter() {
                for t in &targets {
                    if c.image.contains(t.as_str()) {
                        matched.insert(t.as_str());
                        result.push(c.clone());
                        break;
                    }
                }
            }

            // Add placeholders for unmatched targets
            for t in &targets {
                if !matched.contains(t.as_str()) {
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

/// Background task that polls container resource stats every 10 seconds.
/// Uses bollard's one-shot stats API to avoid maintaining per-container streaming connections.
pub(crate) async fn docker_stats_poller(
    lifecycle_cache: DockerCache,
    stats_cache: DockerStatsCache,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(10));
    let _ = interval.tick().await; // skip first immediate tick

    loop {
        interval.tick().await;

        let docker = match Docker::connect_with_local_defaults() {
            Ok(d) => d,
            Err(_) => continue,
        };

        // Get running container names from the lifecycle cache
        let running: Vec<String> = {
            let containers = lifecycle_cache.read().await;
            containers
                .iter()
                .filter(|c| c.state == "running")
                .take(50) // cap to avoid overwhelming the Docker daemon
                .map(|c| c.container_name.clone())
                .collect()
        };

        if running.is_empty() {
            // Clear stale stats
            let mut cache = stats_cache.write().await;
            cache.clear();
            continue;
        }

        // Poll stats for each running container concurrently
        let futures = running.into_iter().map(|name| {
            let docker = docker.clone();
            async move {
                let options = StatsOptions {
                    stream: false,
                    one_shot: true,
                };
                let mut stream = docker.stats(&name, Some(options));
                let stats = match stream.next().await {
                    Some(Ok(s)) => s,
                    _ => return None,
                };

                // CPU%: delta of total_usage / delta of system_usage * num_cpus * 100
                let cpu_percent = (|| -> Option<f32> {
                    let cpu = stats.cpu_stats.as_ref()?;
                    let precpu = stats.precpu_stats.as_ref()?;
                    let cur_total = cpu.cpu_usage.as_ref()?.total_usage?;
                    let pre_total = precpu.cpu_usage.as_ref()?.total_usage?;
                    let cpu_delta = cur_total.saturating_sub(pre_total) as f64;
                    let sys_delta = cpu
                        .system_cpu_usage
                        .unwrap_or(0)
                        .saturating_sub(precpu.system_cpu_usage.unwrap_or(0))
                        as f64;
                    let num_cpus = cpu.online_cpus.unwrap_or(1) as f64;
                    if sys_delta > 0.0 {
                        Some((cpu_delta / sys_delta * num_cpus * 100.0) as f32)
                    } else {
                        Some(0.0)
                    }
                })()
                .unwrap_or(0.0);

                // Memory
                let mem_stats = stats.memory_stats.as_ref();
                let mem_usage_mb = mem_stats.and_then(|m| m.usage).unwrap_or(0) / 1024 / 1024;
                let mem_limit_mb = mem_stats.and_then(|m| m.limit).unwrap_or(0) / 1024 / 1024;

                // Network: sum all interface rx/tx
                let (net_rx, net_tx) = stats
                    .networks
                    .as_ref()
                    .map(|nets| {
                        nets.values().fold((0u64, 0u64), |(rx, tx), n| {
                            (rx + n.rx_bytes.unwrap_or(0), tx + n.tx_bytes.unwrap_or(0))
                        })
                    })
                    .unwrap_or((0, 0));

                Some(DockerContainerStats {
                    container_name: name,
                    cpu_percent,
                    memory_usage_mb: mem_usage_mb,
                    memory_limit_mb: mem_limit_mb,
                    net_rx_bytes: net_rx,
                    net_tx_bytes: net_tx,
                })
            }
        });

        let results: Vec<Option<DockerContainerStats>> =
            futures_util::future::join_all(futures).await;

        // Update the stats cache atomically
        let mut cache = stats_cache.write().await;
        cache.clear();
        for stat in results.into_iter().flatten() {
            cache.insert(stat.container_name.clone(), stat);
        }
    }
}

/// Read container stats from the cache at metric collection time.
pub(crate) async fn read_docker_stats(stats_cache: &DockerStatsCache) -> Vec<DockerContainerStats> {
    let cache = stats_cache.read().await;
    cache.values().cloned().collect()
}

#[cfg(test)]
mod tests {
    // The Docker API prepends '/' to container names ("/my-app" → "my-app"), so the
    // cache strips it. These tests pin that string-trimming convention.

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
}
