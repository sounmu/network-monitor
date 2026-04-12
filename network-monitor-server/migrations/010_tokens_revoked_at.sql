-- Track an explicit "revoke all tokens for this user" timestamp, independent
-- of password changes. Used by:
--   * POST /api/auth/logout               — user-initiated session kill
--   * POST /api/admin/users/{id}/revoke-sessions — operator kill-switch
--
-- Tokens whose `iat` claim is older than the user's tokens_revoked_at are
-- rejected by AuthGuard/AdminGuard, the same way password_changed_at already
-- works. NULL means "never revoked" — checked for in the in-memory cache.

ALTER TABLE users ADD COLUMN IF NOT EXISTS tokens_revoked_at TIMESTAMPTZ NULL;
