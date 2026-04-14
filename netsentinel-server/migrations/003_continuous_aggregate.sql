-- 5-minute continuous aggregate for long-range dashboard queries (7d, 30d).
-- Reads ~50x fewer rows from a materialized view instead of scanning raw data.

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
    COUNT(*)::INT AS sample_count
FROM metrics
GROUP BY host_key, time_bucket('5 minutes', timestamp)
WITH NO DATA;

-- Refresh policy: re-aggregate last 3 days every 5 minutes
SELECT add_continuous_aggregate_policy('metrics_5min',
    start_offset => INTERVAL '3 days',
    end_offset   => INTERVAL '5 minutes',
    schedule_interval => INTERVAL '5 minutes',
    if_not_exists => TRUE
);
