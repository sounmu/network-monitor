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

# 2. Copy environment template and fill in your values
cp .env.example .env

# 3. Start the full stack (PostgreSQL + TimescaleDB + server + web)
docker compose up -d --build
```

The web dashboard will be available at **http://localhost:3001**.

---

## Project Structure

```
netsentinel/
├── netsentinel-server/   # Rust/Axum backend — REST API, scraper, SSE
├── netsentinel-agent/    # Rust daemon — collects host metrics
├── netsentinel-web/      # Next.js dashboard
├── docker-compose.yml        # Full stack orchestration
├── .env.example              # Environment variable template
└── .github/workflows/        # GitHub Actions CI
```

See [ARCHITECTURE.md](README.md#architecture) in the README for data flow details.

---

## Development Workflow

### Server (Rust/Axum)

```bash
cd netsentinel-server
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

### Agent (Rust)

```bash
cd netsentinel-agent
cp .env.example .env          # edit JWT_SECRET, AGENT_PORT etc.
cargo run
```

### Web (Next.js)

```bash
cd netsentinel-web
npm install
cp .env.example .env.local    # set NEXT_PUBLIC_API_URL
npm run dev                   # starts on http://localhost:3001
```

Useful commands:

```bash
npm run lint     # ESLint
npm test         # Vitest unit tests
npm run build    # production build
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

### Git

- Branch naming: `feat/<short-description>`, `fix/<short-description>`, `docs/<short-description>`.
- Commit messages: imperative mood, 72-char subject line, blank line before body.
- One logical change per commit.

---

## Testing

### Server unit tests

```bash
cd netsentinel-server
cargo test
```

Existing tests cover JWT generation/validation, alert-threshold logic, password validation, rate limiting, refresh-token rotation, SSE tickets, request ID generation, and input validation. There are 178 tests total across the project: server (96), agent (33), web (49).

### Database migrations

Schema changes use [sqlx migrations](https://docs.rs/sqlx/latest/sqlx/macro.migrate.html). Migrations run automatically on server startup.

```bash
# To add a new migration:
# 1. Create a new numbered SQL file:
touch netsentinel-server/migrations/006_your_change.sql
# 2. Write idempotent SQL (use IF NOT EXISTS, IF EXISTS, etc.)
# 3. Never modify existing migration files — always create new ones
# 4. Migrations are embedded at compile time via sqlx::migrate!()
```

### Web unit tests

```bash
cd netsentinel-web
npm test
```

Tests use [Vitest](https://vitest.dev/). New tests go in `*.test.ts(x)` files co-located with the source they test.

---

## Deployment & Rollback

### Image tagging

Docker images are tagged with the git short SHA and `latest` on every successful deploy:

```bash
# CI builds and pushes:
ghcr.io/sounmu/netsentinel-server:<short-sha>
ghcr.io/sounmu/netsentinel-server:latest
ghcr.io/sounmu/netsentinel-web:<short-sha>
ghcr.io/sounmu/netsentinel-web:latest
```

### Rolling back

If a deployment causes issues, roll back to a known-good image:

```bash
# 1. Find the previous working image tag (git short SHA from the last good release)
git log --oneline -5          # e.g. 6a0a9d1 is bad, 95afce6 was good

# 2. Pin docker-compose to the known-good image
#    Edit docker-compose.yml (or use an override file):
#      image: ghcr.io/sounmu/netsentinel-server:95afce6
#      image: ghcr.io/sounmu/netsentinel-web:95afce6

# 3. Redeploy
docker compose pull && docker compose up -d

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

---

## Reporting Issues

Please open an issue on GitHub with:

- A clear, concise title.
- Steps to reproduce the problem.
- Expected vs. actual behaviour.
- Relevant logs or screenshots.
- Your environment (OS, Docker version, browser if frontend).

For security vulnerabilities, please **do not** open a public issue. Email the maintainer directly instead.
