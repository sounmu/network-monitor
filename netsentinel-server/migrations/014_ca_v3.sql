-- Recreate metrics_5min CA to include JSONB snapshot columns.
--
-- Problem: The previous CA only stored scalar aggregates (cpu, memory, load,
-- network bytes). Queries >6h returned NULL for disks, temperatures, gpus,
-- and docker_stats, causing those charts to disappear on long time ranges.
--
-- Solution: Use TimescaleDB's last(value, time) aggregate to preserve the
-- final JSONB snapshot in each 5-minute bucket. This keeps per-element
-- granularity (per-disk, per-container, per-sensor) without needing
-- jsonb_array_elements (which CAs don't support).
--
-- Pattern for future metrics:
--   Scalar (single number)     → AVG/MAX/MIN in CA
--   JSONB snapshot (array)     → last(column, timestamp) in CA
--   Cumulative counter         → MAX in CA

-- Step 1: Remove the existing refresh policy
SELECT remove_continuous_aggregate_policy('metrics_5min', if_exists => TRUE);

-- Step 2: Drop the old CA
DROP MATERIALIZED VIEW IF EXISTS metrics_5min CASCADE;

-- Step 3: Recreate with JSONB snapshot columns
CREATE MATERIALIZED VIEW metrics_5min
WITH (timescaledb.continuous) AS
SELECT
    host_key,
    time_bucket('5 minutes', timestamp) AS bucket,
    -- Scalar aggregates
    AVG(cpu_usage_percent)::REAL       AS cpu_usage_percent,
    AVG(memory_usage_percent)::REAL    AS memory_usage_percent,
    AVG(load_1min)::REAL               AS load_1min,
    AVG(load_5min)::REAL               AS load_5min,
    AVG(load_15min)::REAL              AS load_15min,
    bool_and(is_online)                AS is_online,
    COUNT(*)::INT                      AS sample_count,
    -- Cumulative counters (end-of-bucket value)
    MAX((networks->>'total_rx_bytes')::BIGINT) AS total_rx_bytes,
    MAX((networks->>'total_tx_bytes')::BIGINT) AS total_tx_bytes,
    -- JSONB snapshots (last value in each 5-min bucket)
    last(disks, timestamp)             AS disks,
    last(temperatures, timestamp)      AS temperatures,
    last(gpus, timestamp)              AS gpus,
    last(docker_stats, timestamp)      AS docker_stats
FROM metrics
GROUP BY host_key, time_bucket('5 minutes', timestamp)
WITH NO DATA;

-- Step 4: Re-add refresh policy
SELECT add_continuous_aggregate_policy('metrics_5min',
    start_offset    => INTERVAL '3 days',
    end_offset      => INTERVAL '5 minutes',
    schedule_interval => INTERVAL '5 minutes',
    if_not_exists   => TRUE
);

-- Step 5: Recreate index
CREATE INDEX IF NOT EXISTS idx_metrics_5min_host_bucket
    ON metrics_5min (host_key, bucket DESC);
