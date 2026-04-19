# NetSentinel Monorepo

## Structure
- `netsentinel-agent/`: Rust daemon. Scrapes OS (CPU, memory, disk, processes, temperatures, GPU), Docker, port metrics (`tokio::join!`).
- `netsentinel-server/`: Rust/Axum backend. Aggregates metrics, REST API, multi-channel alerts (Discord/Slack/Email), **and serves the web static bundle** (v0.3.6+).
- `netsentinel-web/`: Next.js dashboard. Built with `output: 'export'` so the bundle is plain HTML/JS/CSS; in production it is baked into the server image at `/app/static` and served by `tower-http::ServeDir`. Local dev still runs `npm run dev` on port 3001 with full HMR — same authoring loop as before.

## CRITICAL AI INSTRUCTIONS
- **Next.js 16.2.1**: Contains breaking changes. MUST read `node_modules/next/dist/docs/` before modifying frontend code.
- **Rust edition 2024**: `Cargo.toml` uses `edition = "2024"` (stable since Rust 1.85). `let chains` (`if let ... && cond`) are available and enforced by clippy — use them instead of nested `if let`.
- **CI uses `cargo clippy -- -D warnings`**: All clippy warnings are errors. Fix lints before committing. Common traps: use `.is_none_or()` instead of `.map_or(true, ...)`, use `std::slice::from_ref(&val)` instead of `&[val.clone()]`.
- **Rust formatting**: After editing server or agent Rust files, always run `cargo fmt` in the respective directory before committing. CI enforces `cargo fmt --check` and will reject unformatted code.
- **bincode serialization**: Agent ↔ Server uses bincode (not JSON). When adding fields to `AgentMetrics` or nested structs, add `#[serde(default)]` for backward compat. Field order matters — add new fields at the end.
- **Agent version compatibility**: `AgentMetrics.agent_version` is set from `env!("CARGO_PKG_VERSION")`. Server checks against `MIN_AGENT_VERSION` in `scraper.rs` — logs warning but never rejects old agents. Graceful degradation is mandatory for self-hosters.
- **Documentation sync**: When modifying features, APIs, DB schema, or config — update `README.md`, `AGENTS.md`, and `CONTRIBUTING.md` accordingly. These files must always reflect the current state of the project.
- **DB migrations**: Schema changes go in numbered SQL files under `netsentinel-server/migrations/` (e.g., `005_add_column.sql`). Use `CREATE ... IF NOT EXISTS` and `IF NOT EXISTS` for idempotency. Migrations run automatically on server startup via `sqlx::migrate!()`. Never modify existing migration files — always create new ones.
- **Adding a new metric end-to-end**:
  1. **Agent**: Add field to `AgentMetrics` struct (with `#[serde(default)]`, at the end). Collect the value in the scrape loop.
  2. **Server — insert**: Add column to `metrics` table (new migration). Update the batch INSERT query in `metrics_repo.rs`.
  3. **Server — CA**: Create a new migration that DROP + CREATE `metrics_5min` (TimescaleDB CAs cannot be ALTERed). Use the appropriate aggregate: scalar → `AVG()`, cumulative counter → `MAX()`, JSONB snapshot → `last(col, timestamp)`.
  4. **Server — query**: Update `fetch_metrics_range` in `metrics_repo.rs` — replace `NULL::jsonb` with the real column in both CA branches (6h–14d direct, >14d re-aggregate with `last(col, bucket)`).
  5. **Server — SSE**: If the metric should appear in real-time, add it to the `SseMetricsPayload` struct and the SSE broadcast in `scraper.rs`.
  6. **Frontend**: Add the chart/component in `TimeSeriesChart.tsx` or a new component. Use `--chart-N` CSS variable for color.
  7. **Docs**: Update `AGENTS.md` (DB Schema, Agent Metrics) and `README.md`.
- **Removing a metric**: Do NOT drop columns from `metrics` table (breaks historical queries). Instead: stop collecting in Agent, remove from SSE payload, remove frontend chart. The CA can be rebuilt without the column in a future migration if storage savings are needed.

## Architecture & Data Flow
- **Scraping**: Server schedules each host by its DB-backed `scrape_interval_secs` (JWT auth required, default 10s). `SCRAPE_INTERVAL_SECS` is now a fallback if a host row has no valid interval. Metrics batch-inserted in a single query per scrape sweep (not per-host).
- **Storage**: TimescaleDB hypertable + `metrics_5min` continuous aggregate + 10-min rolling in-memory cache. Metrics deleted after 90 days. Compression enabled on `alert_history`, `http_monitor_results`, `ping_results` (7-day threshold).
- **Continuous Aggregate** (`metrics_5min`): 5-minute pre-aggregated view of raw metrics. Scalar columns: AVG cpu/memory/load, MAX network bytes, bool_and is_online, COUNT sample_count. JSONB snapshot columns: `last(disks/temperatures/gpus/docker_stats, timestamp)` — preserves per-element granularity (per-disk, per-container, per-sensor). Refresh policy: 3-day start_offset, every 5 minutes. Seeded in background on startup (non-blocking). Long-range queries (6h+) read from CA instead of raw table for ~30-300x fewer rows. **Pattern for adding new metrics to CA:** scalar → `AVG()`, cumulative counter → `MAX()`, JSONB snapshot → `last(col, timestamp)`.
- **Query routing** (`fetch_metrics_range`): ≤6h → raw 10s, 6h-14d → CA 5-min direct, >14d → CA 15-min re-aggregate. All time-range queries are cached (TTL 120s, 5-min key rounding). Frontend rounds timestamps to nearest minute for cache alignment.
- **Frontend**: SWR 5s polling to Server REST API. SSE for live status and metrics (rAF-batched updates — 100 events/cycle → 1 setState). SSE `metrics` event now includes disks, temperatures, and docker_stats for real-time chart updates.
- **Config**: DB-driven. `hosts` table for agent config, `alert_configs` for alert rules, `notification_channels` for alert delivery. No config.yaml at runtime.
- **Network**: Zero Trust via Cloudflare Tunnel. DB is internal-only (`backend-internal` Docker network, auto-created by compose). The legacy `shared-network` external-bridge requirement was dropped in v0.3.6 — operators who want to put the stack on an overlay add it via a compose override (see `docs/DEPLOYMENT.md`).
- **Timezone**: Seoul (`chrono-tz`).
- **CORS**: Controlled by `ALLOWED_ORIGINS` env var (comma-separated). Default: `http://localhost:3001` (local dev). In production, since the web bundle and the API share one origin, CORS is effectively unused in the browser path — the header still guards any third-party embed. Do NOT revert to `CorsLayer::permissive()`.
- **Web tier (v0.3.6+)**: `netsentinel-web` is compiled to `output: 'export'` and baked into the server image at `/app/static` by the multi-stage Dockerfile. Axum's `services::static_assets::mount` wires `tower-http::ServeDir` so `/` serves `index.html`, every known route has its own static HTML (`/agents/index.html`, `/alerts/index.html`, …), and the host detail page is the static `/host/index.html` shell. The active `host_key` is passed as `?key=<host:port>` and read client-side via `useSearchParams()`. The server reads `STATIC_ASSETS_DIR` at startup; unset = API-only mode (expected in dev, where `npm run dev` on port 3001 handles the browser route instead).
- **Frontend ↔ Server origin contract**: `NEXT_PUBLIC_API_URL` is baked into the web bundle at build time and tells the browser where to fetch `/api/*`. With the single-container production layout it defaults to **empty** — same-origin as the static bundle, which is the simplest path. In reverse-proxy setups where the UI and API live under different hostnames, set `NEXT_PUBLIC_API_URL=https://api.example.com` at build and list both hostnames in `ALLOWED_ORIGINS` (`https://dashboard.example.com,https://api.example.com`). Changing either value requires a server image rebuild (`docker compose up -d --build server`) — env-only restart does nothing because the URL is embedded in JS at build time.
- **Graceful shutdown**: SIGTERM/SIGINT → 5-second drain timeout for long-lived SSE connections → `process::exit(0)` if drain stalls.
- **Server startup**: Hosts + password cache loaded in parallel via `tokio::join!`. CA refresh deferred to background task. DB pool: `min_connections(3)`, `acquire_timeout(5s)`.
- **In-memory caching**: `MetricsQueryCache` stores `Arc<Vec<MetricsRow>>` for cheap clone on cache hits. Bounded by both TTL (120 s) and entry count (`METRICS_CACHE_MAX_ENTRIES`, default 200) — oldest-inserted entries are evicted once the cap is hit, with expired entries purged first so the scan only sees live rows. `last_known_status` and `MetricsStore.hosts` are cleaned up on host deletion (prevents memory leak).
- **`HostsSnapshot` cache** (`services/hosts_snapshot.rs`): `Arc<RwLock<Arc<HostsSnapshot>>>` holding the `hosts` + resolved `alert_configs` map. Scraper reads via `Arc::clone` (no DB round-trip) instead of the per-cycle `SELECT * FROM hosts` + `SELECT * FROM alert_configs` this replaced. Refresh protocol: synchronously after every mutation handler (`POST/PUT/DELETE /api/hosts/*`, `PUT/DELETE /api/alert-configs/*`) and every 60 s via background tick as a backstop. Snapshot swap is atomic under the write guard — readers always see a coherent view.
- **SSE optimization**: `handle_down()` uses a 3-phase lock pattern (Phase 1: `store.write()` for alert state, Phase 2: `last_known_status.write()` for status update + broadcast, Phase 3: async alert delivery — no locks held). Offline status broadcasts are throttled to 2-min intervals (same as online path) to prevent N×cycle redundant events. SSE broadcast buffer is dynamically sized: `max(SSE_BUFFER_SIZE env, host_count × 3, 128)`. Alert config loading uses `HashMap<(host_key, metric_type)>` for O(1) lookup. SSE tickets are single-use, and the stream re-checks the caller's revocation cutoff every 15s so logout/password change/admin revoke closes live streams.

## Identification: `host_key` (target URL based)
- **`host_key`**: Target URL (e.g. `192.168.1.10:9101`) — unique identifier across the entire stack. Prevents hostname collisions when multiple agents share the same OS hostname.
- **`display_name`**: Agent-reported hostname. UI display only, never used as a key.
- **Where used**: SSE payloads, DB columns, REST API path (`/api/metrics/{host_key}`), frontend host-detail route (`/host/?key={host_key}`), in-memory caches (`last_known_status` keyed by `host_key`). `HostStatusPayload` also carries `scrape_interval_secs` so the web status-aging logic matches the server scheduler.
- **Pre-populate**: On server startup, all hosts from DB are seeded into `last_known_status` as `is_online: false`. SSE clients see every configured host immediately.
- **Offline SSE**: `handle_down()` in `scraper.rs` broadcasts `is_online: false`. Frontend renders hosts from `statusMap` (not `metricsMap`) so offline hosts always appear.

## DB Schema
- **`metrics`**: TimescaleDB hypertable. Columns: `id`, `host_key`, `display_name`, `is_online`, `cpu/memory/load`, `networks/docker_containers/ports/disks/processes/temperatures/gpus` (JSONB), `timestamp`. Index: `(host_key, timestamp DESC)`. 90-day retention. 1-day chunks, compression after 7 days.
- **`metrics_5min`** (CA): TimescaleDB continuous aggregate over `metrics`. 5-minute buckets. Scalar columns: `cpu_usage_percent` (AVG), `memory_usage_percent` (AVG), `load_1/5/15min` (AVG), `is_online` (bool_and), `sample_count` (COUNT), `total_rx_bytes` (MAX), `total_tx_bytes` (MAX). JSONB snapshot columns: `disks`, `temperatures`, `gpus`, `docker_stats` (all via `last(col, timestamp)`). Index: `(host_key, bucket DESC)`. Refresh: 3-day start_offset, 5-min schedule.
- **`hosts`**: Agent registry. PK: `host_key`. Columns: `display_name`, `scrape_interval_secs`, `load_threshold`, `ports` (INT[]), `containers` (TEXT[]), `os_info`, `cpu_model`, `memory_total_mb`, `boot_time`, `ip_address`, `system_info_updated_at`, `created_at`, `updated_at`. System info columns populated by `/system-info` endpoint on reconnection + every 24h.
- **`alert_configs`**: Alert rules (global + per-host override). FK: `host_key → hosts`. Columns: `metric_type` (cpu/memory/disk), `enabled`, `threshold`, `sustained_secs`, `cooldown_secs`. NULL `host_key` = global default.
- **`notification_channels`**: Alert delivery targets. Columns: `name`, `channel_type` (discord/slack/email), `enabled`, `config` (JSONB — webhook_url or SMTP settings), `created_at`, `updated_at`.
- **`alert_history`**: Alert event log. TimescaleDB hypertable, 90-day retention, compression after 7 days (segmentby `host_key`). Columns: `host_key`, `alert_type`, `message`, `created_at`. Indexes: `(host_key, created_at DESC)`, `(created_at DESC)`.
- **`dashboard_layouts`**: Per-user dashboard widget configuration. Columns: `user_id` (UNIQUE), `widgets` (JSONB array of widget configs), `updated_at`.
- **`users`**: User accounts. Columns: `username` (UNIQUE), `password_hash` (argon2), `role` (admin/viewer), `password_changed_at` (for token revocation), `created_at`, `updated_at`.
- **`http_monitors`**: External HTTP endpoint monitors. Columns: `name`, `url`, `method`, `expected_status`, `interval_secs`, `timeout_ms`, `enabled`.
- **`http_monitor_results`**: Check results. TimescaleDB hypertable, 90-day retention, compression after 7 days (segmentby `monitor_id`). FK: `monitor_id → http_monitors`. Columns: `status_code`, `response_time_ms`, `error`, `created_at`.
- **`ping_monitors`**: Network host reachability monitors. Columns: `name`, `host`, `interval_secs`, `timeout_ms`, `enabled`.
- **`ping_results`**: Check results. TimescaleDB hypertable, 90-day retention, compression after 7 days (segmentby `monitor_id`). FK: `monitor_id → ping_monitors`. Columns: `rtt_ms`, `success`, `error`, `created_at`.

## REST API Endpoints
- `POST /api/auth/login` — Login (no auth required)
- `POST /api/auth/setup` — Create initial admin (no auth, only when empty)
- `GET /api/auth/me` — Current user info
- `GET /api/auth/status` — Check if setup needed (no auth)
- `PUT /api/auth/password` — Change current user's password
- `POST /api/auth/logout` — Revoke every JWT issued to the caller (stamps `users.tokens_revoked_at`)
- `POST /api/auth/refresh` — Rotate the httpOnly refresh cookie (`nm_refresh`, Path=/api/auth, SameSite=Strict, `Secure` by default unless `COOKIE_SECURE=false`) and mint a fresh short-lived access JWT. Single-flight on the client to avoid concurrent-call reuse-detection false positives. No auth header required — the refresh cookie is the credential.
- `POST /api/auth/sse-ticket` — Mint a single-use opaque ticket for `/api/stream` (see SSE ticket section)
- `POST /api/admin/users/{id}/revoke-sessions` — Admin kill-switch; force-revoke every session for a target user
- `GET /api/health` — Health check, verifies DB connectivity (no auth)
- `GET /metrics` — Prometheus-compatible metrics export (text/plain format). Requires `Authorization: Bearer <token>` when `METRICS_TOKEN` is set; otherwise returns 401 unless `ALLOW_UNAUTHENTICATED_METRICS=true` (constant-time compared).
- `GET /api/dashboard` — Get user's dashboard widget layout
- `PUT /api/dashboard` — Save user's dashboard widget layout
- `GET /api/metrics/{host_key}` — Metrics by host_key (time range via `?start=&end=`)
- `POST /api/metrics/batch` — Batch metrics for multiple hosts (body: `{host_keys, start, end}`, max 50 keys)
- `GET /api/hosts` — All host summaries (is_online included, from hosts LEFT JOIN metrics)
- `POST /api/hosts` — Register new host
- `PUT /api/hosts/{host_key}` — Update host config
- `DELETE /api/hosts/{host_key}` — Delete host
- `GET /api/alert-configs` — Global alert defaults
- `PUT /api/alert-configs` — Update global alert defaults
- `GET /api/alert-configs/{host_key}` — Host-specific alert overrides
- `PUT /api/alert-configs/{host_key}` — Upsert host alert overrides
- `DELETE /api/alert-configs/{host_key}` — Delete host overrides (revert to global)
- `GET /api/notification-channels` — List all notification channels
- `POST /api/notification-channels` — Create notification channel
- `PUT /api/notification-channels/{id}` — Update notification channel
- `DELETE /api/notification-channels/{id}` — Delete notification channel
- `POST /api/notification-channels/{id}/test` — Send test notification
- `GET /api/alert-history?host_key=&limit=&offset=` — Alert event log
- `GET /api/uptime/{host_key}?days=` — Daily uptime breakdown (TimescaleDB time_bucket)
- `GET /api/http-monitors` — List HTTP monitors
- `POST /api/http-monitors` — Create HTTP monitor
- `GET /api/http-monitors/summaries` — HTTP monitor summaries (latest result + 24h uptime)
- `PUT /api/http-monitors/{id}` — Update HTTP monitor
- `DELETE /api/http-monitors/{id}` — Delete HTTP monitor
- `GET /api/http-monitors/{id}/results?limit=` — HTTP monitor check results
- `GET /api/ping-monitors` — List Ping monitors
- `POST /api/ping-monitors` — Create Ping monitor
- `GET /api/ping-monitors/summaries` — Ping monitor summaries
- `PUT /api/ping-monitors/{id}` — Update Ping monitor
- `DELETE /api/ping-monitors/{id}` — Delete Ping monitor
- `GET /api/ping-monitors/{id}/results?limit=` — Ping monitor check results
- `GET /api/public/status` — Public status page data (no auth required)
- `GET /api/stream?key=` — SSE stream (events: `metrics`, `status`)

## Alert System
- **Alert types**: CpuOverload/Recovery, MemoryOverload/Recovery, DiskOverload/Recovery, LoadOverload/Recovery, PortDown/Recovery, HostDown/Recovery, MonitorDown/Recovery.
- **Disk alerts**: Immediate threshold check (no sustained window), per-mount-point tracking, 300s default cooldown.
- **Monitor failure alerting**: HTTP/Ping monitor failures trigger MonitorDown notifications with 5-min cooldown. Recovery sent when monitor succeeds again.
- **Multi-channel delivery**: DB-managed channels (Discord, Slack, Email).
- **Alert history**: All alerts logged to `alert_history` table after delivery (best-effort). TimescaleDB hypertable with 90-day retention.
- **Slack**: Markdown format conversion (`**bold**` → `*bold*`).
- **Email**: lettre crate, STARTTLS, strips markdown for plain text body.

## Agent Metrics Collected
- **CPU**: Global usage % (200ms delta measurement via sysinfo) + per-core usage percentages
- **Memory**: Total/used MB, usage %
- **Disk**: Per-partition name, mount_point, total_gb, available_gb, usage_percent, read/write_bytes_per_sec (I/O delta)
- **Processes**: Top 10 by CPU usage via `select_nth_unstable` O(n) selection (pid, name, cpu_usage, memory_mb)
- **Temperatures**: All available sensors via sysinfo Components (label, temperature_c)
- **GPU**: NVIDIA via nvml-wrapper (name, usage %, VRAM used/total, temperature, power_watts, frequency_mhz). Empty vec if no NVIDIA driver.
- **Network**: Cumulative RX/TX bytes across physical interfaces (virtual/loopback filtered) + per-interface breakdown
- **Load Average**: 1/5/15 min
- **Docker**: Container name, image, state, status (event-driven cache via bollard) + per-container CPU%, memory_usage_mb, memory_limit_mb, net_rx/tx_bytes
- **Ports**: Async parallel TCP connect test to configurable ports (100ms timeout per port, all ports checked concurrently, capped at 100 ports)
- **System Info**: OS, CPU model, total RAM, boot time, IP address — fetched on reconnection + every 24h via `/system-info` endpoint

## External Monitoring (server-side probes)
- **HTTP monitors**: Server sends HTTP requests (GET/POST/HEAD) to configured URLs at configurable intervals. Stores status_code, response_time_ms, error. Expected status mismatch = error.
- **Ping monitors**: TCP connect test (not ICMP — avoids requiring root/CAP_NET_RAW). Stores rtt_ms, success, error. Default target port 80.
- **Background scraper**: `monitor_scraper.rs` runs every 10s, tracks per-monitor last_checked timestamps, respects individual intervals.
- **Summaries**: LATERAL JOIN queries compute latest result + 24h uptime % in a single query.

## Prometheus Export
- `GET /metrics` — auth required by default; anonymous access only when `ALLOW_UNAUTHENTICATED_METRICS=true`
- Format: Prometheus text exposition format (`text/plain; version=0.0.4`)
- Exports: `netmonitor_host_online`, `netmonitor_cpu_usage_percent`, `netmonitor_memory_usage_percent` as gauges
- Labels: `host_key`, `display_name`
- Data source: in-memory store (real-time, not DB query)

## PWA (Frontend)
- `manifest.json` in public/ — standalone display, theme color #3B82F6
- `sw.js` — service worker caches immutable same-origin static assets only (`/_next/static`, scripts, styles, fonts, images, `manifest.json`). It does not cache App Router documents, RSC payloads, or `/api/*`, and it is unregistered on logout.
- `ServiceWorkerRegistration` component in layout.tsx
- Apple Web App meta tags for iOS home screen support

## Error Handling (`AppError`)
All handlers return `AppError`. Do not return raw `StatusCode` from handlers or extractors.
- `Internal` → 500 (returns generic "Internal server error" — never exposes DB/system details to client), `NotFound` → 404, `BadRequest` → 400, `Unauthorized` → 401, `TooManyRequests` → 429, `Conflict` → 409
- Duplicate `host_key` → `Conflict` (409), not `BadRequest`.
- Login rate limit → `TooManyRequests` (429), not `BadRequest`.
- `UserGuard`/`AdminGuard` extractor `type Rejection = AppError` — returns `AppError::Unauthorized`.

## Input Validation (server)
Validation lives in handlers, not repositories. Rules:
- **Hosts** (`hosts_handler.rs`): `host_key` must be `host:port` format (no `/`, `?`, `#`, `@`), max 255 chars. `display_name` max 255 chars. `scrape_interval_secs` > 0. `ports` 1–65535 (`validate_ports()`)
- **Alert configs** (`alert_configs_handler.rs`): `threshold` 0–100, `sustained_secs` 0–3600, `cooldown_secs` 0–86400
- **HTTP monitors** (`http_monitors_handler.rs`): URL must have http/https scheme + SSRF validation (private IP blocked), `interval_secs` 10–3600, `timeout_ms` 1000–30000, `expected_status` 100–599
- **Ping monitors** (`ping_monitors_handler.rs`): host required + SSRF validation (private IP blocked), `interval_secs` 10–3600, `timeout_ms` 1000–30000
- **Notification channels** (`notification_channels_handler.rs`): `webhook_url` required for Discord/Slack + SSRF validation (HTTPS only, private IP blocked). Email requires `smtp_host`, `smtp_port` (default 587), `smtp_user`, `smtp_pass`, `from`, `to` in `config` JSONB; SSRF-validated against `smtp_host:smtp_port` at both handler and runtime (private IP blocked, non-SMTP ports 22/80/443/3306/5432/6379/11211/27017 rejected). `smtp_pass` is masked `"********"` on GET; server preserves the stored value when an incoming PUT carries the placeholder.
- **SSRF protection** (`services/url_validator.rs`): Shared module blocks private/reserved IPs (RFC 1918, link-local, loopback, CGNAT). Applied at handler validation AND runtime execution (defense-in-depth).

## Authentication
Two-track Bearer auth (`services/auth.rs`):
1. **Agent JWT** (HS256): signed with `JWT_SECRET`, 60s expiry, `aud: "agent"` — used by agents during scraping. Only accepted by the server's internal scraping path, never by API endpoints.
2. **User JWT** (HS256): signed with same `JWT_SECRET`, 24h expiry, `aud: "user"` — contains `sub` (user_id), `username`, `role`, `iat`. Generated on login.
Token type separation via `aud` claim prevents cross-use. Legacy tokens without `aud` accepted for backward compatibility.
SSE endpoint uses single-use opaque ticket via `POST /api/auth/sse-ticket` (EventSource cannot set headers).

**UserGuard**: Axum extractor that only accepts user JWTs (`aud: "user"`). Rejects agent JWTs. Used on all read endpoints (GET). Prevents compromised agents from accessing user data.

**AdminGuard**: Only accepts user JWTs with `role == "admin"`. Used on all mutation endpoints (POST/PUT/DELETE) and notification channel listing (sensitive SMTP credentials). Returns `AppError::Unauthorized` for non-admin users.

**Token revocation**: `iat` claim checked against a unified in-memory cutoff cache keyed by `user_id`. Two sources feed the same cache — `users.password_changed_at` (password change flow) and `users.tokens_revoked_at` (explicit logout / admin kill-switch). For each user the **later** of the two timestamps wins, and tokens whose `iat` is strictly older than that cutoff are rejected by `UserGuard`/`AdminGuard`. The cache is seeded on startup from both columns (see `main.rs`); runtime updates go through `services::auth::update_password_changed_at` and `update_tokens_revoked_at`, both of which funnel into `raise_revocation_cutoff` so a later event never lowers an earlier one.

**Login rate limiting**: 10 attempts per 5 minutes per IP. Uses `ConnectInfo<SocketAddr>` by default (immune to X-Forwarded-For spoofing). Set `TRUSTED_PROXY_COUNT=N` to use Nth-from-right IP in X-Forwarded-For.

**Password policy**: Minimum 8 characters, maximum 128 characters (Argon2 DoS prevention), must contain uppercase, lowercase, digit, and special character. Frontend validates the same rules before submission (`setup/page.tsx`).

**JWT_SECRET**: Must be at least 32 characters. Server refuses to start with shorter secrets. `ENCODING_KEY`/`DECODING_KEY` are `OnceLock`s seeded once at startup — rotating `JWT_SECRET` requires restarting the server (and every agent sharing that secret), and the restart immediately invalidates all previously-issued agent and user JWTs because HMAC signature verification fails. There is no in-process "reload secret" path by design.

**User auth flow:**
- `POST /api/auth/setup` — create initial admin (only when users table is empty, no auth required)
- `POST /api/auth/login` — verify username/password (argon2), return user JWT
- `PUT /api/auth/password` — change current user's password (requires current password verification)
- `GET /api/auth/me` — validate JWT, return user info
- `GET /api/auth/status` — check if setup is needed (no auth)
- Passwords hashed with Argon2id (`argon2` crate)

**Unauthenticated endpoints:** `/api/auth/login`, `/api/auth/setup`, `/api/auth/status`, `/api/public/status`, `/api/health`. `/metrics` is only unauthenticated when `ALLOW_UNAUTHENTICATED_METRICS=true`.

**Frontend auth:** `AuthContext` wraps the app. Context value memoized with `useMemo`; `login`/`logout` callbacks wrapped in `useCallback`. Unauthenticated users are redirected to `/login` (including automatic redirect on 401 responses). Public paths: `/login`, `/setup`, `/status`. Login errors shown via `sonner` toast notifications (401/429/network/generic cases, i18n-aware).

## Security
- **SSRF protection** (`services/url_validator.rs`): Blocks private/reserved IPs (RFC 1918, link-local 169.254.x.x, loopback, CGNAT 100.64/10), including IPv4-mapped IPv6 addresses (`::ffff:127.0.0.1`). Applied to webhook URLs, HTTP monitors, and ping monitors at both handler validation and runtime execution (defense-in-depth).
- **host_key validation**: `host_key` must be `host:port` format — no path, query, fragment, or `@` characters allowed. Prevents SSRF via path injection when the scraper builds `http://{host_key}/metrics`.
- **Error masking**: `AppError::Internal` returns generic "Internal server error" to clients. Detailed error is logged server-side only.
- **Security headers**: `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: strict-origin-when-cross-origin` on all responses.
- **Bincode payload limit**: Agent responses capped at 10 MB to prevent OOM from malicious agents.
- **Token type enforcement**: `UserGuard` rejects agent JWTs on all user-facing endpoints. Agent JWTs are only valid for the server's internal scraping path. `AdminGuard` additionally requires `role == "admin"`.
- **Token revocation**: `iat` claim in user JWT checked against unified in-memory cutoff cache. Tokens issued before last password change or explicit logout are rejected. SSE streams also poll this cutoff every 15s and terminate when the session is no longer valid.
- **SMTP credential redaction**: `GET /api/notification-channels` (AdminGuard) masks `smtp_pass` with `********` in responses.
- **Port scan cap**: Agent limits ports query param to 100 entries to prevent DoS via unbounded TCP connections.

## i18n (Frontend)
- Custom implementation: `app/i18n/translations.ts` (type-safe object) + `app/i18n/I18nContext.tsx` (React context).
- Locale state is client-side only (React context + `localStorage`). No URL-based routing.
- `I18nContext` syncs `<html lang>` attribute with current locale via `useEffect`.
- Language toggle button is in `Navbar.tsx`.
- When adding new UI strings: add keys to both `en` and `ko` sections in `translations.ts`, use `const { t } = useI18n()`.

## Theming (Frontend)
- CSS variable based: `:root` (light) and `[data-theme="dark"]` (dark) in `globals.css`.
- `ThemeContext.tsx`: React context + `localStorage` persistence + system preference detection. Context value memoized with `useMemo`.
- Theme toggle (sun/moon icon) in `Navbar.tsx`.
- All colors MUST use CSS variables. No hardcoded hex values in components. Use `var(--text-on-accent, #fff)` pattern for fallbacks.
- **Toast notifications**: `sonner` library. `<Toaster />` in root `layout.tsx` with `position="top-right"`, `theme="system"`, `richColors`, `duration={4000}`.

## Navigation (Frontend)
- Top navigation bar (`Navbar.tsx`) replaces the previous sidebar layout.
- Mobile: hamburger menu with collapsible dropdown, all icon buttons have `aria-label` for accessibility.
- Desktop: horizontal nav items with icon buttons for theme, locale, and logout.

## Environment Variables (server)
See `netsentinel-server/.env.example` for full reference. Key optional vars:
- `STATIC_ASSETS_DIR` — directory holding the pre-built Next.js static export served alongside the API. Set to `/app/static` inside the production Docker image; unset in local dev (the Next.js dev server handles routing on port 3001 instead).
- `ALLOWED_ORIGINS` — comma-separated CORS origins (default: `http://localhost:3001`). In production the web bundle shares an origin with the API, so CORS is effectively a no-op for the browser path; still list the external origin if a reverse proxy splits the two.
- `SERVER_HOST` / `SERVER_PORT` — bind address (default: `0.0.0.0:3000`)
- `MAX_DB_CONNECTIONS` — PostgreSQL pool size (default: `10`)
- `SCRAPE_INTERVAL_SECS` — fallback scrape interval when a host row has no valid `scrape_interval_secs` (default: `10`)
- `COOKIE_SECURE` — whether refresh cookies use the `Secure` flag (default: `true`; set `false` only for local plain-HTTP dev)
- `ALLOW_UNAUTHENTICATED_METRICS` — explicit opt-in to keep `/metrics` open when `METRICS_TOKEN` is unset (default: disabled)
- `TRUSTED_PROXY_COUNT` — number of trusted reverse proxies for X-Forwarded-For (default: `0`, meaning peer IP used directly)
- `DB_STATEMENT_TIMEOUT_SECS` — per-query PostgreSQL statement timeout (default: `30`)
- `LOGIN_RATE_LIMIT_MAX` — max login attempts per IP within window (default: `10`)
- `LOGIN_RATE_LIMIT_WINDOW_SECS` — sliding window duration for login rate limit (default: `300`)
- `API_RATE_LIMIT_MAX` — max API requests per IP within window (default: `200`)
- `API_RATE_LIMIT_WINDOW_SECS` — sliding window duration for API rate limit (default: `60`)
- `METRICS_CACHE_MAX_ENTRIES` — upper bound on in-memory metrics query cache size (default: `200`). v0.3.0 grew per-sample payload 3–5×, so an unbounded cache risks hundreds of MB under concurrent dashboard load. Oldest-inserted entries evicted once the cap is hit (TTL remains 120 s).

## Commands
- **Full Stack (prod)**: `./scripts/bootstrap.sh && docker compose up -d --build && ./scripts/smoke-test.sh`. `bootstrap.sh` writes `.env` with random JWT_SECRET + DB password, compose builds the single-container stack (server + embedded web), `smoke-test.sh` runs 5 checks. After success, open `http://localhost:3000/setup` for the first admin — full post-install flow in `docs/AFTER_INSTALL.md`. Use `./scripts/doctor.sh` to diagnose any failed step.
- **Server deploy**: GitHub Actions CI/CD (PR-triggered lint/test/build + manual deploy via SSH rsync)
- **Agent**: `cargo build --release` — macOS LaunchDaemon or Linux Docker
- **Web dev**: `npm run dev` in `netsentinel-web/` (port 3001 with HMR). Run `cargo run` in `netsentinel-server/` on port 3000; set `NEXT_PUBLIC_API_URL=http://localhost:3000` in `netsentinel-web/.env`. The dev loop is identical to the pre-v0.3.6 layout.
- **Web static export (prod-like)**: `cd netsentinel-web && npm run build` emits `out/`. Run the server against it: `STATIC_ASSETS_DIR=$(pwd)/netsentinel-web/out cargo run --manifest-path netsentinel-server/Cargo.toml`.
- **CI (local)**: `cargo fmt --check && cargo check && cargo clippy -- -D warnings && cargo test`

## Git Conventions
- Commit messages should use a lowercase type prefix such as `fix:`, `feat:`, `docs:`, `refactor:`, or `test:` followed by an imperative summary.

## Context Management
Automatically compact at 80%+ context usage. Preserve: task objectives, modified files list, key decisions, current progress status, and remaining TODOs. Continue work seamlessly after compacting without confirmation.
