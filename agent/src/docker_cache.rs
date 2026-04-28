//! Event-driven in-memory Docker container cache.
//!
//! An `Arc<RwLock<Vec<DockerContainer>>>` is seeded once at startup and
//! incrementally updated from the Docker Events API, so the `/metrics`
//! handler can respond without making any live Docker API calls.

use crate::models::{DockerContainer, DockerContainerStats};
use bollard::Docker;
use bollard::models::EventMessage;
use bollard::query_parameters::{EventsOptions, ListContainersOptions, StatsOptions};
use futures_util::{StreamExt, stream};
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
    let mut backoff = Duration::from_secs(5);
    const MAX_BACKOFF: Duration = Duration::from_secs(300);
    /// Mirrors `docker_stats_poller::HEALTHY_DURATION_FOR_RESET` — the
    /// stream must stay open this long before the backoff resets to its
    /// 5 s floor. Without this gate, a daemon that accepts the connect
    /// but kills the events stream within a second would oscillate at
    /// the base interval, hammering a recovering daemon. 60 s is well
    /// above any realistic startup race.
    const HEALTHY_DURATION_FOR_RESET: Duration = Duration::from_secs(60);

    loop {
        let docker = match Docker::connect_with_local_defaults() {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(err = ?e, "⚠️  [Docker Events] Connection failed, retrying in {}s", backoff.as_secs());
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
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
        let stream_started_at = std::time::Instant::now();

        while let Some(event_result) = stream.next().await {
            match event_result {
                Ok(event) => handle_docker_event(&docker, &cache, event).await,
                Err(e) => {
                    tracing::error!(err = ?e, "⚠️  [Docker Events] Stream error, reconnecting...");
                    break;
                }
            }
        }

        // Reset backoff only if the stream stayed open long enough to
        // prove the daemon is stable. Fast-flap (connect ok, stream
        // dies within seconds) keeps the exponential escalation alive.
        if stream_started_at.elapsed() >= HEALTHY_DURATION_FOR_RESET {
            backoff = Duration::from_secs(5);
        } else {
            tracing::warn!(
                "⚠️  [Docker Events] Stream ended after {} s — keeping backoff at {} s",
                stream_started_at.elapsed().as_secs(),
                backoff.as_secs()
            );
        }

        tracing::warn!(
            "⚠️  [Docker Events] Stream ended, reconnecting in {}s",
            backoff.as_secs()
        );
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Process a single Docker event and update only the affected container in the cache.
/// Calls the inspect API once for start/create events only; stop/die/pause update state
/// in-cache to minimise Docker daemon API calls.
async fn handle_docker_event(docker: &Docker, cache: &DockerCache, event: EventMessage) {
    let Some(action) = event.action.as_deref() else {
        return;
    };
    let Some(actor) = &event.actor else {
        return;
    };
    let Some(container_id) = actor.id.as_deref() else {
        return;
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
                    // Match against the image's **repository name** segment,
                    // not a raw `contains`. The old substring match reported
                    // `nginx-exporter` as matching a `nginx` target — which
                    // is what the Prometheus folks ship *alongside* the real
                    // nginx image, so a user watching `nginx` could silently
                    // end up monitoring the exporter and miss the outage.
                    //
                    // The image reference format is
                    // `[registry/]repo[:tag][@digest]` (see Docker image
                    // reference spec). We care about the repo portion — take
                    // the substring between the last `/` (or start) and the
                    // first `:` / `@`, then match exactly.
                    if image_repo_matches(&c.image, t.as_str()) {
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

/// Match a Docker image reference against a target name, comparing only
/// the **repository** component. Returns `true` when `target` equals the
/// repo name exactly OR equals a full `namespace/repo` prefix.
///
/// Examples:
///   - `image="nginx:1.25", target="nginx"`            → true
///   - `image="nginx:1.25", target="nginx-exporter"`   → false  ← regression fix
///   - `image="library/nginx:1.25", target="nginx"`    → true
///   - `image="ghcr.io/foo/bar:v1@sha256:…", target="bar"` → true
///   - `image="ghcr.io/foo/bar:v1", target="foo/bar"`  → true
///
/// Not a full OCI reference parser — a registry with a port
/// (`localhost:5000/foo`) is deliberately treated as registry-stripped.
/// Covers the common case that caused false positives without adding a
/// dependency on an image-reference crate.
fn image_repo_matches(image: &str, target: &str) -> bool {
    // Drop `@sha256:…` digest suffix first.
    let without_digest = image.split('@').next().unwrap_or(image);
    // Drop the `:tag` suffix, but only when the colon belongs to the tag —
    // a `host:port/` registry prefix has its own colon that must survive.
    // Find the tag colon by looking only inside the segment after the
    // final `/` (or the whole string if there is no `/`).
    let path_start = without_digest.rfind('/').map(|i| i + 1).unwrap_or(0);
    let without_tag = match without_digest[path_start..].find(':') {
        Some(rel) => &without_digest[..path_start + rel],
        None => without_digest,
    };

    // Match full namespaced form first (`library/nginx` == target).
    if without_tag == target {
        return true;
    }
    // Then match the bare repo name (last path segment).
    let repo_only = without_tag.rsplit('/').next().unwrap_or(without_tag);
    repo_only == target
}

#[cfg(test)]
mod image_match_tests {
    use super::image_repo_matches;

    #[test]
    fn matches_bare_repo() {
        assert!(image_repo_matches("nginx:1.25", "nginx"));
        assert!(image_repo_matches("nginx", "nginx"));
    }

    #[test]
    fn does_not_match_partial_repo_name() {
        // The original bug: `nginx-exporter` used to match a `nginx` target.
        assert!(!image_repo_matches("nginx-exporter:1.0", "nginx"));
        assert!(!image_repo_matches("quay.io/nginx-exporter", "nginx"));
    }

    #[test]
    fn matches_namespace_slash_repo() {
        assert!(image_repo_matches("library/nginx:1.25", "nginx"));
        assert!(image_repo_matches("library/nginx:1.25", "library/nginx"));
    }

    #[test]
    fn matches_through_registry_and_digest() {
        assert!(image_repo_matches("ghcr.io/foo/bar:v1@sha256:abc", "bar"));
    }
}

/// Background task that polls container resource stats every 10 seconds.
/// Uses bollard's one-shot stats API to avoid maintaining per-container streaming connections.
/// Docker client is created once and reused across cycles; reconnects only on error.
pub(crate) async fn docker_stats_poller(
    lifecycle_cache: DockerCache,
    stats_cache: DockerStatsCache,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(10));
    // If the Docker daemon pauses responding for minutes, `tokio::interval`
    // would fire every missed tick back-to-back on recovery. `Delay` skips
    // the backlog and resumes cleanly from the next boundary — avoids a
    // burst of `poll_container_stats` calls that can overwhelm the
    // freshly-recovered daemon.
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let _ = interval.tick().await; // skip first immediate tick

    let mut backoff = Duration::from_secs(10);
    const MAX_BACKOFF: Duration = Duration::from_secs(300);
    /// Minimum time the inner poll loop must run without erroring before
    /// we trust the connection enough to reset the reconnect backoff.
    /// Without this, a daemon that accepts a connect but then dies
    /// within the first poll would endlessly oscillate at the base
    /// 10 s backoff, hammering the recovering daemon with reconnects.
    const HEALTHY_DURATION_FOR_RESET: Duration = Duration::from_secs(60);

    loop {
        // Connect once, reuse across multiple poll cycles
        let docker = match Docker::connect_with_local_defaults() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(err = ?e, "Docker stats connection failed, retrying in {}s", backoff.as_secs());
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
                continue;
            }
        };
        let connected_at = std::time::Instant::now();

        // Inner loop: reuse this docker client until an error forces reconnect
        loop {
            interval.tick().await;
            if poll_container_stats(&docker, &lifecycle_cache, &stats_cache)
                .await
                .is_err()
            {
                break; // reconnect outer loop
            }
        }

        // Reset backoff only when the inner loop stayed healthy long enough
        // to prove the daemon is actually stable. A fast flap (connect OK,
        // first poll errors) keeps the exponential escalation alive.
        if connected_at.elapsed() >= HEALTHY_DURATION_FOR_RESET {
            backoff = Duration::from_secs(10);
        } else {
            tracing::warn!(
                "Docker poll loop died after {} s — keeping backoff at {} s",
                connected_at.elapsed().as_secs(),
                backoff.as_secs()
            );
        }
    }
}

/// Upper bound on concurrent Docker `stats` calls per poll cycle.
/// Replaces the previous hard `take(50)` cap so every running container is
/// observed, while still bounding pressure on the Docker daemon.
const STATS_POLL_CONCURRENCY: usize = 16;
/// Soft threshold for a warning log when a host runs an unusually large
/// number of containers — makes silent fan-out visible to operators.
const STATS_POLL_WARN_THRESHOLD: usize = 200;

/// Single poll cycle: fetch stats for all running containers.
///
/// Returns `Err(())` when the Docker client should be reconnected. The
/// failure signal fires when **every** stats call in the cycle fails while
/// the lifecycle cache still believes at least one container is running —
/// that combination is characteristic of a dead daemon handle (for example,
/// after `dockerd` restarts without tearing down the socket we connected
/// through). Per-container failures on a healthy daemon (e.g. a container
/// exiting mid-poll and returning 404) don't trigger reconnect since at
/// least one sibling call still succeeds.
async fn poll_container_stats(
    docker: &Docker,
    lifecycle_cache: &DockerCache,
    stats_cache: &DockerStatsCache,
) -> Result<(), ()> {
    let running: Vec<String> = {
        let containers = lifecycle_cache.read().await;
        containers
            .iter()
            .filter(|c| c.state == "running")
            .map(|c| c.container_name.clone())
            .collect()
    };

    if running.len() > STATS_POLL_WARN_THRESHOLD {
        tracing::warn!(
            count = running.len(),
            threshold = STATS_POLL_WARN_THRESHOLD,
            concurrency = STATS_POLL_CONCURRENCY,
            "Docker stats poll is running against a large container set"
        );
    }

    if running.is_empty() {
        let mut cache = stats_cache.write().await;
        cache.clear();
        return Ok(());
    }

    let stats_futures = running.into_iter().map(|name| {
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
                    let pct = (cpu_delta / sys_delta * num_cpus * 100.0) as f32;
                    Some(if pct.is_finite() { pct } else { 0.0 })
                } else {
                    Some(0.0)
                }
            })()
            .unwrap_or(0.0);

            // Memory
            let mem_stats = stats.memory_stats.as_ref();
            let mem_usage_mb = mem_stats.and_then(|m| m.usage).unwrap_or(0) / 1024 / 1024;
            let mem_limit_mb = mem_stats.and_then(|m| m.limit).unwrap_or(0) / 1024 / 1024;

            // Network: sum all interface rx/tx (cumulative)
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

    let results: Vec<Option<DockerContainerStats>> = stream::iter(stats_futures)
        .buffer_unordered(STATS_POLL_CONCURRENCY)
        .collect()
        .await;

    let attempted = results.len();
    let succeeded = results.iter().filter(|r| r.is_some()).count();

    // Atomic swap: build new map first, then replace to avoid readers seeing empty cache
    let mut new_map = HashMap::with_capacity(succeeded);
    for stat in results.into_iter().flatten() {
        new_map.insert(stat.container_name.clone(), stat);
    }
    let mut cache = stats_cache.write().await;
    *cache = new_map;
    drop(cache);

    // Dead-daemon signal: every attempt failed despite the lifecycle cache
    // reporting running containers. Surface this to the outer loop so it
    // reconnects the Docker client instead of polling a stale handle forever.
    if attempted > 0 && succeeded == 0 {
        tracing::warn!(
            attempted,
            "Docker stats: all container stat calls failed in this cycle — reconnecting Docker client"
        );
        return Err(());
    }

    Ok(())
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
