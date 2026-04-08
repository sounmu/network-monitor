# Multi-stage build: compile on the server's native architecture (no cross-compilation)
FROM rust:1.94.1 AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./

# Build dependencies only (cached until Cargo.toml/Cargo.lock change)
RUN mkdir src && echo "fn main() {}" > src/main.rs \
    && cargo build --release \
    && rm -rf src

# Copy migrations (embedded at compile time by sqlx::migrate!())
COPY migrations/ ./migrations/

# Build actual source
COPY src/ ./src/
RUN touch src/main.rs && cargo build --release && strip target/release/network-monitor-server

# Minimal runtime image (debian-slim is ~52MB vs ubuntu ~77MB)
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd -r monitor && useradd -r -g monitor -s /usr/sbin/nologin monitor

WORKDIR /app
COPY --from=builder /build/target/release/network-monitor-server /app/server
RUN chown -R monitor:monitor /app

USER monitor
EXPOSE 3000
CMD ["/app/server"]
