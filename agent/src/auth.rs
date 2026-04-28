//! JWT-based auth middleware for the `/metrics` endpoint.
//!
//! The agent verifies tokens minted by the server. Both sides share
//! the same `JWT_SECRET`; `DECODING_KEY` is a process-wide `OnceLock`
//! seeded at startup and cannot be rotated without restart.

use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Claims {
    pub exp: usize,
    /// Audience claim for token type separation (server sends "agent")
    #[serde(default)]
    pub aud: String,
}

static DECODING_KEY: OnceLock<DecodingKey> = OnceLock::new();

/// Initialize the decoding key from the raw shared secret.
///
/// Returns `Err` if the key is already initialized — intentional:
/// an in-process rotation path would silently diverge from the
/// server's key and create hard-to-diagnose auth failures.
pub(crate) fn init_decoding_key(secret: &[u8]) -> Result<(), &'static str> {
    DECODING_KEY
        .set(DecodingKey::from_secret(secret))
        .map_err(|_| "DECODING_KEY was already initialized")
}

/// A syntactically-valid HS256 JWT signed with a throwaway secret. Used
/// only by the missing-header timing-equalisation path below — never
/// returned to the caller as a successful auth.
const DUMMY_TOKEN: &str = "eyJhbGciOiJIUzI1NiJ9.eyJleHAiOjAsImF1ZCI6ImFnZW50In0.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

pub(crate) async fn auth_middleware(req: Request, next: Next) -> Result<Response, StatusCode> {
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|val| val.to_str().ok())
        .filter(|s| s.starts_with("Bearer "));

    let Some(key) = DECODING_KEY.get() else {
        tracing::error!("❌ [Auth] DECODING_KEY not initialized — rejecting request");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };
    // Single validation path: `aud="agent"` is mandatory.
    // Legacy no-aud acceptance was removed — every server released since
    // v0.3.0 mints tokens with `aud="agent"`, and keeping the fallback
    // weakened the token-type separation defense (a leaked user JWT could
    // slip through on legacy_validation and hit the agent).
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_audience(&["agent"]);
    // Clock-skew grace: NetSentinel's agent tokens have a 60 s `exp`, so a
    // homelab pair of boxes where one has drifted ~20 s ahead of NTP
    // would reject perfectly valid scrape tokens. 30 s leeway matches
    // the server's `JWT_CLOCK_SKEW_LEEWAY_SECS` and is well under the
    // token lifetime — no practical extension of attacker replay windows.
    validation.leeway = 30;

    // Run a decode pass even when the header is missing or malformed.
    // The previous shape returned 401 immediately on `None`, so
    // "missing header" responses came back microseconds faster than
    // "wrong signature" responses — a tiny but technically-real timing
    // oracle for "is auth even configured?". Decoding `DUMMY_TOKEN`
    // costs the same handful of µs as the real path; the result is
    // discarded so a legitimate parse-success against a bogus token
    // can never grant access.
    let token = auth_header.map(|s| &s[7..]).unwrap_or(DUMMY_TOKEN);
    let result = decode::<Claims>(token, key, &validation);

    if auth_header.is_none() {
        let _ = result;
        return Err(StatusCode::UNAUTHORIZED);
    }

    match result {
        Ok(_) => Ok(next.run(req).await),
        Err(e) => {
            tracing::warn!(err = ?e, "⚠️ [Auth] JWT validation failed");
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};

    fn test_validation() -> Validation {
        let mut v = Validation::new(Algorithm::HS256);
        v.validate_exp = false;
        v.set_audience(&["agent"]);
        v
    }

    #[test]
    fn test_valid_jwt_decodes_successfully() {
        let secret = b"test-agent-secret";
        let token = encode(
            &Header::new(Algorithm::HS256),
            &Claims {
                exp: usize::MAX,
                aud: "agent".to_string(),
            },
            &EncodingKey::from_secret(secret),
        )
        .expect("Token creation failed");
        let result = decode::<Claims>(
            &token,
            &DecodingKey::from_secret(secret),
            &test_validation(),
        );
        assert!(result.is_ok(), "Should succeed with the correct secret");
    }

    #[test]
    fn test_jwt_with_wrong_secret_is_rejected() {
        let token = encode(
            &Header::new(Algorithm::HS256),
            &Claims {
                exp: usize::MAX,
                aud: "agent".to_string(),
            },
            &EncodingKey::from_secret(b"correct-secret"),
        )
        .expect("Token creation failed");
        let result = decode::<Claims>(
            &token,
            &DecodingKey::from_secret(b"wrong-secret"),
            &test_validation(),
        );
        assert!(result.is_err(), "Should fail with the wrong secret");
    }

    #[test]
    fn test_malformed_token_is_rejected() {
        let result = decode::<Claims>(
            "not.a.valid.jwt",
            &DecodingKey::from_secret(b"any"),
            &test_validation(),
        );
        assert!(result.is_err(), "Malformed tokens must be rejected");
    }
}
