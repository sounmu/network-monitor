mod errors;
mod handlers;
mod logger;
mod models;
mod repositories;
mod request_id;
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

    // ── Unified token revocation cache ──
    // Two mechanisms feed into this cache, both meaning "JWTs with `iat`
    // older than the stored value are rejected":
    //   (a) password changes         — `users.password_changed_at`
    //   (b) explicit logouts / admin — `users.tokens_revoked_at`
    // The cache is empty at first and populated below, once the DB pool and
    // migrations are both ready.
    let token_revocation_cache: Arc<RwLock<HashMap<i32, i64>>> =
        Arc::new(RwLock::new(HashMap::new()));
    services::auth::init_token_revocation_cache(Arc::clone(&token_revocation_cache));

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
    // statement_timeout prevents any single query from holding a connection
    // indefinitely. The 30-second default is generous enough for the heaviest
    // analytics queries (90-day CA refresh) while still catching runaway
    // transactions before the pool is exhausted. Configurable via env var.
    let statement_timeout_secs: u64 = std::env::var("DB_STATEMENT_TIMEOUT_SECS")
        .unwrap_or_else(|_| "30".to_string())
        .parse()
        .unwrap_or(30);
    let connect_options: sqlx::postgres::PgConnectOptions = database_url
        .parse::<sqlx::postgres::PgConnectOptions>()
        .context("Invalid DATABASE_URL")?
        .options([(
            "statement_timeout",
            format!("{}s", statement_timeout_secs).as_str(),
        )]);
    let db_pool = PgPoolOptions::new()
        .max_connections(max_db_connections)
        .min_connections(3)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect_with(connect_options)
        .await
        .context("Failed to connect to PostgreSQL")?;
    tracing::info!(
        statement_timeout_secs,
        "⏱️ [DB] Query statement_timeout set"
    );

    tracing::info!("✅ [DB] Connected to PostgreSQL.");

    // ── Run database migrations ──
    sqlx::migrate!()
        .run(&db_pool)
        .await
        .context("Failed to run database migrations")?;
    tracing::info!("✅ [DB] Migrations applied successfully.");

    // ── SSE broadcast channel ──
    // Size the buffer to hold at least one full scrape cycle (N hosts × 2 events each)
    // so slow consumers don't get Lagged errors under normal load.
    let host_count: i64 = match sqlx::query_scalar("SELECT COUNT(*) FROM hosts")
        .fetch_one(&db_pool)
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(err = ?e, "⚠️ [SSE] Failed to count hosts, using default buffer size");
            0
        }
    };
    let env_buffer: usize = std::env::var("SSE_BUFFER_SIZE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let auto_buffer = (host_count as usize) * 3;
    let sse_buffer = env_buffer.max(auto_buffer).max(128);
    tracing::info!(sse_buffer, host_count, "📡 [SSE] Broadcast channel sized");
    let (sse_tx, _) = tokio::sync::broadcast::channel(sse_buffer);

    // ── Shared application state ──
    let metrics_query_cache = Arc::new(models::app_state::MetricsQueryCache::new(
        std::time::Duration::from_secs(120),
    ));

    let trusted_proxy_count: usize = std::env::var("TRUSTED_PROXY_COUNT")
        .unwrap_or_else(|_| "0".to_string())
        .parse()
        .unwrap_or(0);

    let sse_ticket_store = Arc::new(services::sse_ticket::SseTicketStore::new());

    let state = Arc::new(AppState {
        store: Arc::new(RwLock::new(MetricsStore::new())),
        http_client: reqwest::Client::new(),
        db_pool,
        scrape_interval_secs,
        sse_tx,
        last_known_status: Arc::new(RwLock::new(HashMap::new())),
        metrics_query_cache: metrics_query_cache.clone(),
        login_rate_limiter: Arc::new(LoginRateLimiter::new(
            std::env::var("LOGIN_RATE_LIMIT_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            std::time::Duration::from_secs(
                std::env::var("LOGIN_RATE_LIMIT_WINDOW_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(300),
            ),
        )),
        trusted_proxy_count,
        token_revocation_cutoffs: Arc::clone(&token_revocation_cache),
        sse_ticket_store: Arc::clone(&sse_ticket_store),
        api_rate_limiter: Arc::new(LoginRateLimiter::new(
            std::env::var("API_RATE_LIMIT_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(200),
            std::time::Duration::from_secs(
                std::env::var("API_RATE_LIMIT_WINDOW_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(60),
            ),
        )),
    });

    // Background task: evict expired cache entries every 60 seconds
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            metrics_query_cache.evict_expired();
        }
    });

    // Background task: evict expired SSE tickets every 30 seconds.
    // Lazy eviction on `consume` already handles the hot path, but a periodic
    // sweep prevents unbounded growth if tickets are issued but never redeemed.
    {
        let store = Arc::clone(&sse_ticket_store);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                store.evict_expired();
            }
        });
    }

    // Background task: delete `refresh_tokens` rows that have been expired
    // for more than a week. Keeps the table bounded without losing recent
    // forensic history (admins may want to inspect issued_at / ip on a
    // recent session).
    {
        let pool = state.db_pool.clone();
        tokio::spawn(async move {
            loop {
                // Run once per hour — churn is low and DELETE is cheap.
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                match repositories::refresh_tokens_repo::delete_expired(&pool).await {
                    Ok(0) => {}
                    Ok(n) => tracing::info!(count = n, "🧹 [Auth] Evicted expired refresh tokens"),
                    Err(e) => tracing::warn!(err = ?e, "⚠️ [Auth] refresh_tokens cleanup failed"),
                }
            }
        });
    }

    // ── Continuous aggregate refresh policy ──
    // The periodic policy only needs to cover recent data (3 days) — enough to
    // handle late-arriving inserts and reprocessing. Historical data (up to 90
    // days) is seeded once on startup via the explicit CALL below, so the policy
    // doesn't need to re-scan the entire retention window every 5 minutes.
    // Previously this was set to 90 days, causing unnecessary memory pressure
    // as TimescaleDB loaded metadata for all compressed chunks on each refresh.
    let _ = sqlx::query(
        "SELECT remove_continuous_aggregate_policy('metrics_5min', if_not_exists => TRUE)",
    )
    .execute(&state.db_pool)
    .await;
    if let Err(e) = sqlx::query(
        "SELECT add_continuous_aggregate_policy('metrics_5min', \
             start_offset => INTERVAL '3 days', \
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
    let (hosts_result, password_result, revoked_result) = tokio::join!(
        repositories::hosts_repo::list_hosts(&state.db_pool),
        repositories::users_repo::load_password_changed_at(&state.db_pool),
        repositories::users_repo::load_tokens_revoked_at(&state.db_pool),
    );

    match hosts_result {
        Ok(hosts) => {
            tracing::info!(count = hosts.len(), "📋 [Hosts] Loaded from DB");
            state.pre_populate_status(&hosts);
        }
        Err(e) => tracing::warn!(err = ?e, "⚠️ [Hosts] Failed to load initial hosts"),
    }

    // Merge password_changed_at and tokens_revoked_at into the unified cutoff
    // cache. For each user we keep the **later** of the two timestamps, since
    // the semantic is "tokens with iat older than this are rejected".
    {
        let mut cutoffs = match state.token_revocation_cutoffs.write() {
            Ok(c) => c,
            Err(poisoned) => poisoned.into_inner(),
        };
        match password_result {
            Ok(map) => {
                for (uid, ts) in map {
                    cutoffs.insert(uid, ts);
                }
                tracing::info!("🔐 [Auth] password_changed_at timestamps loaded");
            }
            Err(e) => tracing::warn!(
                err = ?e,
                "⚠️ [Auth] Failed to load password_changed_at (column may be missing pre-migration)"
            ),
        }
        match revoked_result {
            Ok(map) => {
                let count = map.len();
                for (uid, ts) in map {
                    cutoffs
                        .entry(uid)
                        .and_modify(|existing| {
                            if ts > *existing {
                                *existing = ts;
                            }
                        })
                        .or_insert(ts);
                }
                tracing::info!(
                    revoked_user_count = count,
                    "🔐 [Auth] tokens_revoked_at timestamps merged"
                );
            }
            Err(e) => tracing::warn!(
                err = ?e,
                "⚠️ [Auth] Failed to load tokens_revoked_at (column may be missing pre-migration)"
            ),
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
            .allow_headers([
                axum::http::header::AUTHORIZATION,
                axum::http::header::CONTENT_TYPE,
                axum::http::header::ACCEPT,
            ])
            // Required for the httpOnly refresh cookie to be sent/received
            // by the browser on cross-origin fetch calls with
            // `credentials: "include"`.
            .allow_credentials(true)
    };
    let compression = tower_http::compression::CompressionLayer::new();

    let app = metrics_routes::create_router(Arc::clone(&state))
        .layer(middleware::map_response(add_api_version_header))
        .layer(middleware::from_fn(request_id::request_id))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            request_id::api_rate_limit,
        ))
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
