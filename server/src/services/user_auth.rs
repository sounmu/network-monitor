use argon2::password_hash::SaltString;
use argon2::password_hash::rand_core::OsRng;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use jsonwebtoken::{Algorithm, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use super::auth::{DECODING_KEY, ENCODING_KEY};
use crate::errors::AppError;

/// JWT claims for authenticated web users.
/// Distinguished from agent Claims by the presence of `sub` (user ID).
#[derive(Debug, Serialize, Deserialize)]
pub struct UserClaims {
    /// User ID
    pub sub: i32,
    pub username: String,
    pub role: String,
    /// Issued-at (Unix timestamp) — used for token revocation on password change
    #[serde(default)]
    pub iat: usize,
    /// Expiration (Unix timestamp)
    pub exp: usize,
    /// Audience claim — "user" for user tokens (token type separation)
    #[serde(default)]
    pub aud: String,
}

/// Hash a plaintext password with Argon2id
pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2.hash_password(password.as_bytes(), &salt)?;
    Ok(hash.to_string())
}

/// Verify a plaintext password against a stored Argon2 hash
pub fn verify_password(password: &str, hash: &str) -> bool {
    let parsed = match PasswordHash::new(hash) {
        Ok(h) => h,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Generate a short-lived user access JWT.
///
/// Access tokens live in memory on the client — never in localStorage. They
/// are refreshed silently via `/api/auth/refresh` (rotating httpOnly cookie),
/// so the client-observable session is effectively long while the attack
/// surface of any individual leaked access token is bounded to
/// `ACCESS_TTL_MINUTES`.
pub fn generate_user_jwt(user_id: i32, username: &str, role: &str) -> Result<String, AppError> {
    let now = chrono::Utc::now().timestamp() as usize;
    let ttl_secs = (super::refresh_token::ACCESS_TTL_MINUTES as usize) * 60;
    let claims = UserClaims {
        sub: user_id,
        username: username.to_string(),
        role: role.to_string(),
        iat: now,
        exp: now + ttl_secs,
        aud: "user".to_string(),
    };
    let key = ENCODING_KEY
        .get()
        .ok_or_else(|| AppError::Internal("JWT encoding key not initialized".into()))?;
    encode(&Header::new(Algorithm::HS256), &claims, key)
        .map_err(|e| AppError::Internal(format!("JWT encoding failed: {e}")))
}

/// Decode and validate a user JWT, returning claims if valid.
///
/// Strict audience enforcement: only tokens with `aud: "user"` are accepted.
/// The legacy no-`aud` fallback that used to live here was a privilege-
/// escalation vector — any process holding `JWT_SECRET` (including every
/// agent host) could mint `{sub, username:"admin", role:"admin"}` with the
/// `aud` field omitted and gain admin access. One 401 + re-login is the
/// correct cost; see `docs/review-20260417.md` Top-10 #3.
pub fn decode_user_jwt(token: &str) -> Option<UserClaims> {
    let dk = DECODING_KEY.get()?;
    let mut user_validation = Validation::new(Algorithm::HS256);
    user_validation.set_audience(&["user"]);
    // Small clock-skew grace so a server NTP correction (or a virtualised
    // host whose wall-clock ticks backwards briefly) does not reject
    // legitimate tokens on the next request. jsonwebtoken applies this
    // window to both `exp` (expiration) and `nbf` (not-before) checks.
    user_validation.leeway = JWT_CLOCK_SKEW_LEEWAY_SECS;
    decode::<UserClaims>(token, dk, &user_validation)
        .ok()
        .map(|data| data.claims)
}

/// Clock-skew grace window applied to JWT `exp` / `nbf` validation.
/// 30 s is the standard leeway recommended by RFC 7519 §4.1.4 commentary
/// and matches what downstream services (Cloudflare, Okta, AWS IAM)
/// default to. Any larger is a security smell; any smaller and an NTP
/// correction can start rejecting freshly-refreshed tokens.
pub const JWT_CLOCK_SKEW_LEEWAY_SECS: u64 = 30;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::auth::init_encoding_key;
    use jsonwebtoken::EncodingKey;

    // Must match the TEST_SECRET in services::auth::tests so the shared OnceLock
    // holds a consistent decoding key regardless of test order.
    const TEST_SECRET: &str = "test-secret-for-unit-tests";

    #[test]
    fn test_hash_password_produces_valid_argon2_hash() {
        let hash = hash_password("TestPass123!").expect("hashing should succeed");
        // Argon2 hashes start with $argon2
        assert!(
            hash.starts_with("$argon2"),
            "Hash should be a valid argon2 string, got: {hash}"
        );
        // Should be parseable
        PasswordHash::new(&hash).expect("hash should be parseable by PasswordHash");
    }

    #[test]
    fn test_verify_password_correct() {
        let hash = hash_password("CorrectHorse!1").expect("hashing should succeed");
        assert!(
            verify_password("CorrectHorse!1", &hash),
            "verify_password should return true for the correct password"
        );
    }

    #[test]
    fn test_verify_password_wrong() {
        let hash = hash_password("CorrectHorse!1").expect("hashing should succeed");
        assert!(
            !verify_password("WrongPassword!1", &hash),
            "verify_password should return false for a wrong password"
        );
    }

    #[test]
    fn test_verify_password_invalid_hash() {
        assert!(
            !verify_password("anything", "not-a-valid-hash"),
            "verify_password should return false for an unparseable hash"
        );
    }

    #[test]
    fn test_decode_user_jwt_rejects_token_from_other_secret() {
        init_encoding_key(TEST_SECRET);

        // Mint a user JWT with a different secret than the one the server holds.
        let foreign_key = EncodingKey::from_secret(b"secret-from-a-previous-deployment");
        let now = chrono::Utc::now().timestamp() as usize;
        let claims = UserClaims {
            sub: 1,
            username: "alice".to_string(),
            role: "admin".to_string(),
            iat: now,
            exp: now + 3600,
            aud: "user".to_string(),
        };
        let foreign_token = encode(&Header::new(Algorithm::HS256), &claims, &foreign_key)
            .expect("Token creation failed");

        assert!(
            decode_user_jwt(&foreign_token).is_none(),
            "A user JWT minted with a rotated-away secret must be rejected"
        );
    }

    #[test]
    fn test_decode_user_jwt_rejects_legacy_no_aud_token_from_other_secret() {
        init_encoding_key(TEST_SECRET);

        // Legacy user token (empty aud) signed with an old secret — must still fail.
        let foreign_key = EncodingKey::from_secret(b"secret-from-a-previous-deployment");
        let now = chrono::Utc::now().timestamp() as usize;
        let claims = UserClaims {
            sub: 1,
            username: "alice".to_string(),
            role: "admin".to_string(),
            iat: now,
            exp: now + 3600,
            aud: String::new(),
        };
        let foreign_token = encode(&Header::new(Algorithm::HS256), &claims, &foreign_key)
            .expect("Token creation failed");

        assert!(
            decode_user_jwt(&foreign_token).is_none(),
            "Legacy (no-aud) user tokens from a rotated-away secret must also be rejected"
        );
    }
}
