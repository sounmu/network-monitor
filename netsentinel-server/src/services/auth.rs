use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::errors::AppError;
use chrono::Utc;
use chrono_tz::Asia::Seoul;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub exp: usize,
    /// Audience claim — "agent" for agent tokens (token type separation)
    #[serde(default)]
    pub aud: String,
}

pub static ENCODING_KEY: OnceLock<EncodingKey> = OnceLock::new();
pub static DECODING_KEY: OnceLock<DecodingKey> = OnceLock::new();

/// Per-user "tokens issued before this instant are invalid" cutoff cache.
///
/// Shared across two mechanisms:
///   * password change        — `update_password_changed_at`
///   * explicit revoke (logout / admin kill) — `update_tokens_revoked_at`
///
/// Both write into the same map because they mean the same thing to the
/// verification path: the stored timestamp is the earliest `iat` that is
/// still allowed to pass. A new write is kept only if it is strictly
/// *later* than the existing entry, so password change + logout cannot
/// accidentally undo each other.
static TOKEN_REVOCATION_CACHE: OnceLock<Arc<RwLock<HashMap<i32, i64>>>> = OnceLock::new();

pub fn init_encoding_key(secret: &str) {
    let key = EncodingKey::from_secret(secret.as_bytes());
    let _ = ENCODING_KEY.set(key);
    let dk = DecodingKey::from_secret(secret.as_bytes());
    let _ = DECODING_KEY.set(dk);
}

/// Initialize the token revocation cache reference (called from main.rs).
/// The cache is pre-seeded with the latest of `password_changed_at` and
/// `tokens_revoked_at` for each user.
pub fn init_token_revocation_cache(cache: Arc<RwLock<HashMap<i32, i64>>>) {
    let _ = TOKEN_REVOCATION_CACHE.set(cache);
}

/// Internal helper: raise the cutoff for `user_id` to `timestamp`, but never
/// lower it. Both password-change and logout paths feed through here.
fn raise_revocation_cutoff(user_id: i32, timestamp: i64) {
    if let Some(cache) = TOKEN_REVOCATION_CACHE.get()
        && let Ok(mut map) = cache.write()
    {
        map.entry(user_id)
            .and_modify(|existing| {
                if timestamp > *existing {
                    *existing = timestamp;
                }
            })
            .or_insert(timestamp);
    }
}

/// Update the cutoff after a password change.
pub fn update_password_changed_at(user_id: i32, timestamp: i64) {
    raise_revocation_cutoff(user_id, timestamp);
}

/// Update the cutoff after an explicit token revocation (logout / admin kill).
pub fn update_tokens_revoked_at(user_id: i32, timestamp: i64) {
    raise_revocation_cutoff(user_id, timestamp);
}

/// Check if a user JWT's `iat` is after the latest revocation event for that
/// user (password change OR explicit logout). Returns true if the token is
/// still valid. A missing cache entry means the user has never revoked and
/// every signed token with a valid `iat` is accepted.
fn is_token_iat_still_valid(user_id: i32, iat: usize) -> bool {
    let Some(cache) = TOKEN_REVOCATION_CACHE.get() else {
        return true; // Cache not initialized — graceful degradation
    };
    let Ok(map) = cache.read() else {
        return true; // Lock poisoned — graceful degradation
    };
    match map.get(&user_id) {
        Some(&cutoff) => (iat as i64) >= cutoff,
        None => true,
    }
}

pub fn generate_jwt() -> Result<String, AppError> {
    let exp = Utc::now().with_timezone(&Seoul).timestamp() as usize + 60;
    let claims = Claims {
        exp,
        aud: "agent".to_string(),
    };
    let key = ENCODING_KEY
        .get()
        .ok_or_else(|| AppError::Internal("JWT encoding key not initialized".into()))?;
    encode(&Header::new(Algorithm::HS256), &claims, key)
        .map_err(|e| AppError::Internal(format!("JWT encoding failed: {e}")))
}

/// Axum extractor that enforces JWT-based authentication:
///
/// - Agent JWT (HS256, 60s expiry): used by agents during scraping.
/// - User JWT (HS256, 24h expiry): contains sub/username/role, used by web dashboard.
///
/// Either JWT type passing is sufficient. Missing or invalid auth returns 401.
pub struct AuthGuard;

impl<S> FromRequestParts<S> for AuthGuard
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::Unauthorized("Missing Authorization header".to_string()))?;

        let token = auth_header.strip_prefix("Bearer ").ok_or_else(|| {
            AppError::Unauthorized("Authorization header must use Bearer scheme".to_string())
        })?;

        // Try agent JWT first (with aud: "agent"), then legacy agent (no aud), then user JWT
        let decoding_key = DECODING_KEY
            .get()
            .ok_or_else(|| AppError::Internal("DECODING_KEY not initialized".to_string()))?;

        let mut agent_validation = Validation::new(Algorithm::HS256);
        agent_validation.set_audience(&["agent"]);
        if decode::<Claims>(token, decoding_key, &agent_validation).is_ok() {
            return Ok(AuthGuard);
        }
        // Legacy agent tokens without aud claim
        let mut legacy_validation = Validation::new(Algorithm::HS256);
        legacy_validation.validate_aud = false;
        if let Ok(data) = decode::<Claims>(token, decoding_key, &legacy_validation)
            && data.claims.aud.is_empty()
        {
            return Ok(AuthGuard);
        }

        // User JWT (different claims structure) — also check revocation cutoff
        if let Some(claims) = super::user_auth::decode_user_jwt(token) {
            if !is_token_iat_still_valid(claims.sub, claims.iat) {
                return Err(AppError::Unauthorized("Token revoked".to_string()));
            }
            return Ok(AuthGuard);
        }

        Err(AppError::Unauthorized(
            "Invalid or expired token".to_string(),
        ))
    }
}

/// Axum extractor that enforces admin-only access.
/// Only user JWTs with role == "admin" are accepted. Agent JWTs are rejected.
pub struct AdminGuard;

impl<S> FromRequestParts<S> for AdminGuard
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::Unauthorized("Missing Authorization header".to_string()))?;

        let token = auth_header.strip_prefix("Bearer ").ok_or_else(|| {
            AppError::Unauthorized("Authorization header must use Bearer scheme".to_string())
        })?;

        let claims = super::user_auth::decode_user_jwt(token)
            .ok_or_else(|| AppError::Unauthorized("Invalid or expired token".to_string()))?;

        if !is_token_iat_still_valid(claims.sub, claims.iat) {
            return Err(AppError::Unauthorized("Token revoked".to_string()));
        }

        if claims.role != "admin" {
            return Err(AppError::Unauthorized("Admin access required".to_string()));
        }

        Ok(AdminGuard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{DecodingKey, Validation, decode};

    // OnceLock is set once per process, so all tests in this suite share the same secret.
    const TEST_SECRET: &str = "test-secret-for-unit-tests";

    fn test_decoding_key() -> DecodingKey {
        DecodingKey::from_secret(TEST_SECRET.as_bytes())
    }

    fn test_validation() -> Validation {
        let mut v = Validation::new(Algorithm::HS256);
        v.validate_exp = false;
        v.set_audience(&["agent"]);
        v
    }

    #[test]
    fn test_generate_jwt_produces_three_part_token() {
        init_encoding_key(TEST_SECRET);
        let token = generate_jwt().expect("JWT generation failed");
        assert!(!token.is_empty());
        assert_eq!(
            token.split('.').count(),
            3,
            "JWT must be in header.payload.signature format"
        );
    }

    #[test]
    fn test_generated_jwt_is_decodable_with_correct_secret() {
        init_encoding_key(TEST_SECRET);
        let token = generate_jwt().expect("JWT generation failed");
        let result = decode::<Claims>(&token, &test_decoding_key(), &test_validation());
        assert!(
            result.is_ok(),
            "Should be decodable with the correct secret"
        );
    }

    #[test]
    fn test_jwt_signed_with_wrong_secret_fails_validation() {
        // Use encode/decode directly to avoid OnceLock global state — keeps this test isolated.
        use jsonwebtoken::{EncodingKey, Header, encode};
        let token = encode(
            &Header::new(Algorithm::HS256),
            &Claims {
                exp: usize::MAX,
                aud: "agent".to_string(),
            },
            &EncodingKey::from_secret(b"correct-secret"),
        )
        .expect("Token creation failed");

        let mut wrong_validation = test_validation();
        wrong_validation.validate_exp = false;
        let result = decode::<Claims>(
            &token,
            &DecodingKey::from_secret(b"wrong-secret"),
            &wrong_validation,
        );
        assert!(
            result.is_err(),
            "Validation must fail with the wrong secret"
        );
    }

    #[test]
    fn test_generated_jwt_exp_is_in_future() {
        use chrono::Utc;
        init_encoding_key(TEST_SECRET);
        let token = generate_jwt().expect("JWT generation failed");
        let data = decode::<Claims>(&token, &test_decoding_key(), &test_validation())
            .expect("Decoding failed");
        let now = Utc::now().timestamp() as usize;
        assert!(
            data.claims.exp > now,
            "exp must be in the future (token expires ~60 seconds from now)"
        );
    }

    // ── Secret rotation contract ─────────────────
    // Rotating JWT_SECRET (which in practice means restarting the server with
    // a new secret) must invalidate every previously-issued token. The
    // guarantee comes from jsonwebtoken's HMAC signature verification. The
    // equivalent contract tests for the user-JWT path live in
    // `services::user_auth::tests`; they cover both the `aud: "user"` branch
    // and the legacy no-aud fallback. When `check_jwt_query` was removed in
    // favour of single-use SSE tickets, its rotation tests were deleted
    // rather than ported — the `user_auth` tests already cover the same
    // code path (`decode_user_jwt`) and there is no longer a query-parameter
    // JWT acceptance point to exercise.
}
