# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

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
