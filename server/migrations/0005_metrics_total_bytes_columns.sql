-- Cumulative network counter projections for raw metrics rows.
--
-- The chart endpoint's ≤1h branch (and `fetch_metrics_range`'s ≤6h branch)
-- previously did `CAST(json_extract(networks, '$.total_rx_bytes') AS INTEGER)`
-- per row. With v0.3.0+ payload growth (per-interface arrays inside the
-- `networks` JSON), that JSON parse on every row was the dominant cost of
-- a 1 h × 6 samples/min × N hosts chart load.
--
-- Mirrors migration 0003's approach for `rx_bytes_per_sec`/`tx_bytes_per_sec`:
-- duplicate the scalar inside the JSON into a real column so SQLite reads
-- it directly. Nullable for backward compat with rows inserted before this
-- migration; the read path treats NULL as "fall back to json_extract on
-- the row's `networks` blob if needed", though in practice the rollup
-- worker's 60 s tick will repopulate the rollup table from raw rows that
-- do have the columns within minutes of deployment.

ALTER TABLE metrics ADD COLUMN total_rx_bytes INTEGER;
ALTER TABLE metrics ADD COLUMN total_tx_bytes INTEGER;
