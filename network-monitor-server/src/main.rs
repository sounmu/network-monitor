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

    // ── Seed continuous aggregate (cannot run inside migration transaction) ──
    if let Err(e) = sqlx::query(
        "CALL refresh_continuous_aggregate('metrics_5min', NOW() - INTERVAL '3 days', NOW())",
    )
    .execute(&state.db_pool)
    .await
    {
        tracing::warn!(err = ?e, "⚠️ [CA] Failed to seed metrics_5min (may not exist yet)");
    } else {
        tracing::info!("📊 [CA] metrics_5min seeded with last 3 days");
    }

    // ── Pre-populate last_known_status cache from DB ──
    match repositories::hosts_repo::list_hosts(&state.db_pool).await {
        Ok(hosts) => {
            tracing::info!(count = hosts.len(), "📋 [Hosts] Loaded from DB");
            state.pre_populate_status(&hosts);
        }
        Err(e) => tracing::warn!(err = ?e, "⚠️ [Hosts] Failed to load initial hosts"),
    }

    // ── Pre-populate password_changed_at cache for token revocation ──
    match repositories::users_repo::load_password_changed_at(&state.db_pool).await {
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

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("Server error during execution")?;

    // ── Graceful shutdown: cancel background tasks ──
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
