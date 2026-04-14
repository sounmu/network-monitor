-- Track when passwords were last changed for token revocation.
-- Tokens issued before password_changed_at are rejected by AuthGuard/AdminGuard.

ALTER TABLE users ADD COLUMN IF NOT EXISTS password_changed_at
    TIMESTAMPTZ NOT NULL DEFAULT now();
