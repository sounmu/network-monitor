//! Event-driven in-memory Docker container cache.
//!
//! An `Arc<RwLock<Vec<DockerContainer>>>` is seeded once at startup and
//! incrementally updated from the Docker Events API, so the `/metrics`
//! handler can respond without making any live Docker API calls.

use crate::models::DockerContainer;
use bollard::Docker;
use bollard::models::EventMessage;
use bollard::query_parameters::{EventsOptions, ListContainersOptions};
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
