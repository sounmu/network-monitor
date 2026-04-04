use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::errors::AppError;
use chrono::Utc;
use chrono_tz::Asia::Seoul;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub exp: usize,
}

pub static ENCODING_KEY: OnceLock<EncodingKey> = OnceLock::new();
pub static DECODING_KEY: OnceLock<DecodingKey> = OnceLock::new();

pub fn init_encoding_key(secret: &str) {
    let key = EncodingKey::from_secret(secret.as_bytes());
    let _ = ENCODING_KEY.set(key);
    let dk = DecodingKey::from_secret(secret.as_bytes());
    let _ = DECODING_KEY.set(dk);
}

/// Validate a JWT token passed as a query parameter (for SSE — EventSource cannot set headers).
/// Accepts both agent JWTs (Claims) and user JWTs (UserClaims).
pub fn check_jwt_query(token: &str) -> bool {
    let Some(dk) = DECODING_KEY.get() else {
        return false;
    };
    let validation = Validation::new(Algorithm::HS256);
    decode::<Claims>(token, dk, &validation).is_ok()
        || super::user_auth::decode_user_jwt(token).is_some()
}

pub fn generate_jwt() -> Result<String, jsonwebtoken::errors::Error> {
    let exp = Utc::now().with_timezone(&Seoul).timestamp() as usize + 60;
    let claims = Claims { exp };
    let key = ENCODING_KEY.get().expect("ENCODING_KEY not initialized");
    encode(&Header::new(Algorithm::HS256), &claims, key)
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

        // Try agent JWT first, then user JWT
        let decoding_key = DECODING_KEY
            .get()
            .ok_or_else(|| AppError::Internal("DECODING_KEY not initialized".to_string()))?;
        let validation = Validation::new(Algorithm::HS256);

        if decode::<Claims>(token, decoding_key, &validation).is_ok() {
            return Ok(AuthGuard);
        }

        // User JWT (different claims structure)
        if super::user_auth::decode_user_jwt(token).is_some() {
            return Ok(AuthGuard);
        }

        Err(AppError::Unauthorized(
            "Invalid or expired token".to_string(),
        ))
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
            &Claims { exp: usize::MAX },
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
}
