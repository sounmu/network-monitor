-- Bandwidth scalar columns on raw metrics remove per-row json_extract
-- from the rollup worker's 60 s tick.
--
-- These duplicate rate fields inside the networks JSON, trading a small
-- write-time projection for zero JSON parsing when rollups average bandwidth.
-- Nullable for backward compatibility with rows inserted before this migration;
-- AVG() ignores NULL so historical buckets stay clean.

ALTER TABLE metrics ADD COLUMN rx_bytes_per_sec REAL;
ALTER TABLE metrics ADD COLUMN tx_bytes_per_sec REAL;
