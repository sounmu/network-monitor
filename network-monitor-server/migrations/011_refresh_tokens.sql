-- Refresh token table for the rotating-short-lived-access-token pattern.
--
-- Plain refresh tokens never live in the database — only their SHA-256 hash
-- is stored. A DB dump therefore cannot be replayed. The client keeps the
-- plaintext in an httpOnly+Secure+SameSite=Strict cookie.
--
-- "Family" rows tie every rotation of a given login session together. When
-- a token is rotated, the old row is marked revoked and a new row is
-- inserted with `parent_id` pointing at the previous one and the same
-- `family_id`. If a client ever presents a revoked token again (classic
-- sign of theft / replay), the server revokes the **entire family** plus
-- stamps users.tokens_revoked_at, logging the user out everywhere.

CREATE TABLE IF NOT EXISTS refresh_tokens (
    id BIGSERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- 32-byte SHA-256 digest. The plaintext is 256 bits of OsRng output and
    -- is never stored anywhere on the server side.
    token_hash BYTEA NOT NULL,
    -- 16-byte random family id (binds all rotations of one session).
    family_id BYTEA NOT NULL,
    parent_id BIGINT REFERENCES refresh_tokens(id) ON DELETE SET NULL,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    -- Best-effort audit metadata (for forensic review; never trusted for auth).
    user_agent TEXT,
    ip TEXT
);

-- Primary lookup path: hash-based verification must be O(1).
CREATE UNIQUE INDEX IF NOT EXISTS idx_refresh_tokens_hash
    ON refresh_tokens(token_hash);

-- Used by family-wide revocation on reuse detection.
CREATE INDEX IF NOT EXISTS idx_refresh_tokens_family
    ON refresh_tokens(family_id);

-- Used by the per-user cleanup path during logout / admin kill-switch.
CREATE INDEX IF NOT EXISTS idx_refresh_tokens_user_active
    ON refresh_tokens(user_id)
    WHERE revoked_at IS NULL;

-- Background eviction of expired rows.
CREATE INDEX IF NOT EXISTS idx_refresh_tokens_expires_at
    ON refresh_tokens(expires_at);
