-- Enable TimescaleDB compression on hypertables that store time-series data.
-- Compresses chunks older than 7 days, reducing storage by ~50-70%.

-- alert_history: segment by host_key for efficient per-host queries on compressed chunks
ALTER TABLE alert_history SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'host_key'
);

SELECT add_compression_policy(
    'alert_history',
    compress_after => INTERVAL '7 days',
    if_not_exists => TRUE
);

-- http_monitor_results: segment by monitor_id
ALTER TABLE http_monitor_results SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'monitor_id'
);

SELECT add_compression_policy(
    'http_monitor_results',
    compress_after => INTERVAL '7 days',
    if_not_exists => TRUE
);

-- ping_results: segment by monitor_id
ALTER TABLE ping_results SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'monitor_id'
);

SELECT add_compression_policy(
    'ping_results',
    compress_after => INTERVAL '7 days',
    if_not_exists => TRUE
);
