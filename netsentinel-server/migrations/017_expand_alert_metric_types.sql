-- Phase 3: expand alert_configs to cover load / network / temperature / gpu,
-- and introduce sub_key for per-sensor / per-interface / per-GPU overrides.

-- 1. Drop legacy CHECK constraint on metric_type so we can widen it.
ALTER TABLE alert_configs
    DROP CONSTRAINT IF EXISTS alert_configs_metric_type_check;

ALTER TABLE alert_configs
    ADD CONSTRAINT alert_configs_metric_type_check
    CHECK (metric_type IN ('cpu', 'memory', 'disk', 'load', 'network', 'temperature', 'gpu'));

-- 2. Add optional sub_key for scoping a rule to a specific sensor/interface/device.
ALTER TABLE alert_configs
    ADD COLUMN IF NOT EXISTS sub_key TEXT NULL;

-- 3. Replace the (host_key, metric_type) uniqueness with (host_key, metric_type, sub_key).
--    NULLS NOT DISTINCT preserves the "NULL host_key means global" convention.
ALTER TABLE alert_configs
    DROP CONSTRAINT IF EXISTS alert_configs_host_key_metric_type_key;

CREATE UNIQUE INDEX IF NOT EXISTS alert_configs_host_metric_sub_idx
    ON alert_configs (host_key, metric_type, sub_key)
    NULLS NOT DISTINCT;

-- 4. Seed global defaults for the newly supported metrics (no-op if rows exist).
INSERT INTO alert_configs (host_key, metric_type, sub_key, enabled, threshold, sustained_secs, cooldown_secs)
VALUES (NULL, 'load',        NULL, false, 4.0,  300, 300),
       (NULL, 'network',     NULL, false, 500000000.0, 300, 600),
       (NULL, 'temperature', NULL, false, 85.0, 120, 600),
       (NULL, 'gpu',         NULL, false, 90.0, 300, 300)
ON CONFLICT DO NOTHING;
