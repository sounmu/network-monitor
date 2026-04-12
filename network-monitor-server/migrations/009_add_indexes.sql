-- Index on users.username for O(1) login lookups (was full table scan)
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_username ON users (username);

-- Partial indexes on enabled monitors — scraped every 10s, was full table scan
CREATE INDEX IF NOT EXISTS idx_http_monitors_enabled ON http_monitors (enabled) WHERE enabled = true;
CREATE INDEX IF NOT EXISTS idx_ping_monitors_enabled ON ping_monitors (enabled) WHERE enabled = true;
