-- TimescaleDB hypertable conversion, retention, continuous aggregates, and compression.
-- All operations are idempotent (if_not_exists / .ok() equivalents).

-- Convert metrics table to hypertable: 1-day chunk partitioning
-- ~8,640 rows/host/day at 10-second scrape interval
SELECT create_hypertable(
    'metrics', 'timestamp',
    chunk_time_interval => INTERVAL '1 day',
    if_not_exists => TRUE,
    migrate_data => TRUE
);

-- Automatic retention: drop chunks older than 90 days (O(1) metadata operation)
SELECT add_retention_policy(
    'metrics',
    INTERVAL '90 days',
    if_not_exists => TRUE
);

-- Chunk compression for data older than 7 days (~10x disk reduction)
ALTER TABLE metrics SET (
    timescaledb.compress,
    timescaledb.compress_segmentby = 'host_key',
    timescaledb.compress_orderby = 'timestamp DESC'
);

SELECT add_compression_policy(
    'metrics',
    compress_after => INTERVAL '7 days',
    if_not_exists => TRUE
);
