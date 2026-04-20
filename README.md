# NetSentinel

> Real-time server infrastructure monitoring — lightweight, self-hosted, and zero-trust.

![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)
![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)
![Next.js](https://img.shields.io/badge/next.js-16-black.svg)

---

## Overview

**NetSentinel** is a pull-based server monitoring system built with Rust and Next.js. A central server scrapes lightweight agents installed on each target host, stores metrics in an embedded SQLite file, and streams real-time data to the web dashboard via SSE.

**What sets it apart:**
- **One binary + one file.** The server keeps every metric, alert, user, and config in a single SQLite database (`data/netsentinel.db`, WAL mode). Backups are `cp`, restores are trivial, and there is no extra DB container to keep healthy.
- **Pull-scraping behind Zero Trust.** The server reaches agents over Cloudflare Tunnel, so hosts behind NAT or a firewall are monitored without opening a single inbound port.
- **Binary agent protocol, gzipped.** Agents serve `bincode` over HTTP — not JSON, not Prometheus text — with `tower-http` gzip on top, so scrape payloads stay small over tunneled links.
- **Time-range-aware query routing.** `≤6h` hits raw 10 s metrics; `6h–14d` reads the 5-min rollup table; `>14d` re-aggregates that rollup into 15-min buckets. Long-range charts never scan the raw table. The rollup is populated by an in-process 60 s worker — SQLite's equivalent of a continuous aggregate.
- **One stack instead of four.** Host metrics, Docker state, HTTP/Ping external monitors, and multi-channel alerting (Discord / Slack / Email) live in a single self-hosted service — no Prometheus + Uptime Kuma + Alertmanager + Grafana to wire together.
- **Streaming SSE dashboard.** Status and metrics push over one SSE connection; the client batches bursts of up to 100 events into a single React render via `requestAnimationFrame`.
- **Event-driven Docker cache.** The agent subscribes to the Docker Events API instead of polling `docker ps`, keeping container state fresh with effectively zero daemon load between events.
- **NVIDIA and Apple Silicon GPU in one agent.** NVML and macmon are both wired in behind Cargo feature flags — the same binary handles a CUDA rig or an M-series Mac.
- **Single-binary Rust for both server and agent.** No Python runtime, no JVM, no sidecars. The agent runs comfortably on a Raspberry Pi.

---

## Architecture

```mermaid
graph LR
    A[Agent<br/>Rust daemon] -->|JWT / HTTP pull| S[Server<br/>Rust / Axum<br/>+ embedded Web]
    S -->|sqlx| DB[(SQLite<br/>data/netsentinel.db)]
    S -.->|serves static bundle| BR[Browser]
    BR -->|SSE stream / REST| S
    S -->|Webhook| D[Discord / Slack]
    S -->|SMTP| E[Email Alerts]
    BR -->|Zero Trust| CF[Cloudflare Tunnel]
    A -->|Zero Trust| CF
```

From v0.3.6 the Next.js web tier is compiled to a static export and served directly by Axum via `tower-http::ServeDir`, so production runs in a **single container**. The old separate `web` container + its ~35 MB Node runtime is gone. From v0.4.0 the separate PostgreSQL / TimescaleDB container is also gone — storage moved to an embedded SQLite file mounted at `/app/data/netsentinel.db`. Local development is unchanged — `npm run dev` still spins up the Next.js dev server on port 3001 with HMR.

**Frontend route contract:** the host detail page is now the static route `/host/?key=<host_key>`. Because the bundle is exported as plain HTML with `trailingSlash: true`, the canonical URL keeps the trailing slash and the `host_key` is passed as a query parameter — resolved client-side via `useSearchParams()` instead of being encoded as a dynamic path segment.

**Data flow:**
1. Server schedules each registered agent by that host's `scrape_interval_secs` (10 s by default), batch-inserts metrics in a single query
2. Raw metrics stored in SQLite (3-day retention) + a 5-min rollup table (90-day retention) maintained by an in-process `rollup_worker` running on a 60-second tick. A daily `retention_worker` prunes each time-series table past its window.
3. Browser loads the static bundle from the same origin as the API, then connects to the SSE stream for real-time updates (in-memory — no DB hit, rAF-batched). SSE `metrics` event includes CPU, memory, load, network, disks, temperatures, and Docker stats for live chart overlay; long-lived streams are cut when the session is revoked
4. REST API with automatic downsampling: ≤6h raw, 6h-14d 5-min rollup, >14d 15-min re-aggregation
5. Alerts delivered to Discord, Slack, and/or Email channels

---

## Monorepo Structure

```
netsentinel/
├── netsentinel-server/   # Rust/Axum backend — metrics API, scraper, alerts,
│                         # and (in prod) the embedded web static bundle
├── netsentinel-web/      # Next.js dashboard — compiled to `output: 'export'`
│                         # and baked into the server image at build time
└── netsentinel-agent/    # Rust agent daemon
```

---

## Quick Start

### Install the hub (one line)

On the machine that will run the dashboard + API:

```bash
curl -fsSL https://raw.githubusercontent.com/sounmu/netsentinel/main/scripts/install-hub.sh | bash
```

It clones the repo into `~/netsentinel`, generates `.env` with random secrets, runs `docker compose up -d --build`, verifies the install with a 5-check smoke test, and prints the URL + the JWT_SECRET you'll need for the agent step below.

Prerequisites: Docker + Compose v2, `git`, `curl`, `openssl`. Tested on Linux and macOS; Windows users should run this inside WSL2.

### Install an agent on every monitored host (one line)

On each target machine, paste the JWT_SECRET the hub printed:

```bash
curl -fsSL https://raw.githubusercontent.com/sounmu/netsentinel/main/scripts/install-agent.sh \
  | sudo bash -s -- \
      --jwt-secret "PASTE_THE_HUB_SECRET_HERE" \
      --bind "0.0.0.0" \
      --port 9101 \
      --ref main
```

Use `--bind "100.x.y.z"` to listen only on the agent's Tailscale interface, or change `--port` and register the matching `<agent-ip>:<port>` in the hub. The installer builds the agent binary, drops a systemd unit (Linux) or launchd daemon (macOS), starts the service, and prints the exact `host_key` you should paste into the hub's Agents page. Re-run the same command later with `--ref <tag-or-branch>` to update the native agent in place.

> The agent currently builds from source via `cargo install` — if the target machine does not have Rust, the installer prints the one-line rustup command to run first. Phase B will publish prebuilt binaries via GitHub Releases so this step becomes a pure download.

### Register the host in the UI

1. Open `http://<hub-ip>:3000/setup` → create the first admin account.
2. Navigate to **Agents → + Add Agent** and paste the `host_key` the agent installer printed (for example `192.168.1.10:9101`).
3. The host flips from `pending` → `online` within one scrape interval (default 10 s).

Full walkthrough with troubleshooting: [`docs/AFTER_INSTALL.md`](docs/AFTER_INSTALL.md).

### If something goes wrong

```bash
cd ~/netsentinel
./scripts/doctor.sh        # laddered diagnosis, tells you the exact recovery command
```

### Offline / air-gapped install

When the one-liner can't reach GitHub (corporate proxy, offline lab, etc.), run the same three steps by hand:

```bash
git clone https://github.com/sounmu/netsentinel.git
cd netsentinel
./scripts/bootstrap.sh            # generates .env
docker compose up -d --build
./scripts/smoke-test.sh
```

---

## Running without Docker (development only)

Use this path when you are actively changing code. For production homelab installs, the Quick Start above is faster and safer.

### Server (port 3000)

```bash
cd netsentinel-server
cp .env.example .env   # set JWT_SECRET; DATABASE_URL defaults to sqlite://./data/netsentinel.db
mkdir -p data          # SQLite needs the parent directory to exist
cargo run
```

### Web dashboard (port 3001, HMR)

```bash
cd netsentinel-web
cp .env.example .env.local   # NEXT_PUBLIC_API_URL=http://localhost:3000
npm install
npm run dev
```

`npm run dev` runs the full Next.js dev server — dynamic routes, fast refresh, everything. The `output: 'export'` + Axum-embed layout only kicks in when the production image is built.

### Agent (port 9101)

```bash
cd netsentinel-agent
cp .env.example .env   # JWT_SECRET must match the server
cargo run
```

---

## Environment Variables

### Root `.env` (Docker Compose)

| Variable | Required | Default | Description |
|---|---|---|---|
| `JWT_SECRET` | **Yes** | — | HS256 secret (≥ 32 chars). Every agent needs the same value. `bootstrap.sh` generates it via `openssl rand -hex 32`. |
| `CLOUDFLARE_TUNNEL_TOKEN` | No | — | Cloudflare Tunnel token. Only read when you activate the `tunnel` service via a compose override — see [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md). |
| `NEXT_PUBLIC_API_URL` | No | empty (same-origin) | Backend URL the **browser** fetches from. Empty = same-origin, correct for localhost and single-hostname reverse proxies. Override only when the dashboard and API live on different hostnames — see [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md). Baked into the web bundle at build time; changing this value requires `docker compose up -d --build server`. |

> Upgrading from v0.3.x? The old `POSTGRES_USER` / `POSTGRES_PASSWORD` / `POSTGRES_DB` variables are no longer read — remove them from `.env`. There is nothing to migrate if this is a greenfield install. See the v0.4.0 section of [`CHANGELOG.md`](./CHANGELOG.md) for how to move data from an existing Postgres deployment.

### Server — all keys below

Under Docker Compose the server reads **root `.env`** (via `env_file: .env` in `docker-compose.yml`). `netsentinel-server/.env` is only consulted by a local `cargo run`. So add these keys to `./env` for a Docker install, or to `netsentinel-server/.env` for a local dev install — never both.

| Variable | Required | Default | Description |
|---|---|---|---|
| `DATABASE_URL` | **Docker: auto** / **local: yes** | — | SQLite connection URL. `docker-compose.yml` hard-codes `sqlite:///app/data/netsentinel.db` — a local `cargo run` has to set it explicitly (`sqlite://./data/netsentinel.db`). The parent directory must exist; the `.db` file (plus `-wal` / `-shm` sidecars) is created on first boot. |
| `ALLOWED_ORIGINS` | No | `http://localhost:3001` | Comma-separated CORS origins. With the single-container layout this mostly only gates third-party embeds; split-origin deployments must list both hostnames — see [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md). No trailing slash, no `*`. |
| `SERVER_HOST` | No | `0.0.0.0` | Bind address |
| `SERVER_PORT` | No | `3000` | Bind port |
| `SCRAPE_INTERVAL_SECS` | No | `10` | Fallback scrape interval if a host row has no valid `scrape_interval_secs`; normal scheduling is per-host |
| `MAX_DB_CONNECTIONS` | No | `10` | sqlx connection pool size. SQLite serialises writes via a single writer lock, so values beyond ~10 provide no throughput gain and only grow idle pool memory. |
| `SSE_BUFFER_SIZE` | No | `128` | SSE broadcast channel buffer |
| `TRUSTED_PROXY_COUNT` | No | `0` | Reverse proxy count for X-Forwarded-For (0 = use peer IP directly) |
| `METRICS_CACHE_MAX_ENTRIES` | No | `200` | Max in-memory query-cache entries (oldest-inserted evicted when full; TTL 120 s) |
| `COOKIE_SECURE` | No | `true` | Whether the refresh cookie carries the `Secure` flag. Leave enabled in production; set `false` only for local plain-HTTP development. |
| `METRICS_TOKEN` | No | — | Bearer token for `/metrics` (Prometheus). When set, every scrape must send `Authorization: Bearer <token>`. |
| `ALLOW_UNAUTHENTICATED_METRICS` | No | `false` | Explicit opt-in to leave `/metrics` open when `METRICS_TOKEN` is unset. |

### Agent `netsentinel-agent/.env`

| Variable | Required | Default | Description |
|---|---|---|---|
| `JWT_SECRET` | **Yes** | — | Must match server's `JWT_SECRET` |
| `AGENT_PORT` | No | `9101` | Port the agent HTTP server listens on |
| `AGENT_BIND` | No | `0.0.0.0` | Bind address. Use a Tailscale IP such as `100.x.y.z` to expose the native agent only on that interface. |

---

## API Endpoints

All endpoints require `Authorization: Bearer <JWT>` unless noted. Read endpoints use `UserGuard` (rejects agent JWTs). Mutation endpoints require `AdminGuard` (admin role only).

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/auth/login` | Login **(no auth)** |
| `POST` | `/api/auth/setup` | Create initial admin **(no auth, first run only)** |
| `GET` | `/api/auth/me` | Current user info |
| `GET` | `/api/auth/status` | Check if setup needed **(no auth)** |
| `PUT` | `/api/auth/password` | Change current user's password |
| `GET` | `/api/health` | Health check — verifies DB **(no auth)** |
| `GET` | `/api/dashboard` | Get user's dashboard layout |
| `PUT` | `/api/dashboard` | Save user's dashboard layout |
| `GET` | `/api/hosts` | List all hosts with online status |
| `GET` | `/api/hosts/{host_key}` | Get a single host configuration |
| `POST` | `/api/hosts` | Register a new host |
| `PUT` | `/api/hosts/{host_key}` | Update host configuration |
| `DELETE` | `/api/hosts/{host_key}` | Delete a host |
| `GET` | `/api/metrics/{host_key}` | Recent 50 metric rows |
| `GET` | `/api/metrics/{host_key}?start=&end=` | Metrics in a time range (ISO 8601) |
| `POST` | `/api/metrics/batch` | Batch metrics for multiple hosts (max 50) |
| `GET` | `/api/uptime/{host_key}?days=` | Daily uptime breakdown |
| `GET` | `/api/alert-configs` | Global alert defaults |
| `PUT` | `/api/alert-configs` | Update global defaults |
| `GET` | `/api/alert-configs/{host_key}` | Host-specific alert overrides |
| `PUT` | `/api/alert-configs/{host_key}` | Upsert host alert overrides |
| `DELETE` | `/api/alert-configs/{host_key}` | Delete host overrides |
| `GET` | `/api/notification-channels` | List notification channels |
| `POST` | `/api/notification-channels` | Create channel |
| `PUT` | `/api/notification-channels/{id}` | Update channel |
| `DELETE` | `/api/notification-channels/{id}` | Delete channel |
| `POST` | `/api/notification-channels/{id}/test` | Send test notification |
| `GET` | `/api/alert-history?host_key=&limit=` | Alert event log |
| `GET` | `/api/http-monitors` | List HTTP monitors |
| `POST` | `/api/http-monitors` | Create HTTP monitor |
| `GET` | `/api/http-monitors/summaries` | HTTP monitor summaries |
| `PUT` | `/api/http-monitors/{id}` | Update HTTP monitor |
| `DELETE` | `/api/http-monitors/{id}` | Delete HTTP monitor |
| `GET` | `/api/http-monitors/{id}/results` | HTTP check results |
| `GET` | `/api/ping-monitors` | List Ping monitors |
| `POST` | `/api/ping-monitors` | Create Ping monitor |
| `GET` | `/api/ping-monitors/summaries` | Ping monitor summaries |
| `PUT` | `/api/ping-monitors/{id}` | Update Ping monitor |
| `DELETE` | `/api/ping-monitors/{id}` | Delete Ping monitor |
| `GET` | `/api/ping-monitors/{id}/results` | Ping check results |
| `GET` | `/api/public/status` | Public status page data **(no auth)** |
| `GET` | `/metrics` | Prometheus metrics export (**auth required by default** — set `METRICS_TOKEN`, or explicitly opt in via `ALLOW_UNAUTHENTICATED_METRICS=true`) |
| `POST` | `/api/auth/logout` | Revoke all tokens for current user |
| `POST` | `/api/auth/refresh` | Rotate refresh cookie + mint fresh access JWT (cookie is the credential; no auth header) |
| `POST` | `/api/auth/sse-ticket` | Mint single-use ticket for SSE |
| `POST` | `/api/admin/users/{id}/revoke-sessions` | Admin: force-revoke user sessions |
| `GET` | `/api/stream?key=<ticket>` | SSE stream (`metrics` + `status`) |

Frontend permalink contract:
- Host detail: `/host/?key=<host_key>`
- Example: `/host/?key=192.168.1.10:9101`

---

## Database Schema

All tables live in a single SQLite file (`data/netsentinel.db`, WAL mode, STRICT + WITHOUT ROWID on hot paths). Time-series tables are pruned by the in-process `retention_worker`; there is no hypertable or continuous aggregate — see [`docs/SQLITE_MIGRATION.md`](docs/SQLITE_MIGRATION.md) for the design rationale.

| Table | Description |
|---|---|
| **`metrics`** | Raw scrape rows. 3-day retention. Stores CPU, memory, load, network, disk, process, temperature, GPU, Docker, port data as JSON text columns. |
| **`metrics_5min`** | 5-minute rollup table (`STRICT, WITHOUT ROWID`, PK `(host_key, bucket)`). Populated by `services::rollup_worker` on a 60-second tick via an idempotent UPSERT from `metrics`. 90-day retention. |
| **`hosts`** | Agent registry (scrape interval, thresholds, monitored ports/containers, system info: OS/CPU/RAM/IP). `ports` / `containers` stored as JSON arrays in TEXT columns. |
| **`alert_configs`** | Alert rules; `NULL host_key` = global default, per-host rows override. `UNIQUE NULLS NOT DISTINCT` is emulated with an expression-based UNIQUE INDEX on `(coalesce(host_key, ''), metric_type, coalesce(sub_key, ''))`. |
| **`notification_channels`** | Alert delivery targets (Discord webhook, Slack webhook, Email SMTP). Config stored as JSON text. |
| **`dashboard_layouts`** | Per-user dashboard widget layout (JSON text). |
| **`users`** | User accounts with Argon2 password hashing. Roles: admin, viewer. Tracks `password_changed_at` and `tokens_revoked_at` for unified JWT revocation. |
| **`refresh_tokens`** | Refresh token family table (`BLOB` hash / family_id, INTEGER epoch timestamps). Supports rotation + reuse detection. |
| **`alert_history`** | Immutable log of all alert events with timestamps. 90-day retention. |
| **`http_monitors`** | External HTTP endpoint monitors with check intervals. |
| **`http_monitor_results`** | HTTP check results (status code, response time, errors). 90-day retention. |
| **`ping_monitors`** | Network host reachability monitors (TCP connect). |
| **`ping_results`** | Ping check results (RTT stored as `REAL`, success as INTEGER 0/1). 90-day retention. |

---

## Tech Stack

| Component | Technology |
|---|---|
| Backend | Rust, Axum 0.8, sqlx 0.8 (bundled SQLite), lettre (SMTP) |
| Frontend | Next.js 16, React 19, Recharts, SWR, sonner (toast) |
| Agent | Rust, tokio, sysinfo, bollard (Docker), nvml-wrapper (NVIDIA GPU) |
| Database | Embedded SQLite (WAL mode) — one file under `data/` |
| Deployment | Docker Compose (single container), Cloudflare Tunnel |

---

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md) for development setup, coding conventions, and the PR process.

---

## License

[Apache License 2.0](./LICENSE) © 2026 sounmu
