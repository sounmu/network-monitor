mod db;
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
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file (ignored if not present — env vars may come from Docker / systemd)
    dotenvy::dotenv().ok();

    let _guard = logger::init_tracing();
    tracing::info!("🚀 Starting netsentinel-server...");

    // ── Required environment variables ──
    // We intentionally print actionable guidance here instead of letting
    // the first downstream failure (sqlx / jsonwebtoken) surface as a
    // generic connection or HMAC error. Startup is the right moment to
    // tell the operator exactly how to recover.
    let database_url = std::env::var("DATABASE_URL").map_err(|_| {
        anyhow::anyhow!(
            "DATABASE_URL is not set.\n\n\
             NetSentinel stores everything in a single SQLite file. Set:\n\
                 DATABASE_URL=sqlite:///var/lib/netsentinel/netsentinel.db\n\
             (Docker) or\n\
                 DATABASE_URL=sqlite://./data/netsentinel.db\n\
             (local `cargo run`). The directory must exist and be writable;\n\
             the `.db` file itself is created on first boot."
        )
    })?;

    let jwt_secret = std::env::var("JWT_SECRET").map_err(|_| {
        anyhow::anyhow!(
            "JWT_SECRET is not set.\n\n\
             Run `./scripts/bootstrap.sh` from the repo root — it generates a\n\
             32-byte random secret via `openssl rand -hex 32` and writes it\n\
             to .env. The SAME value must appear in every agent's .env."
        )
    })?;
    if jwt_secret.len() < 32 {
        // `str::len()` returns *bytes*, not characters. The original message
        // said "characters" which would mislead anyone passing a multi-byte
        // UTF-8 secret through this check. RFC 7518 §3.2 specifies that the
        // HS256 MAC key SHOULD be ≥ 256 bits (32 bytes), and that threshold
        // is what this guard enforces — so we report bytes and cite the RFC
        // so the number doesn't look arbitrary in a support thread later.
        anyhow::bail!(
            "JWT_SECRET is {} bytes — must be ≥ 32 bytes for adequate HS256 security\n\
             (RFC 7518 §3.2 recommends ≥ 256 bits of keying material).\n\n\
             Regenerate with: `./scripts/bootstrap.sh --force` (this rotates the\n\
             secret and invalidates every previously-issued JWT). Be sure to\n\
             distribute the new value to every agent afterwards.",
            jwt_secret.len()
        );
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
        .unwrap_or_else(|_| "4".to_string())
        .parse()
        .unwrap_or(4);

    // ── Database connection pool ──
    // SQLite is the single embedded backend. `crate::db` owns the pool
    // setup and migration runner so the startup path stays compact.
    let statement_timeout_secs: u64 = std::env::var("DB_STATEMENT_TIMEOUT_SECS")
        .unwrap_or_else(|_| "30".to_string())
        .parse()
        .unwrap_or(30);
    let db_pool = db::connect(&database_url, max_db_connections, statement_timeout_secs).await?;

    // ── Run database migrations ──
    db::run_migrations(&db_pool).await?;

    // ── Background maintenance workers ──
    // TimescaleDB's continuous-aggregate refresh and retention
    // policies live here as plain Tokio tasks. Handles are detached —
    // dropping them on shutdown aborts the tasks, which is the
    // intended behaviour.
    let _rollup = services::rollup_worker::spawn(db_pool.clone());
    let _retention = services::retention_worker::spawn(db_pool.clone());
    tracing::info!("✅ [DB] rollup + retention workers spawned");

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
    // Floor is 128 — empirically the point where a single Lagged event
    // resync (re-snapshotting last_known_status) stays under 100 ms for
    // host counts we expect to support. Env can raise but not lower.
    let env_buffer: usize = std::env::var("SSE_BUFFER_SIZE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(128);
    let auto_buffer = (host_count as usize) * 3;
    let sse_buffer = env_buffer.max(auto_buffer).max(128);
    tracing::info!(sse_buffer, host_count, "📡 [SSE] Broadcast channel sized");
    let (sse_tx, _) = tokio::sync::broadcast::channel(sse_buffer);

    // ── Shared application state ──
    // Cap query caches by both entry count and estimated payload bytes. v0.3.0
    // multiplied per-sample payload size by adding per-core CPU, per-interface
    // network, and per-container docker_stats JSON — count-only caps still let
    // a few long-range entries pin hundreds of MB. The byte budget is shared
    // by policy but enforced separately for full and lightweight chart caches.
    let metrics_cache_max_entries: usize = std::env::var("METRICS_CACHE_MAX_ENTRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let metrics_cache_max_bytes: usize = std::env::var("METRICS_CACHE_MAX_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(32 * 1024 * 1024);
    let metrics_query_cache = Arc::new(models::app_state::MetricsQueryCache::new(
        std::time::Duration::from_secs(120),
        metrics_cache_max_entries,
        metrics_cache_max_bytes,
    ));
    let chart_metrics_query_cache = Arc::new(models::app_state::MetricsQueryCache::new(
        std::time::Duration::from_secs(120),
        metrics_cache_max_entries,
        metrics_cache_max_bytes,
    ));

    let trusted_proxy_count: usize = std::env::var("TRUSTED_PROXY_COUNT")
        .unwrap_or_else(|_| "0".to_string())
        .parse()
        .unwrap_or(0);
    if trusted_proxy_count == 0 {
        tracing::warn!(
            "⚠️ [Security] TRUSTED_PROXY_COUNT=0 — if deploying behind Cloudflare \
             Tunnel or another reverse proxy, set it to 1 so per-IP rate limits \
             key off the original client IP (via CF-Connecting-IP / X-Forwarded-For) \
             instead of the single tunnel IP."
        );
    }

    let sse_ticket_store = Arc::new(services::sse_ticket::SseTicketStore::new());

    // Seed the hosts + alert_configs snapshot before the router goes up so
    // the first scrape cycle reads cached data, not the DB. `empty()` is
    // the placeholder the cell starts with; the real content arrives from
    // `refresh` below.
    let hosts_snapshot = services::hosts_snapshot::empty();
    let monitors_snapshot = services::monitors_snapshot::empty();

    let state = Arc::new(AppState {
        store: Arc::new(RwLock::new(MetricsStore::new())),
        http_client: reqwest::Client::new(),
        db_pool,
        scrape_interval_secs,
        max_db_connections,
        sse_tx,
        last_known_status: Arc::new(RwLock::new(HashMap::new())),
        metrics_query_cache: metrics_query_cache.clone(),
        chart_metrics_query_cache: chart_metrics_query_cache.clone(),
        // Per-IP bucket — default raised from 10 → 30 when the per-username
        // bucket was introduced. A NAT office with several concurrent
        // dashboards needs headroom for the occasional typo'd password from
        // one user without locking out the rest; the per-username bucket
        // (below) keeps targeted brute force at the original tight cap.
        login_rate_limiter: Arc::new(LoginRateLimiter::new(
            std::env::var("LOGIN_RATE_LIMIT_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            std::time::Duration::from_secs(
                std::env::var("LOGIN_RATE_LIMIT_WINDOW_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(300),
            ),
        )),
        login_user_rate_limiter: Arc::new(LoginRateLimiter::new(
            std::env::var("LOGIN_USER_RATE_LIMIT_MAX")
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
        public_api_rate_limiter: Arc::new(LoginRateLimiter::new(
            std::env::var("PUBLIC_API_RATE_LIMIT_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            std::time::Duration::from_secs(
                std::env::var("PUBLIC_API_RATE_LIMIT_WINDOW_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(60),
            ),
        )),
        sse_connections: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        max_sse_connections: std::env::var("MAX_SSE_CONNECTIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&n: &usize| n > 0)
            .unwrap_or(32),
        hosts_snapshot: hosts_snapshot.clone(),
        monitors_snapshot: monitors_snapshot.clone(),
    });

    // Synchronous seed — blocks router startup only as long as two SELECTs
    // take. Avoids a window where the scraper reads an empty snapshot.
    services::hosts_snapshot::refresh(&state.db_pool, &hosts_snapshot).await;
    services::hosts_snapshot::spawn_background_refresher(state.db_pool.clone(), hosts_snapshot);

    // Same pattern for the monitors snapshot — synchronous seed + 60 s
    // background refresher.
    services::monitors_snapshot::refresh(&state.db_pool, &monitors_snapshot).await;
    services::monitors_snapshot::spawn_background_refresher(
        state.db_pool.clone(),
        monitors_snapshot,
    );

    // Background task: evict expired cache entries every 60 seconds
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            metrics_query_cache.evict_expired();
            chart_metrics_query_cache.evict_expired();
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

    // Background task: evict stale rate limiter entries every 5 minutes.
    // Prevents unbounded HashMap growth from unique IPs that never return.
    {
        let login_limiter = Arc::clone(&state.login_rate_limiter);
        let api_limiter = Arc::clone(&state.api_rate_limiter);
        let public_api_limiter = Arc::clone(&state.public_api_rate_limiter);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                login_limiter.evict_stale();
                api_limiter.evict_stale();
                public_api_limiter.evict_stale();
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

    // `metrics_5min` is maintained by `services::rollup_worker`, which
    // was spawned above — no TimescaleDB continuous aggregate policy
    // to configure here.

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
        // Invalid origins were previously dropped silently by `filter_map` —
        // a typo in `.env` would produce a working-looking CORS layer with
        // zero allowed origins and *every* cross-origin request rejected.
        // That looked identical to a "CORS is broken" bug at the browser
        // but had nothing to do with the code path the operator was
        // debugging. Log each reject loudly so the misconfiguration
        // surfaces on startup.
        let origins: Vec<HeaderValue> = raw
            .split(',')
            .filter_map(|s| {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    return None;
                }
                match trimmed.parse::<HeaderValue>() {
                    Ok(v) => Some(v),
                    Err(e) => {
                        tracing::warn!(
                            origin = trimmed,
                            err = %e,
                            "⚠️ [CORS] Ignoring invalid ALLOWED_ORIGINS entry"
                        );
                        None
                    }
                }
            })
            .collect();
        if origins.is_empty() {
            tracing::error!(
                "❌ [CORS] No valid origins parsed from ALLOWED_ORIGINS — \
                 every cross-origin request will be rejected. Check the env \
                 var for typos (protocol required, e.g. https://example.com)."
            );
        }
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

    let app = metrics_routes::create_router(Arc::clone(&state));

    // Mount the pre-built web static bundle when STATIC_ASSETS_DIR points
    // at it. In production this is the single container's /app/static,
    // produced by `next build` with `output: 'export'`. In local dev the
    // env var is typically unset — developers run `npm run dev` on port
    // 3001 against this API on 3000, identical to the old separate-web
    // layout.
    let app = if let Ok(dir_str) = std::env::var("STATIC_ASSETS_DIR") {
        let dir = std::path::PathBuf::from(&dir_str);
        if dir.is_dir() {
            tracing::info!("📦 [Web] Serving static assets from {}", dir.display());
            services::static_assets::mount(app, &dir)
        } else {
            tracing::warn!(
                "STATIC_ASSETS_DIR={} is not a directory — API-only mode",
                dir.display()
            );
            app
        }
    } else {
        tracing::info!("📦 [Web] STATIC_ASSETS_DIR unset — API-only mode (expected in dev)");
        app
    }
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
    headers.insert(
        "Strict-Transport-Security",
        HeaderValue::from_static("max-age=63072000; includeSubDomains"),
    );
    response
}
