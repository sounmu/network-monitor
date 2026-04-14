//! Low-level SQL for the `refresh_tokens` table.
//!
//! Hash-based lookups only — the plaintext token never leaves the SHA-256
//! boundary inside `services::refresh_token`. See `migrations/011_refresh_tokens.sql`
//! for the full schema rationale.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct RefreshTokenRow {
    pub id: i64,
    pub user_id: i32,
    pub family_id: Vec<u8>,
    pub parent_id: Option<i64>,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Insert a new refresh token row and return the generated id.
#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &PgPool,
    user_id: i32,
    token_hash: &[u8],
    family_id: &[u8],
    parent_id: Option<i64>,
    expires_at: DateTime<Utc>,
    user_agent: Option<&str>,
    ip: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let (id,): (i64,) = sqlx::query_as(
        r#"
        INSERT INTO refresh_tokens
            (user_id, token_hash, family_id, parent_id, expires_at, user_agent, ip)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(token_hash)
    .bind(family_id)
    .bind(parent_id)
    .bind(expires_at)
    .bind(user_agent)
    .bind(ip)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Look up a token row by its stored hash. Returns `None` if not present —
/// callers must not distinguish "not found" from "revoked" in the response
/// path (no enumeration oracle).
pub async fn find_by_hash(
    pool: &PgPool,
    token_hash: &[u8],
) -> Result<Option<RefreshTokenRow>, sqlx::Error> {
    sqlx::query_as::<_, RefreshTokenRow>(
        r#"
        SELECT id, user_id, family_id, parent_id, issued_at, expires_at, revoked_at
        FROM refresh_tokens
        WHERE token_hash = $1
        "#,
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
}

/// Mark a single row as revoked. Idempotent — already-revoked rows stay
/// pointing at their original revocation instant.
pub async fn revoke_by_id(pool: &PgPool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE refresh_tokens SET revoked_at = NOW() WHERE id = $1 AND revoked_at IS NULL",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Revoke every live row in the given family. Used when reuse is detected.
pub async fn revoke_family(pool: &PgPool, family_id: &[u8]) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE refresh_tokens SET revoked_at = NOW() WHERE family_id = $1 AND revoked_at IS NULL",
    )
    .bind(family_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// Revoke every live row for a user. Used by logout and admin kill-switch.
pub async fn revoke_all_for_user(pool: &PgPool, user_id: i32) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE refresh_tokens SET revoked_at = NOW() WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// Delete rows that expired more than `grace` ago. Called periodically from
/// a background task so the table does not grow without bound.
pub async fn delete_expired(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let res =
        sqlx::query("DELETE FROM refresh_tokens WHERE expires_at < NOW() - INTERVAL '7 days'")
            .execute(pool)
            .await?;
    Ok(res.rows_affected())
}
