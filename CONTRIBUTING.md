# Contributing to NetSentinel

Thank you for your interest in contributing! This document covers everything you need to get the project running locally and submit a pull request.

## Table of Contents

- [Prerequisites](#prerequisites)
- [Getting Started](#getting-started)
- [Project Structure](#project-structure)
- [Development Workflow](#development-workflow)
- [Code Conventions](#code-conventions)
- [Testing](#testing)
- [Submitting a Pull Request](#submitting-a-pull-request)
- [Reporting Issues](#reporting-issues)

---

## Prerequisites

| Tool | Minimum version | Purpose |
|------|----------------|---------|
| Docker + Docker Compose | 24+ | Full stack orchestration |
| Rust toolchain | 1.85 (stable) | Server & agent |
| Node.js | 22 | Web frontend |
| npm | 10 | Web package management |

> **Tip:** Install Rust via [rustup](https://rustup.rs/). Install Node.js via [nvm](https://github.com/nvm-sh/nvm) or [fnm](https://github.com/Schniz/fnm).

---

## Getting Started

```bash
# 1. Clone the repository
git clone https://github.com/<owner>/netsentinel.git
cd netsentinel

# 2. Generate .env with random secrets (JWT_SECRET)
./scripts/bootstrap.sh

# 3. Start the published stack (single server container serves both API
#    and the embedded web static bundle)
docker compose pull server
docker compose up -d server

# 4. Verify the install
./scripts/smoke-test.sh
```

The dashboard **and** the API share **http://localhost:3000**. Open `/setup` for the first-admin flow — `docs/AFTER_INSTALL.md` walks through the full 10-minute path (admin → first host → first agent). Use `./scripts/doctor.sh` if anything above fails.

---

## Project Structure

```
netsentinel/
├── server/   # Rust/Axum backend — REST API, scraper, SSE
├── agent/    # Rust daemon — collects host metrics
├── web/      # Next.js dashboard
├── docker-compose.yml        # Pull-only homelab stack
├── .env.example              # Environment variable template
└── .github/workflows/        # GitHub Actions CI
```

See [ARCHITECTURE.md](README.md#architecture) in the README for data flow details.

---

## Development Workflow

The root `docker-compose.yml` is the homelab install path and pulls a published image. When you need Docker to build the current checkout instead, drop a `docker-compose.override.yml` next to it — `docker compose` automatically merges any file with that name on top of the base, so the override stays untracked (`.gitignore`d) and never leaks into the upstream:

```yaml
# docker-compose.override.yml — untracked, per-host
services:
  server:
    image: netsentinel-server:dev
    build:
      context: .
      dockerfile: server/Dockerfile
      args:
        - NEXT_PUBLIC_API_URL=${NEXT_PUBLIC_API_URL:-}
```

```bash
docker compose up -d --build server
```

### Server (Rust/Axum)

```bash
cd server
cp .env.example .env          # edit DATABASE_URL etc.
cargo run                     # starts on 0.0.0.0:3000 by default
```

Useful commands:

```bash
cargo check                   # fast syntax/type check
cargo clippy -- -D warnings   # lint (CI-equivalent)
cargo fmt                     # auto-format
cargo test                    # run unit tests
```

When testing auth locally over plain HTTP, set `COOKIE_SECURE=false` in `server/.env`; production should keep the default secure cookie. Prometheus scraping is auth-required by default, so set `METRICS_TOKEN` (recommended) or explicitly opt in to anonymous scraping with `ALLOW_UNAUTHENTICATED_METRICS=true`.

### Agent (Rust)

```bash
cd agent
cp .env.example .env          # edit JWT_SECRET, AGENT_PORT, AGENT_BIND
cargo run
```

### Web (Next.js)

```bash
cd web
npm install
cp .env.example .env.local    # set NEXT_PUBLIC_API_URL=http://localhost:3000
npm run dev                   # starts on http://localhost:3001 with HMR
```

The dev server still runs Next.js normally (HMR, fast refresh, dynamic routes). The `output: 'export'` + Axum-embed layout only applies to the production Docker image — `npm run dev` is untouched.

Host detail URLs are now part of the static-export contract: use `/host/?key=<host_key>` (trailing slash intentional — `trailingSlash: true` is set for production export) rather than `/host/<host_key>`. The latter may still be served by a generic fallback in local tooling, but it is not the canonical frontend route anymore.

Useful commands:

```bash
npm run lint     # ESLint
npm test         # Vitest unit tests
npm run build    # production static export → emits out/
```

---

## Code Conventions

### Rust

- Follow `rustfmt` defaults — run `cargo fmt` before every commit.
- Address all `cargo clippy -- -D warnings` findings before opening a PR.
- Error types go in `src/errors.rs`; use `AppError` variants throughout.
- All public functions and types should have a one-line doc comment (`///`).
- Write comments in **English**.

### TypeScript / Next.js

- Use the existing `useI18n()` hook (`app/i18n/I18nContext.tsx`) for any UI strings — do not hardcode visible text.
- Follow the established file structure: pages in `app/`, reusable components in `app/components/`.
- Inline styles are acceptable for now; prefer CSS variables defined in `globals.css` for colours and spacing.
- Do not cache App Router documents or RSC payloads in the service worker; only immutable static assets belong there.

### Git

- Branch naming: `feat/<short-description>`, `fix/<short-description>`, `docs/<short-description>`.
- Commit messages: use a lowercase type prefix such as `fix:`, `feat:`, `docs:`, `refactor:`, or `test:` followed by an imperative summary; keep the subject line within 72 characters and leave a blank line before the body when needed.
- One logical change per commit.

---

## Testing

### Server unit tests

```bash
cd server
cargo test
```

Existing tests cover JWT generation/validation, alert-threshold logic, password validation, rate limiting, refresh-token rotation, SSE tickets, request ID generation, and input validation. There are 190 tests total across the project: server (107), agent (33), web (50).

### Database migrations

Schema changes use [sqlx migrations](https://docs.rs/sqlx/latest/sqlx/macro.migrate.html). Migrations run automatically on server startup.

```bash
# To add a new migration:
# 1. Create a new numbered SQL file:
touch server/migrations/006_your_change.sql
# 2. Write idempotent SQL (use IF NOT EXISTS, IF EXISTS, etc.)
# 3. Never modify existing migration files — always create new ones
# 4. Migrations are embedded at compile time via sqlx::migrate!()
```

For new time-series metrics, keep raw and rollup storage in sync: add nullable raw columns for write-time scalar projections when they avoid repeated JSON parsing, update the batch insert path, update `metrics_5min` aggregation, and update every branch of `fetch_metrics_range`. If the metric is rendered on `/host?key=`, also update the lightweight `/api/metrics/{host_key}/chart` projection (`fetch_chart_metrics_range`) so chart pages do not fall back to caching full snapshot rows.

### Web unit tests

```bash
cd web
npm test
```

Tests use [Vitest](https://vitest.dev/). New tests go in `*.test.ts(x)` files co-located with the source they test.

---

## Deployment & Rollback

### Image tagging

From v0.4.2 there is **one** Docker image — `netsentinel-server` bakes the web static bundle into `/app/static`. The separate `netsentinel-web` image has been removed. Tagged releases publish both the release tag and `latest`:

```bash
# CI builds and pushes:
ghcr.io/sounmu/netsentinel-server:<release-tag>
ghcr.io/sounmu/netsentinel-server:latest
```

### Rolling back

If a deployment causes issues, roll back to a known-good image:

```bash
# 1. Find the previous working release tag
git tag --sort=-creatordate | head

# 2. Pin docker-compose to the known-good image in .env:
#      NETSENTINEL_VERSION=v0.4.2

# 3. Redeploy
docker compose pull server && docker compose up -d server

# 4. Verify health
curl -sf http://localhost:3000/api/health
```

### Database migration rollback

Migrations are forward-only (`sqlx::migrate!()`). If a migration causes issues:

1. **Do NOT delete the migration file** — this breaks `sqlx::migrate!()` checksums.
2. Create a **new** migration that reverses the change (e.g., `DROP COLUMN IF EXISTS`, `DROP TABLE IF EXISTS`).
3. Test the reverse migration locally before deploying.

### Agent rollback

Agents run as native binaries (not Docker). Keep the previous binary alongside the new one:

```bash
# Before deploying a new agent:
cp /usr/local/bin/netsentinel-agent /usr/local/bin/netsentinel-agent.bak

# To roll back:
mv /usr/local/bin/netsentinel-agent.bak /usr/local/bin/netsentinel-agent
sudo systemctl restart netsentinel-agent   # or launchctl on macOS
```

---

## Submitting a Pull Request

1. **Fork** the repository and create a feature branch from `main`.
2. Make your changes and ensure all CI checks pass locally:
   ```bash
   # Rust
   cargo fmt --check && cargo clippy -- -D warnings && cargo test
   # Web
   npm run lint && npm test && npm run build
   ```
3. Open a pull request against `main`. Fill in the PR template (summary, test plan).
4. A maintainer will review within a few days. Feedback may be requested before merging.

> **Breaking changes**: If your PR modifies the SSE payload schema or REST API contracts, note it clearly in the PR description so consumers can prepare.
> Server and frontend SSE type changes must ship in the same commit so the static web bundle never decodes a different wire shape than the server emits.
> Update `README.md`, `CONTRIBUTING.md`, and any relevant `.env.example` files whenever config defaults, auth behavior, or API/SSE contracts change.

---

## Reporting Issues

Please open an issue on GitHub with:

- A clear, concise title.
- Steps to reproduce the problem.
- Expected vs. actual behaviour.
- Relevant logs or screenshots.
- Your environment (OS, Docker version, browser if frontend).

For security vulnerabilities, please **do not** open a public issue. Email the maintainer directly instead.
