-- NetSentinel SQLite schema — Phase 1 draft.
--
-- This file is NOT yet executed by the running binary. Phase 2 wires
-- `sqlx::migrate!()` to this directory when the `backend-sqlite`
-- feature is active. Schema choices are recorded here so Phase 2 only
-- has to swap the pool type, not re-design storage.
--
-- Conventions (see docs/SQLITE_MIGRATION.md §4):
--   • Timestamps: INTEGER epoch seconds UTC. All application conversions
--     happen at the repo boundary.
--   • JSON columns: TEXT holding json1 payloads.
--   • Booleans: INTEGER 0/1 with CHECK (col IN (0,1)).
--   • STRICT tables everywhere — forces column type affinity.
--   • WITHOUT ROWID on narrow hot-path tables with a compact natural PK.
--   • UNIQUE NULLS NOT DISTINCT requires SQLite ≥ 3.45 (bundled).
--
-- Foreign keys rely on `PRAGMA foreign_keys = ON` applied per connection
-- by the SqliteConnectOptions layer (Phase 2).

-- ── Users / auth ─────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS users (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    username              TEXT NOT NULL UNIQUE,
    password_hash         TEXT NOT NULL,
    role                  TEXT NOT NULL DEFAULT 'viewer'
                          CHECK (role IN ('admin','viewer')),
    password_changed_at   INTEGER,
    tokens_revoked_at     INTEGER,
    created_at            INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    updated_at            INTEGER NOT NULL DEFAULT (strftime('%s','now'))
) STRICT;

-- Column list mirrors migrations/011_refresh_tokens.sql on the Postgres
-- side: BYTEA → BLOB, BIGSERIAL → INTEGER PRIMARY KEY AUTOINCREMENT,
-- TIMESTAMPTZ → INTEGER epoch. `family_id` groups all rotated tokens
-- so reuse-detection can revoke them atomically.
CREATE TABLE IF NOT EXISTS refresh_tokens (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash      BLOB NOT NULL UNIQUE,
    family_id       BLOB NOT NULL,
    parent_id       INTEGER REFERENCES refresh_tokens(id) ON DELETE SET NULL,
    issued_at       INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    expires_at      INTEGER NOT NULL,
    revoked_at      INTEGER,
    user_agent      TEXT,
    ip              TEXT
) STRICT;

CREATE INDEX IF NOT EXISTS idx_refresh_tokens_user
    ON refresh_tokens(user_id, expires_at DESC);

CREATE INDEX IF NOT EXISTS idx_refresh_tokens_family
    ON refresh_tokens(family_id);

-- ── Hosts (agent registry) ───────────────────────────────────────
CREATE TABLE IF NOT EXISTS hosts (
    host_key                TEXT PRIMARY KEY,
    display_name            TEXT NOT NULL,
    scrape_interval_secs    INTEGER NOT NULL DEFAULT 10
                            CHECK (scrape_interval_secs BETWEEN 1 AND 3600),
    load_threshold          REAL NOT NULL DEFAULT 4.0,
    -- JSON arrays (replaces Postgres INT[] / TEXT[]).
    ports                   TEXT NOT NULL DEFAULT '[]',
    containers              TEXT NOT NULL DEFAULT '[]',
    os_info                 TEXT,
    cpu_model               TEXT,
    memory_total_mb         INTEGER,
    boot_time               INTEGER,
    ip_address              TEXT,
    system_info_updated_at  INTEGER,
    created_at              INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    updated_at              INTEGER NOT NULL DEFAULT (strftime('%s','now'))
) STRICT, WITHOUT ROWID;

-- ── Alert configuration ──────────────────────────────────────────
CREATE TABLE IF NOT EXISTS alert_configs (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    host_key        TEXT REFERENCES hosts(host_key) ON DELETE CASCADE,
    metric_type     TEXT NOT NULL
                    CHECK (metric_type IN
                        ('cpu','memory','disk','load','network','temperature','gpu')),
    sub_key         TEXT,
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
    threshold       REAL NOT NULL,
    sustained_secs  INTEGER NOT NULL DEFAULT 300
                    CHECK (sustained_secs BETWEEN 0 AND 3600),
    cooldown_secs   INTEGER NOT NULL DEFAULT 1800
                    CHECK (cooldown_secs BETWEEN 0 AND 86400),
    created_at      INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    updated_at      INTEGER NOT NULL DEFAULT (strftime('%s','now'))
    -- Uniqueness is enforced below by an expression-based UNIQUE INDEX
    -- because SQLite's `UNIQUE(...)` column constraint treats each NULL
    -- as distinct — the opposite of the Postgres `UNIQUE NULLS NOT
    -- DISTINCT` we rely on. `coalesce(col, '')` collapses NULLs to a
    -- single sentinel so the index enforces one global row per
    -- (metric_type, sub_key).
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_alert_configs_scope_unique
    ON alert_configs (
        coalesce(host_key, ''),
        metric_type,
        coalesce(sub_key, '')
    );

CREATE INDEX IF NOT EXISTS idx_alert_configs_host
    ON alert_configs(host_key, metric_type);

-- ── Notification channels ────────────────────────────────────────
CREATE TABLE IF NOT EXISTS notification_channels (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    name            TEXT NOT NULL,
    channel_type    TEXT NOT NULL
                    CHECK (channel_type IN ('discord','slack','email')),
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
    config          TEXT NOT NULL DEFAULT '{}',
    created_at      INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    updated_at      INTEGER NOT NULL DEFAULT (strftime('%s','now'))
) STRICT;

-- ── Alert event log ──────────────────────────────────────────────
-- Postgres was a TimescaleDB hypertable; here we're a plain table with
-- a retention worker (docs/SQLITE_MIGRATION.md §5.2). Partitioning is
-- unnecessary at SQLite's target scale.
CREATE TABLE IF NOT EXISTS alert_history (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    host_key        TEXT NOT NULL,
    alert_type      TEXT NOT NULL,
    message         TEXT NOT NULL,
    created_at      INTEGER NOT NULL DEFAULT (strftime('%s','now'))
) STRICT;

CREATE INDEX IF NOT EXISTS idx_alert_history_host_time
    ON alert_history(host_key, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_alert_history_time
    ON alert_history(created_at DESC);

-- ── Dashboard layouts ────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS dashboard_layouts (
    user_id         INTEGER PRIMARY KEY
                    REFERENCES users(id) ON DELETE CASCADE,
    widgets         TEXT NOT NULL DEFAULT '[]',
    updated_at      INTEGER NOT NULL DEFAULT (strftime('%s','now'))
) STRICT, WITHOUT ROWID;

-- ── External monitors ────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS http_monitors (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    name             TEXT NOT NULL,
    url              TEXT NOT NULL,
    method           TEXT NOT NULL DEFAULT 'GET',
    expected_status  INTEGER NOT NULL DEFAULT 200
                     CHECK (expected_status BETWEEN 100 AND 599),
    interval_secs    INTEGER NOT NULL DEFAULT 60
                     CHECK (interval_secs BETWEEN 10 AND 3600),
    timeout_ms       INTEGER NOT NULL DEFAULT 5000
                     CHECK (timeout_ms BETWEEN 1000 AND 30000),
    enabled          INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
    created_at       INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    updated_at       INTEGER NOT NULL DEFAULT (strftime('%s','now'))
) STRICT;

CREATE TABLE IF NOT EXISTS http_monitor_results (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    monitor_id        INTEGER NOT NULL
                      REFERENCES http_monitors(id) ON DELETE CASCADE,
    status_code       INTEGER,
    response_time_ms  INTEGER,
    error             TEXT,
    created_at        INTEGER NOT NULL DEFAULT (strftime('%s','now'))
) STRICT;

CREATE INDEX IF NOT EXISTS idx_http_results_monitor_time
    ON http_monitor_results(monitor_id, created_at DESC);

CREATE TABLE IF NOT EXISTS ping_monitors (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    name            TEXT NOT NULL,
    host            TEXT NOT NULL,
    interval_secs   INTEGER NOT NULL DEFAULT 60
                    CHECK (interval_secs BETWEEN 10 AND 3600),
    timeout_ms      INTEGER NOT NULL DEFAULT 5000
                    CHECK (timeout_ms BETWEEN 1000 AND 30000),
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
    created_at      INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    updated_at      INTEGER NOT NULL DEFAULT (strftime('%s','now'))
) STRICT;

CREATE TABLE IF NOT EXISTS ping_results (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    monitor_id  INTEGER NOT NULL
                REFERENCES ping_monitors(id) ON DELETE CASCADE,
    -- RTT is stored as REAL (DOUBLE PRECISION on Postgres) so sub-ms
    -- measurements survive round-trip. An INTEGER column would force
    -- rounding and trips sqlx-sqlite's `cannot store REAL value` guard
    -- under STRICT mode.
    rtt_ms      REAL,
    success     INTEGER NOT NULL CHECK (success IN (0,1)),
    error       TEXT,
    created_at  INTEGER NOT NULL DEFAULT (strftime('%s','now'))
) STRICT;

CREATE INDEX IF NOT EXISTS idx_ping_results_monitor_time
    ON ping_results(monitor_id, created_at DESC);

-- ── Raw metrics (replaces the TimescaleDB hypertable) ────────────
-- 3-day retention enforced by the retention worker (§5.2). Older data
-- lives in metrics_5min. Column list mirrors the Postgres table plus
-- `timestamp` renamed to match the MetricsRow struct field (sqlx-sqlite
-- decodes DateTime<Utc> from INTEGER epoch so an `_unix` suffix would
-- just force pointless aliasing in every SELECT).
CREATE TABLE IF NOT EXISTS metrics (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    host_key              TEXT NOT NULL,
    display_name          TEXT NOT NULL,
    is_online             INTEGER NOT NULL CHECK (is_online IN (0,1)),
    cpu_usage_percent     REAL,
    memory_usage_percent  REAL,
    load_1min             REAL,
    load_5min             REAL,
    load_15min            REAL,
    networks              TEXT,
    docker_containers     TEXT,
    ports                 TEXT,
    disks                 TEXT,
    processes             TEXT,
    temperatures          TEXT,
    gpus                  TEXT,
    cpu_cores             TEXT,
    network_interfaces    TEXT,
    docker_stats          TEXT,
    timestamp             INTEGER NOT NULL DEFAULT (strftime('%s','now'))
) STRICT;

CREATE INDEX IF NOT EXISTS idx_metrics_host_time
    ON metrics(host_key, timestamp DESC);

CREATE INDEX IF NOT EXISTS idx_metrics_time
    ON metrics(timestamp DESC);

-- ── 5-minute rollup (replaces TimescaleDB continuous aggregate) ──
-- Maintained by the rollup worker (§5.1). Composite PK on
-- (host_key, bucket_unix) gives us an index-only lookup for the hot
-- dashboard queries. WITHOUT ROWID drops the implicit rowid slot.
-- `bucket` matches the Postgres continuous-aggregate column name so
-- both backends share the same SQL column identifier.
CREATE TABLE IF NOT EXISTS metrics_5min (
    host_key              TEXT NOT NULL,
    bucket                INTEGER NOT NULL,  -- floor(ts / 300) * 300
    cpu_usage_percent     REAL,
    memory_usage_percent  REAL,
    load_1min             REAL,
    load_5min             REAL,
    load_15min            REAL,
    is_online             INTEGER,
    sample_count          INTEGER NOT NULL DEFAULT 0,
    total_rx_bytes        INTEGER,
    total_tx_bytes        INTEGER,
    -- JSONB "last-in-bucket" snapshots (see §5.1).
    disks                 TEXT,
    temperatures          TEXT,
    gpus                  TEXT,
    docker_stats          TEXT,
    PRIMARY KEY (host_key, bucket)
) STRICT, WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS idx_metrics_5min_time
    ON metrics_5min(bucket DESC);
