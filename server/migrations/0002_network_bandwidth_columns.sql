-- Add per-bucket average bandwidth columns to the 5-minute rollup.
--
-- The agent now emits `rx_bytes_per_sec` / `tx_bytes_per_sec` alongside
-- the cumulative `total_*_bytes` counters (NetworkTotal, Option B in
-- the bandwidth-semantics plan). The rollup worker averages those
-- rates across the 5-minute bucket — mirroring how `cpu_usage_percent`
-- is aggregated — so long-range dashboard queries can return a true
-- bandwidth column instead of forcing the frontend to differentiate
-- `MAX(total_rx_bytes)` deltas between adjacent buckets.
--
-- Columns are nullable so existing rollup rows survive the migration
-- unchanged. The next rollup tick after deploy will start filling
-- them in from the new raw samples; older raw rows (pre-upgrade)
-- simply do not contribute to the average.
--
-- SQLite 3.35+ supports `ALTER TABLE ... ADD COLUMN` without requiring
-- a table rebuild. `IF NOT EXISTS` is not supported on ADD COLUMN,
-- so re-running this migration against an already-upgraded schema
-- would error — sqlx::migrate!() records applied migrations and
-- will skip it, so the naive form is safe in practice.

ALTER TABLE metrics_5min ADD COLUMN avg_rx_bytes_per_sec REAL;
ALTER TABLE metrics_5min ADD COLUMN avg_tx_bytes_per_sec REAL;
