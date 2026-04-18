CREATE INDEX IF NOT EXISTS idx_alert_history_created_at
    ON alert_history (created_at DESC);
