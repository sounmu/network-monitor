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
use axum::middleware;
use axum::response::Response;
use tower_http::cors::CorsLayer;

use anyhow::Context;
use models::app_state::{AppState, LoginRateLimiter, MetricsStore};
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
    if jwt_secret.len() < 32 {
        anyhow::bail!("JWT_SECRET must be at least 32 characters for adequate security");
    }
    services::auth::init_encoding_key(&jwt_secret);

    // ── Load password_changed_at cache for token revocation ──
    let password_changed_cache = Arc::new(RwLock::new(HashMap::<i32, i64>::new()));
    services::auth::init_password_changed_cache(Arc::clone(&password_changed_cache));

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
        .min_connections(3)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&database_url)
        .await
        .context("Failed to connect to PostgreSQL")?;

    tracing::info!("✅ [DB] Connected to PostgreSQL.");

    // ── Run database migrations ──
    sqlx::migrate!()
        .run(&db_pool)
        .await
        .context("Failed to run database migrations")?;
    tracing::info!("✅ [DB] Migrations applied successfully.");

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

    let trusted_proxy_count: usize = std::env::var("TRUSTED_PROXY_COUNT")
        .unwrap_or_else(|_| "0".to_string())
        .parse()
        .unwrap_or(0);

    let state = Arc::new(AppState {
        store: Arc::new(RwLock::new(MetricsStore::new())),
        http_client: reqwest::Client::new(),
        db_pool,
        scrape_interval_secs,
        sse_tx,
        last_known_status: Arc::new(RwLock::new(HashMap::new())),
        metrics_query_cache: metrics_query_cache.clone(),
        login_rate_limiter: Arc::new(LoginRateLimiter::new(
            10,
            std::time::Duration::from_secs(300),
        )),
        trusted_proxy_count,
        password_changed_at: password_changed_cache,
    });

    // Background task: evict expired cache entries every 60 seconds
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            metrics_query_cache.evict_expired();
        }
    });

    // ── Ensure continuous aggregate covers full retention period (90 days) ──
    // The refresh policy must match the data retention so long-range queries
    // (7d, 30d, 90d) always find materialized data in the CA.
    let _ = sqlx::query(
        "SELECT remove_continuous_aggregate_policy('metrics_5min', if_not_exists => TRUE)",
    )
    .execute(&state.db_pool)
    .await;
    if let Err(e) = sqlx::query(
        "SELECT add_continuous_aggregate_policy('metrics_5min', \
             start_offset => INTERVAL '90 days', \
             end_offset   => INTERVAL '5 minutes', \
             schedule_interval => INTERVAL '5 minutes', \
             if_not_exists => TRUE)",
    )
    .execute(&state.db_pool)
    .await
    {
        tracing::warn!(err = ?e, "⚠️ [CA] Failed to update refresh policy");
    }

    // Seed the CA with existing data in the background (non-blocking startup).
    // On first run this materializes up to 90 days; subsequent starts are fast.
    {
        let ca_pool = state.db_pool.clone();
        tokio::spawn(async move {
            if let Err(e) = sqlx::query(
                "CALL refresh_continuous_aggregate('metrics_5min', NOW() - INTERVAL '90 days', NOW())",
            )
            .execute(&ca_pool)
            .await
            {
                tracing::warn!(err = ?e, "⚠️ [CA] Failed to seed metrics_5min (may not exist yet)");
            } else {
                tracing::info!("📊 [CA] metrics_5min refreshed (90-day window)");
            }
        });
    }

    // ── Pre-populate caches from DB (parallel) ──
    let (hosts_result, password_result) = tokio::join!(
        repositories::hosts_repo::list_hosts(&state.db_pool),
        repositories::users_repo::load_password_changed_at(&state.db_pool),
    );

    match hosts_result {
        Ok(hosts) => {
            tracing::info!(count = hosts.len(), "📋 [Hosts] Loaded from DB");
            state.pre_populate_status(&hosts);
        }
        Err(e) => tracing::warn!(err = ?e, "⚠️ [Hosts] Failed to load initial hosts"),
    }

    match password_result {
        Ok(map) => {
            if let Ok(mut cache) = state.password_changed_at.write() {
                *cache = map;
            }
            tracing::info!("🔐 [Auth] Password change timestamps loaded");
        }
        Err(e) => {
            tracing::warn!(err = ?e, "⚠️ [Auth] Failed to load password timestamps (column may not exist yet)")
        }
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
        .layer(middleware::map_response(add_api_version_header))
        .layer(cors)
        .layer(compression);

    // ── Start background scraper tasks ──
    let scraper_handle = services::scraper::start_scraper(Arc::clone(&state));
    let monitor_handle = services::monitor_scraper::spawn_monitor_scraper(Arc::clone(&state));

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

    // Graceful shutdown with drain timeout for long-lived connections (SSE).
    // When the signal fires, a 5-second kill timer spawns. If the drain
    // completes before the timer, cleanup runs normally. If not,
    // process::exit forces termination (DB handles connection cleanup).
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        shutdown_signal().await;
        tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            tracing::warn!("⚠️ Graceful drain timed out after 5s — forcing exit");
            std::process::exit(0);
        });
    })
    .await
    .context("Server error during execution")?;

    // ── Cleanup background tasks and DB pool ──
    tracing::info!("🛑 Shutting down background tasks...");
    scraper_handle.abort();
    monitor_handle.abort();
    state.db_pool.close().await;
    tracing::info!("✅ Shutdown complete.");

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => tracing::info!("🛑 Received Ctrl+C"),
            _ = sigterm.recv() => tracing::info!("🛑 Received SIGTERM"),
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
        tracing::info!("🛑 Received Ctrl+C");
    }
}

async fn add_api_version_header(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert("X-API-Version", HeaderValue::from_static("1"));
    headers.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    headers.insert(
        "Referrer-Policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    response
}
