mod errors;
mod handlers;
mod logger;
mod models;
mod repositories;
mod routes;
mod services;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use axum::http::{HeaderValue, Method};
use tower_http::cors::CorsLayer;

use anyhow::Context;
use models::app_state::{AppState, MetricsStore};
use routes::metrics_routes;
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file (ignored if not present — env vars may come from Docker / systemd)
    dotenvy::dotenv().ok();

    let _guard = logger::init_tracing();
    tracing::info!("🚀 Starting network-monitor-server...");

    // ── Required environment variables ──
    let database_url =
        std::env::var("DATABASE_URL").context("DATABASE_URL environment variable is not set")?;

    let jwt_secret =
        std::env::var("JWT_SECRET").context("JWT_SECRET environment variable is not set")?;
    services::auth::init_encoding_key(&jwt_secret);

    // ── Optional environment variables with defaults ──
    let scrape_interval_secs: u64 = std::env::var("SCRAPE_INTERVAL_SECS")
        .unwrap_or_else(|_| "10".to_string())
        .parse()
        .unwrap_or(10);

    let max_db_connections: u32 = std::env::var("MAX_DB_CONNECTIONS")
        .unwrap_or_else(|_| "10".to_string())
        .parse()
        .unwrap_or(10);

    // ── PostgreSQL connection pool ──
    let db_pool = PgPoolOptions::new()
        .max_connections(max_db_connections)
        .connect(&database_url)
        .await
        .context("Failed to connect to PostgreSQL")?;

    tracing::info!("✅ [DB] Connected to PostgreSQL.");

    // ── Initialize tables (metrics, hosts, alert_configs) ──
    repositories::metrics_repo::init_db(&db_pool)
        .await
        .context("Failed to initialize database tables")?;

    // ── SSE broadcast channel ──
    let sse_buffer: usize = std::env::var("SSE_BUFFER_SIZE")
        .unwrap_or_else(|_| "128".to_string())
        .parse()
        .unwrap_or(128);
    let (sse_tx, _) = tokio::sync::broadcast::channel(sse_buffer);

    // ── Shared application state ──
    let metrics_query_cache = Arc::new(models::app_state::MetricsQueryCache::new(
        std::time::Duration::from_secs(120),
    ));

    let state = Arc::new(AppState {
        store: Arc::new(RwLock::new(MetricsStore::new())),
        http_client: reqwest::Client::new(),
        db_pool,
        scrape_interval_secs,
        sse_tx,
        last_known_status: Arc::new(RwLock::new(HashMap::new())),
        metrics_query_cache: metrics_query_cache.clone(),
    });

    // Background task: evict expired cache entries every 60 seconds
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            metrics_query_cache.evict_expired();
        }
    });

    // ── Pre-populate last_known_status cache from DB ──
    match repositories::hosts_repo::list_hosts(&state.db_pool).await {
        Ok(hosts) => {
            tracing::info!(count = hosts.len(), "📋 [Hosts] Loaded from DB");
            state.pre_populate_status(&hosts);
        }
        Err(e) => tracing::warn!(err = ?e, "⚠️ [Hosts] Failed to load initial hosts"),
    }

    // Build CORS layer from ALLOWED_ORIGINS env var (comma-separated).
    // Falls back to localhost:3001 for local development.
    let cors = {
        let raw = std::env::var("ALLOWED_ORIGINS")
            .unwrap_or_else(|_| "http://localhost:3001".to_string());
        tracing::info!("🌐 [CORS] ALLOWED_ORIGINS = {:?}", raw);
        let origins: Vec<HeaderValue> = raw
            .split(',')
            .filter_map(|s| s.trim().parse::<HeaderValue>().ok())
            .collect();
        tracing::info!("🌐 [CORS] Parsed origins: {:?}", origins);
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::DELETE,
                Method::OPTIONS,
            ])
            .allow_headers(tower_http::cors::Any)
    };
    let compression = tower_http::compression::CompressionLayer::new();

    let app = metrics_routes::create_router(Arc::clone(&state))
        .layer(cors)
        .layer(compression);

    // ── Start background scraper tasks ──
    services::scraper::start_scraper(Arc::clone(&state));
    services::monitor_scraper::spawn_monitor_scraper(Arc::clone(&state));

    let host = std::env::var("SERVER_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = std::env::var("SERVER_PORT").unwrap_or_else(|_| "3000".to_string());
    let bind_addr = format!("{}:{}", host, port);

    let listener = TcpListener::bind(&bind_addr)
        .await
        .context("Failed to bind server port")?;

    tracing::info!("🚀 Server running on http://{}", bind_addr);
    tracing::info!(
        "   Scrape interval: {}s (DB-driven targets)",
        scrape_interval_secs
    );

    axum::serve(listener, app)
        .await
        .context("Server error during execution")?;

    Ok(())
}
