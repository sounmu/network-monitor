-- Add 90-day retention policies for alert_history, http_monitor_results, ping_results.
-- Converts tables to TimescaleDB hypertables for O(1) chunk-based retention (same as metrics).

-- alert_history: drop PK constraint (hypertable requires partition key in unique constraints)
ALTER TABLE alert_history DROP CONSTRAINT IF EXISTS alert_history_pkey;

SELECT create_hypertable(
    'alert_history', 'created_at',
    chunk_time_interval => INTERVAL '7 days',
    if_not_exists => TRUE,
    migrate_data => TRUE
);

SELECT add_retention_policy(
    'alert_history',
    INTERVAL '90 days',
    if_not_exists => TRUE
);

-- http_monitor_results: drop PK constraint
ALTER TABLE http_monitor_results DROP CONSTRAINT IF EXISTS http_monitor_results_pkey;

SELECT create_hypertable(
    'http_monitor_results', 'created_at',
    chunk_time_interval => INTERVAL '7 days',
    if_not_exists => TRUE,
    migrate_data => TRUE
);

SELECT add_retention_policy(
    'http_monitor_results',
    INTERVAL '90 days',
    if_not_exists => TRUE
);

-- ping_results: drop PK constraint
ALTER TABLE ping_results DROP CONSTRAINT IF EXISTS ping_results_pkey;

SELECT create_hypertable(
    'ping_results', 'created_at',
    chunk_time_interval => INTERVAL '7 days',
    if_not_exists => TRUE,
    migrate_data => TRUE
);

SELECT add_retention_policy(
    'ping_results',
    INTERVAL '90 days',
    if_not_exists => TRUE
);
