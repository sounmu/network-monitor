-- v0.3.0: Expanded metrics — per-core CPU, per-interface network, container resource stats.
-- Disk I/O is embedded in the existing `disks` JSONB column (extended struct).

ALTER TABLE metrics ADD COLUMN IF NOT EXISTS cpu_cores JSONB;
ALTER TABLE metrics ADD COLUMN IF NOT EXISTS network_interfaces JSONB;
ALTER TABLE metrics ADD COLUMN IF NOT EXISTS docker_stats JSONB;
