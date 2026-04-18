//! Refresh-token rotation, hashing, and reuse detection.
//!
//! This is the "hard part" of the short-access-token + long-refresh-token
//! pattern. Done wrong, rotation becomes a liability; done right, it adds
//! a theft-detection signal that bare JWTs cannot provide.
//!
//! Threat model addressed:
//! 1. **DB exfiltration** — only SHA-256 hashes are stored, so a dump
//!    cannot be replayed.
//! 2. **Cookie theft** — refresh tokens rotate on every use; a stolen
//!    cookie is single-use and burns out on first legitimate refresh.
//! 3. **Cookie theft, attacker refreshes first** — the victim's next
//!    refresh presents a row that is already `revoked_at`-stamped. We
//!    interpret that as reuse, revoke the entire family, and stamp
//!    `users.tokens_revoked_at`. Both parties are logged out and the
//!    theft is surfaced via server logs.
//! 4. **Long-running breach** — the 14-day absolute lifetime caps
//!    damage regardless of activity.
//!
//! Not addressed here (handled elsewhere or out of scope):
//! - XSS that exfiltrates the access token from memory. The short 15 m
//!   access TTL still reduces damage versus the old 24 h.
//! - Compromised JWT_SECRET; the rotation-contract tests cover that.

use argon2::password_hash::rand_core::{OsRng, RngCore};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::errors::AppError;
use crate::repositories::refresh_tokens_repo::{self, RefreshTokenRow};

/// Lifetime of a refresh token from issuance to hard expiry.
pub const REFRESH_TTL_DAYS: i64 = 14;
/// Lifetime of an access token. The refresh cadence is governed by this —
/// a shorter access TTL means the client refreshes more often, each
/// refresh rotates the cookie, and a stolen cookie burns out sooner.
pub const ACCESS_TTL_MINUTES: i64 = 15;

/// Size of the plaintext refresh token in bytes (256 bits of entropy).
const TOKEN_BYTES: usize = 32;
/// Size of the family identifier in bytes.
const FAMILY_BYTES: usize = 16;

/// Shape of what `issue_new_family` and `rotate` return: the plaintext
/// token (to be put into a Set-Cookie header), the authenticated user id,
/// and the hard expiry of the new token.
#[allow(dead_code)]
pub struct IssuedRefreshToken {
    pub plaintext: String,
    pub user_id: i32,
    pub expires_at: DateTime<Utc>,
}

/// Hash a plaintext refresh token into its DB-storable form.
///
/// Why SHA-256 and not argon2: we store *high-entropy random tokens*,
/// not low-entropy user passwords. A dictionary attack is impossible,
/// so a slow hash adds cost with no security gain. SHA-256 is constant
/// time at this length and fits cleanly in a 32-byte `BYTEA`.
fn hash_token(plaintext: &str) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(plaintext.as_bytes());
    hasher.finalize().to_vec()
}

fn random_token_bytes() -> [u8; TOKEN_BYTES] {
    let mut buf = [0u8; TOKEN_BYTES];
    OsRng.fill_bytes(&mut buf);
    buf
}

fn random_family_id() -> [u8; FAMILY_BYTES] {
    let mut buf = [0u8; FAMILY_BYTES];
    OsRng.fill_bytes(&mut buf);
    buf
}

fn encode_plaintext(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Issue a brand-new refresh token for a successful password login. Starts
/// a fresh family — the result is not linked to any prior session.
pub async fn issue_new_family(
    pool: &PgPool,
    user_id: i32,
    user_agent: Option<&str>,
    ip: Option<&str>,
) -> Result<IssuedRefreshToken, AppError> {
    let plaintext = encode_plaintext(&random_token_bytes());
    let token_hash = hash_token(&plaintext);
    let family_id = random_family_id();
    let expires_at = Utc::now() + Duration::days(REFRESH_TTL_DAYS);

    refresh_tokens_repo::insert(
        pool,
        user_id,
        &token_hash,
        &family_id,
        None,
        expires_at,
        user_agent,
        ip,
    )
    .await
    .map_err(|e| AppError::Internal(format!("Failed to persist refresh token: {e}")))?;

    Ok(IssuedRefreshToken {
        plaintext,
        user_id,
        expires_at,
    })
}

/// Result of presenting a refresh token to `rotate`. Distinct variants
/// let the handler decide how to react (success vs. theft vs. generic
/// rejection).
pub enum RotateOutcome {
    /// The presented token was valid and has been replaced.
    Rotated(IssuedRefreshToken),
    /// The token was once valid but is already revoked. Treated as a
    /// theft signal — the whole family has been burned and the user's
    /// `tokens_revoked_at` cutoff has been raised. The caller should
    /// respond with 401 and wipe the client cookie.
    ReuseDetected { user_id: i32 },
    /// The token was never known, or has already expired, or was not a
    /// valid refresh token at all. Indistinguishable from the client's
    /// side — the caller should respond with 401.
    Rejected,
}

/// Verify and rotate a presented refresh token in one atomic step:
///   1. Look up by SHA-256 hash.
///   2. Check expiry.
///   3. If already revoked → reuse detection path.
///   4. Otherwise mark the old row revoked and insert a new row in the
///      same family, pointing `parent_id` at the old row.
///
/// Callers should follow up with `issue_access_token_and_cookie` in the
/// handler layer.
pub async fn rotate(
    pool: &PgPool,
    presented: &str,
    user_agent: Option<&str>,
    ip: Option<&str>,
) -> Result<RotateOutcome, AppError> {
    if presented.is_empty() {
        return Ok(RotateOutcome::Rejected);
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to begin transaction: {e}")))?;

    let token_hash = hash_token(presented);
    let existing = sqlx::query_as::<_, RefreshTokenRow>(
        r#"
        SELECT id, user_id, family_id, parent_id, issued_at, expires_at, revoked_at
        FROM refresh_tokens
        WHERE token_hash = $1
        FOR UPDATE
        "#,
    )
    .bind(&token_hash)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| AppError::Internal(format!("Failed to look up refresh token: {e}")))?;

    let Some(row) = existing else {
        tx.rollback().await.ok();
        return Ok(RotateOutcome::Rejected);
    };

    if row.expires_at <= Utc::now() {
        tx.rollback().await.ok();
        return Ok(RotateOutcome::Rejected);
    }

    if row.revoked_at.is_some() {
        tx.rollback().await.ok();
        return Ok(handle_reuse_detected(pool, &row).await);
    }

    let new_plain = encode_plaintext(&random_token_bytes());
    let new_hash = hash_token(&new_plain);
    let new_expires_at = Utc::now() + Duration::days(REFRESH_TTL_DAYS);

    let revoke_result = sqlx::query(
        "UPDATE refresh_tokens SET revoked_at = NOW() WHERE id = $1 AND revoked_at IS NULL",
    )
    .bind(row.id)
    .execute(&mut *tx)
    .await
    .map_err(|e| AppError::Internal(format!("Failed to revoke rotated token: {e}")))?;

    if revoke_result.rows_affected() != 1 {
        tx.rollback().await.ok();
        return Ok(handle_reuse_detected(pool, &row).await);
    }

    sqlx::query(
        r#"
        INSERT INTO refresh_tokens
            (user_id, token_hash, family_id, parent_id, expires_at, user_agent, ip)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(row.user_id)
    .bind(&new_hash)
    .bind(&row.family_id)
    .bind(Some(row.id))
    .bind(new_expires_at)
    .bind(user_agent)
    .bind(ip)
    .execute(&mut *tx)
    .await
    .map_err(|e| AppError::Internal(format!("Failed to insert rotated refresh token: {e}")))?;

    tx.commit()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to commit rotation: {e}")))?;

    Ok(RotateOutcome::Rotated(IssuedRefreshToken {
        plaintext: new_plain,
        user_id: row.user_id,
        expires_at: new_expires_at,
    }))
}

/// Reuse-detected recovery: burn the family, raise the user's JWT cutoff,
/// and surface the event in logs. Best-effort — a DB hiccup here still
/// lets the caller respond with 401, which is the primary security goal.
async fn handle_reuse_detected(pool: &PgPool, row: &RefreshTokenRow) -> RotateOutcome {
    tracing::warn!(
        user_id = row.user_id,
        row_id = row.id,
        "🚨 [Auth] Refresh token reuse detected — revoking family and all JWTs for user"
    );

    if let Err(e) = refresh_tokens_repo::revoke_family(pool, &row.family_id).await {
        tracing::error!(err = ?e, "⚠️ [Auth] Failed to revoke family on reuse detection");
    }
    // Also raise the JWT cutoff so any still-live access token issued inside
    // this compromised session is rejected immediately.
    if let Err(e) = crate::repositories::users_repo::revoke_user_tokens(pool, row.user_id).await {
        tracing::error!(err = ?e, "⚠️ [Auth] Failed to stamp tokens_revoked_at on reuse detection");
    }
    let now = Utc::now().timestamp();
    crate::services::auth::update_tokens_revoked_at(row.user_id, now);

    RotateOutcome::ReuseDetected {
        user_id: row.user_id,
    }
}

/// Revoke a single presented refresh token (logout flow). Silent on an
/// unknown or already-revoked token — the client already considers
/// itself logged out.
pub async fn revoke_single(pool: &PgPool, presented: &str) -> Result<(), AppError> {
    if presented.is_empty() {
        return Ok(());
    }
    let token_hash = hash_token(presented);
    if let Some(row) = refresh_tokens_repo::find_by_hash(pool, &token_hash)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to look up refresh token: {e}")))?
    {
        refresh_tokens_repo::revoke_by_id(pool, row.id)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to revoke refresh token: {e}")))?;
    }
    Ok(())
}

/// Revoke every live refresh token for a user. Used by server logout and
/// the admin kill-switch (complements `users.tokens_revoked_at`).
pub async fn revoke_all_for_user(pool: &PgPool, user_id: i32) -> Result<(), AppError> {
    refresh_tokens_repo::revoke_all_for_user(pool, user_id)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to revoke refresh tokens: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_token_is_deterministic() {
        let t = "abc";
        assert_eq!(hash_token(t), hash_token(t));
    }

    #[test]
    fn test_hash_token_differs_for_different_inputs() {
        assert_ne!(hash_token("a"), hash_token("b"));
    }

    #[test]
    fn test_hash_token_length_is_32_bytes() {
        assert_eq!(
            hash_token("whatever").len(),
            32,
            "SHA-256 produces 32 bytes"
        );
    }

    #[test]
    fn test_random_token_bytes_are_unique() {
        let a = random_token_bytes();
        let b = random_token_bytes();
        assert_ne!(a, b, "CSPRNG must not collide at 256-bit width");
    }

    #[test]
    fn test_encode_plaintext_is_url_safe_no_padding() {
        let bytes = [0u8; 32];
        let s = encode_plaintext(&bytes);
        assert!(!s.contains('='), "base64url encoded token must not pad");
        assert!(!s.contains('+') && !s.contains('/'), "must be URL-safe");
    }
}
