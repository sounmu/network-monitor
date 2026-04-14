-- Recreate metrics_5min continuous aggregate with network columns.
-- TimescaleDB does not support ALTER on continuous aggregates,
-- so we must drop and recreate.

-- Step 1: Remove the existing refresh policy (required before dropping)
SELECT remove_continuous_aggregate_policy('metrics_5min', if_exists => TRUE);

-- Step 2: Drop the old CA
DROP MATERIALIZED VIEW IF EXISTS metrics_5min CASCADE;

-- Step 3: Recreate with network columns (total_rx_bytes, total_tx_bytes)
-- Network bytes are cumulative counters — MAX gives end-of-bucket value,
-- matching the frontend's consecutive-point rate calculation.
CREATE MATERIALIZED VIEW IF NOT EXISTS metrics_5min
WITH (timescaledb.continuous) AS
SELECT
    host_key,
    time_bucket('5 minutes', timestamp) AS bucket,
    AVG(cpu_usage_percent)::REAL AS cpu_usage_percent,
    AVG(memory_usage_percent)::REAL AS memory_usage_percent,
    AVG(load_1min)::REAL AS load_1min,
    AVG(load_5min)::REAL AS load_5min,
    AVG(load_15min)::REAL AS load_15min,
    bool_and(is_online) AS is_online,
    COUNT(*)::INT AS sample_count,
    MAX((networks->>'total_rx_bytes')::BIGINT) AS total_rx_bytes,
    MAX((networks->>'total_tx_bytes')::BIGINT) AS total_tx_bytes
FROM metrics
GROUP BY host_key, time_bucket('5 minutes', timestamp)
WITH NO DATA;

-- Step 4: Re-add refresh policy (same parameters as before)
SELECT add_continuous_aggregate_policy('metrics_5min',
    start_offset => INTERVAL '3 days',
    end_offset   => INTERVAL '5 minutes',
    schedule_interval => INTERVAL '5 minutes',
    if_not_exists => TRUE
);

-- Step 5: Index for the primary query pattern (WHERE host_key = ? AND bucket >= ?)
CREATE INDEX IF NOT EXISTS idx_metrics_5min_host_bucket
    ON metrics_5min (host_key, bucket DESC);

-- Note: CALL refresh_continuous_aggregate() cannot run inside a transaction block
-- (sqlx wraps each migration in a transaction). The initial CA seed is performed
-- by the server startup code in main.rs instead.
