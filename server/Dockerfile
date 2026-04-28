# ─────────────────────────────────────────────────────────────────────────
# Multi-stage build that produces a single container serving BOTH the
# Axum API on /api/* and the statically-exported Next.js web bundle from
# /. Drops the previous separate-web-container layout + its ~35 MB Node
# runtime. Local dev still uses `npm run dev` on port 3001.
# ─────────────────────────────────────────────────────────────────────────

# ── Stage 1 — web: `next build` with `output: 'export'` ────────────────
FROM node:20-alpine AS web-builder

WORKDIR /web
COPY web/package.json web/package-lock.json* ./
RUN npm ci

# NEXT_PUBLIC_API_URL is baked into the bundle at build time. Same
# contract as before: whatever URL the browser fetches from must appear
# in the server's ALLOWED_ORIGINS list. Default to same-origin so a
# homelab single-container deploy "just works" with no env required.
ARG NEXT_PUBLIC_API_URL=""
ENV NEXT_PUBLIC_API_URL=$NEXT_PUBLIC_API_URL
ENV NEXT_TELEMETRY_DISABLED=1

COPY web/ ./
RUN npm run build
# Next.js with `output: 'export'` writes its static bundle to /web/out/

# ── Stage 2 — server: compile the Rust binary ─────────────────────────
FROM rust:1.94.1 AS server-builder

WORKDIR /build
COPY server/Cargo.toml server/Cargo.lock ./

# Warm the dependency cache with an empty src/main.rs so subsequent
# source edits only recompile the binary, not every crate.
RUN mkdir src && echo "fn main() {}" > src/main.rs \
    && cargo build --release \
    && rm -rf src

# Migrations are embedded at compile time via `sqlx::migrate!()`.
COPY server/migrations/ ./migrations/

COPY server/src/ ./src/
RUN touch src/main.rs && cargo build --release && strip target/release/netsentinel-server

# ── Stage 3 — runtime: minimal debian + binary + static bundle ────────
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
# `curl` is required for the healthcheck defined in docker-compose.yml
# (`curl -sf http://localhost:3000/api/health`). ~3 MB runtime cost —
# worth it for compose `depends_on: condition: service_healthy` wiring.

WORKDIR /app
COPY --from=server-builder /build/target/release/netsentinel-server /app/server
COPY --from=web-builder    /web/out                                  /app/static

# Persistent SQLite state lives here. Declared as a VOLUME so `docker
# run` (without an explicit bind mount) still gives the file a stable
# home; docker-compose.yml mounts the host's `./data` over it. Container
# runs as root by design — the deployment model assumes Tailscale /
# WireGuard fronting, so the only externally reachable surface is the
# dashboard whose auth, SSRF, and input validation live in the app.
RUN mkdir -p /app/data
VOLUME ["/app/data"]

ENV STATIC_ASSETS_DIR=/app/static

EXPOSE 3000
CMD ["/app/server"]
