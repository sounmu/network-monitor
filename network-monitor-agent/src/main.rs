//! Network Monitor Agent — HTTP exporter for OS, Docker, and port metrics.
//!
//! This file is intentionally thin: it wires the submodules together and
//! runs the Axum server. Collection logic, authentication, and data
//! structures live in their own modules.

mod auth;
mod docker_cache;
mod gpu;
mod handler;
mod logger;
mod models;
mod ports;
mod sysinfo_collector;

use anyhow::Context;
use axum::Router;
use axum::extract::Query;
use axum::middleware;
use axum::routing::get;
use bollard::Docker;
use std::sync::Arc;
use sysinfo::System;
use tokio::net::TcpListener;
use tokio::sync::RwLock;

use crate::docker_cache::{DockerCache, docker_event_listener, initial_docker_load};
use crate::handler::metrics_handler;
use crate::models::MetricsQuery;

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
    auth::init_decoding_key(jwt_secret.as_bytes())
        .map_err(|e| anyhow::anyhow!("{e} — this should not happen"))?;

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
    // Incrementally updates the cache only when container lifecycle events fire —
    // far cheaper than periodic polling.
    let docker_handle = tokio::spawn(docker_event_listener(docker_cache.clone()));

    // Compress /metrics responses when the caller advertises Accept-Encoding: gzip.
    // bincode is already binary but repeated strings (process names, container images,
    // mount points) still compress 30-50%. The server's reqwest client is built with
    // the `gzip` feature enabled, so it negotiates and decompresses transparently.
    let compression = tower_http::compression::CompressionLayer::new().gzip(true);

    let app = Router::new()
        .route(
            "/metrics",
            get({
                let hostname = hostname.clone();
                let cache = docker_cache.clone();
                move |query: Query<MetricsQuery>| async move {
                    metrics_handler(hostname.clone(), cache.clone(), query).await
                }
            }),
        )
        .layer(compression)
        .layer(middleware::from_fn(auth::auth_middleware));

    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind to port {} — is it already in use?", port))?;

    tracing::info!("Agent exporter running on http://{}", addr);
    tracing::info!("Scrape endpoint: GET http://{}/metrics", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("Agent server encountered a fatal error")?;

    tracing::info!("🛑 Shutting down agent...");
    docker_handle.abort();
    tracing::info!("✅ Agent shutdown complete.");

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                tokio::select! {
                    _ = ctrl_c => tracing::info!("🛑 Received Ctrl+C"),
                    _ = sigterm.recv() => tracing::info!("🛑 Received SIGTERM"),
                }
            }
            Err(e) => {
                tracing::warn!(err = ?e, "⚠️ Failed to install SIGTERM handler, falling back to Ctrl+C only");
                ctrl_c.await.ok();
                tracing::info!("🛑 Received Ctrl+C");
            }
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
        tracing::info!("🛑 Received Ctrl+C");
    }
}
