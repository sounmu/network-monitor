-- Covering index for batch-uptime reads on metrics_5min.
--
-- `fetch_batch_uptime_pct(days)` groups by host_key across a rolling
-- `bucket >= now - days*86400` window. Before this index the planner fell
-- back to a full scan of the rollup table: with 90 days × N hosts ×
-- 288 buckets/day the scan cost grows linearly with the host fleet and
-- dominates the `/api/public/status` latency for status-page scrapers
-- that hit the endpoint every minute.
--
-- Column order rationale:
--   • `bucket DESC` — the WHERE filter. DESC matches the retention-worker
--     prune order so the planner can stop at the cutoff.
--   • `host_key` — the GROUP BY key. Placed second so the index is
--     covering for both the range filter and the grouping step.
--   • `is_online`, `sample_count` — both aggregated. Including them in
--     the index lets SQLite satisfy the query entirely from the index
--     without hitting the table, which is what "covering" means here.
--
-- `IF NOT EXISTS` keeps the migration idempotent; existing deployments
-- pay the one-time index build on next server start.

CREATE INDEX IF NOT EXISTS idx_metrics_5min_bucket_host_cover
    ON metrics_5min (bucket DESC, host_key, is_online, sample_count);
