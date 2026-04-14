-- External HTTP and Ping monitors (server-side probes)

-- HTTP endpoint monitors
CREATE TABLE IF NOT EXISTS http_monitors (
    id              SERIAL PRIMARY KEY,
    name            TEXT NOT NULL,
    url             TEXT NOT NULL,
    method          TEXT NOT NULL DEFAULT 'GET',
    expected_status INT NOT NULL DEFAULT 200,
    interval_secs   INT NOT NULL DEFAULT 60,
    timeout_ms      INT NOT NULL DEFAULT 10000,
    enabled         BOOLEAN NOT NULL DEFAULT true,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS http_monitor_results (
    id              BIGSERIAL PRIMARY KEY,
    monitor_id      INT NOT NULL REFERENCES http_monitors(id) ON DELETE CASCADE,
    status_code     INT,
    response_time_ms INT,
    error           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_http_results_monitor_time
    ON http_monitor_results (monitor_id, created_at DESC);

-- Ping (TCP connect) monitors
CREATE TABLE IF NOT EXISTS ping_monitors (
    id              SERIAL PRIMARY KEY,
    name            TEXT NOT NULL,
    host            TEXT NOT NULL,
    interval_secs   INT NOT NULL DEFAULT 60,
    timeout_ms      INT NOT NULL DEFAULT 5000,
    enabled         BOOLEAN NOT NULL DEFAULT true,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS ping_results (
    id              BIGSERIAL PRIMARY KEY,
    monitor_id      INT NOT NULL REFERENCES ping_monitors(id) ON DELETE CASCADE,
    rtt_ms          DOUBLE PRECISION,
    success         BOOLEAN NOT NULL,
    error           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_ping_results_monitor_time
    ON ping_results (monitor_id, created_at DESC);
