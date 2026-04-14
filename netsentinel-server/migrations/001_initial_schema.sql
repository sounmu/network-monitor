-- Initial schema: core tables for metrics, hosts, alerts, notifications, users, dashboards
-- All statements use IF NOT EXISTS for idempotency with pre-existing databases.

-- Enable TimescaleDB extension (must run before hypertable conversion)
CREATE EXTENSION IF NOT EXISTS timescaledb;

-- Metrics time-series table (no PK on id — TimescaleDB requires partition key in unique constraints)
CREATE TABLE IF NOT EXISTS metrics (
    id                   BIGSERIAL,
    host_key             VARCHAR(255) NOT NULL,
    display_name         VARCHAR(255) NOT NULL,
    is_online            BOOLEAN NOT NULL,
    cpu_usage_percent    REAL NOT NULL,
    memory_usage_percent REAL NOT NULL,
    load_1min            REAL NOT NULL DEFAULT 0.0,
    load_5min            REAL NOT NULL DEFAULT 0.0,
    load_15min           REAL NOT NULL DEFAULT 0.0,
    networks             JSONB,
    docker_containers    JSONB,
    ports                JSONB,
    disks                JSONB,
    processes            JSONB,
    temperatures         JSONB,
    gpus                 JSONB,
    timestamp            TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Composite index for dashboard time-series queries
CREATE INDEX IF NOT EXISTS idx_metrics_host_key_time
    ON metrics (host_key, timestamp DESC);

-- Host (agent) registry
CREATE TABLE IF NOT EXISTS hosts (
    host_key             TEXT PRIMARY KEY,
    display_name         TEXT NOT NULL,
    scrape_interval_secs INT NOT NULL DEFAULT 10,
    load_threshold       FLOAT NOT NULL DEFAULT 4.0,
    ports                INT[] NOT NULL DEFAULT '{80,443}',
    containers           TEXT[] NOT NULL DEFAULT '{}',
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Alert configuration rules (global when host_key IS NULL, per-host when set)
CREATE TABLE IF NOT EXISTS alert_configs (
    id              SERIAL PRIMARY KEY,
    host_key        TEXT REFERENCES hosts(host_key) ON DELETE CASCADE,
    metric_type     TEXT NOT NULL CHECK (metric_type IN ('cpu', 'memory', 'disk')),
    enabled         BOOLEAN NOT NULL DEFAULT true,
    threshold       FLOAT NOT NULL,
    sustained_secs  INT NOT NULL DEFAULT 300,
    cooldown_secs   INT NOT NULL DEFAULT 60,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE NULLS NOT DISTINCT (host_key, metric_type)
);

-- Seed global default alert thresholds
INSERT INTO alert_configs (host_key, metric_type, enabled, threshold, sustained_secs, cooldown_secs)
VALUES (NULL, 'cpu', true, 80.0, 300, 60),
       (NULL, 'memory', true, 90.0, 300, 60),
       (NULL, 'disk', true, 90.0, 0, 300)
ON CONFLICT (host_key, metric_type) DO NOTHING;

-- Notification delivery channels (Discord, Slack, Email)
CREATE TABLE IF NOT EXISTS notification_channels (
    id           SERIAL PRIMARY KEY,
    name         TEXT NOT NULL,
    channel_type TEXT NOT NULL CHECK (channel_type IN ('discord', 'slack', 'email')),
    enabled      BOOLEAN NOT NULL DEFAULT true,
    config       JSONB NOT NULL DEFAULT '{}',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Alert event log
CREATE TABLE IF NOT EXISTS alert_history (
    id           BIGSERIAL PRIMARY KEY,
    host_key     VARCHAR(255) NOT NULL,
    alert_type   VARCHAR(100) NOT NULL,
    message      TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_alert_history_host_time
    ON alert_history (host_key, created_at DESC);

-- User accounts (argon2 password hashes)
CREATE TABLE IF NOT EXISTS users (
    id            SERIAL PRIMARY KEY,
    username      TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    role          TEXT NOT NULL DEFAULT 'admin',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Per-user dashboard widget layout
CREATE TABLE IF NOT EXISTS dashboard_layouts (
    id         SERIAL PRIMARY KEY,
    user_id    INT NOT NULL UNIQUE,
    widgets    JSONB NOT NULL DEFAULT '[]',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
