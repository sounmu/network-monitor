use argon2::password_hash::SaltString;
use argon2::password_hash::rand_core::OsRng;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use jsonwebtoken::{Algorithm, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use super::auth::{DECODING_KEY, ENCODING_KEY};

/// JWT claims for authenticated web users.
/// Distinguished from agent Claims by the presence of `sub` (user ID).
#[derive(Debug, Serialize, Deserialize)]
pub struct UserClaims {
    /// User ID
    pub sub: i32,
    pub username: String,
    pub role: String,
    /// Expiration (Unix timestamp)
    pub exp: usize,
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

/// Generate a user JWT (24-hour expiry)
pub fn generate_user_jwt(
    user_id: i32,
    username: &str,
    role: &str,
) -> Result<String, jsonwebtoken::errors::Error> {
    let exp = chrono::Utc::now().timestamp() as usize + 24 * 60 * 60;
    let claims = UserClaims {
        sub: user_id,
        username: username.to_string(),
        role: role.to_string(),
        exp,
    };
    let key = ENCODING_KEY.get().expect("ENCODING_KEY not initialized");
    encode(&Header::new(Algorithm::HS256), &claims, key)
}

/// Decode and validate a user JWT, returning claims if valid
pub fn decode_user_jwt(token: &str) -> Option<UserClaims> {
    let dk = DECODING_KEY.get()?;
    let validation = Validation::new(Algorithm::HS256);
    decode::<UserClaims>(token, dk, &validation)
        .ok()
        .map(|data| data.claims)
}
