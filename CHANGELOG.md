# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [1.0.0] — 2026-04-05

### Added
- AdminGuard extractor — mutation endpoints require admin role
- Login rate limiting (10 attempts per 5 minutes per IP via X-Forwarded-For)
- Password change endpoint (PUT /api/auth/password)
- Health check endpoint (GET /api/health — verifies DB connectivity, returns server version)
- Graceful shutdown for server and agent (SIGTERM/SIGINT signal handling)
- Monitor failure alerting — HTTP/Ping monitor failures trigger notifications with 5-min cooldown
- Agent version field (agent_version) for server-agent compatibility tracking
- X-API-Version: 1 response header on all API responses
- Scraper exponential backoff for unresponsive hosts (10s → 160s cap)
- React ErrorBoundary wrapping main layout
- Skip-to-content link and focus-visible ring for keyboard accessibility
- aria-live region for SSE connection status
- 30+ i18n translation keys (EN/KO) for sidebar, agents, alerts, dashboard, ports
- CHANGELOG.md with git-cliff configuration for automated generation
- sqlx migrations (5 numbered SQL files replacing code-based init_db)

### Changed
- Authentication simplified to two-track: Agent JWT + User JWT (removed static API key)
- Chart colors now use CSS variables for proper dark mode support
- Server Dockerfile runs as non-root 'monitor' user
- Docker log rotation added to all services (10MB x 3 files)
- Deploy health check upgraded from / to /api/health (verifies DB)

### Fixed
- Replaced .expect() panics in auth.rs/user_auth.rs with proper AppError returns
- Frontend auto-redirects to /login on 401 (expired token)
- Input validation added for alert configs, monitors, and notification channels
- Uptime calculation always showing 100% — offline periods now write is_online=false metric records

### Security
- 90-day retention policies for alert_history, http_monitor_results, ping_results (TimescaleDB hypertables)

## [0.1.0] — 2026-04-04

### Added
- Full-stack network monitoring: Rust agent (CPU, memory, disk, GPU, Docker, ports) + Rust/Axum server + Next.js dashboard
- Real-time metrics via SSE and SWR polling
- TimescaleDB hypertable with 90-day retention and 5-minute continuous aggregates
- Multi-channel alerts: Discord, Slack, Email with per-host overrides and cooldown
- HTTP endpoint and Ping (TCP) external monitoring
- User authentication (Argon2id + JWT) with admin/viewer roles
- Customizable dashboard with pinnable widgets
- i18n support (English / Korean)
- Dark mode with CSS variable theming
- PWA support with service worker
- Prometheus `/metrics` export endpoint
- Public status page (`/status`)
- CI/CD: PR-triggered lint/test/build + manual deploy via SSH rsync + native ARM64 build
