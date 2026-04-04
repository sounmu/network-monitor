use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use serde::Deserialize;

use crate::errors::AppError;
use crate::models::app_state::AppState;
use crate::repositories::users_repo::{self, UserInfo};
use crate::services::auth::AuthGuard;
use crate::services::user_auth;

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(serde::Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: UserInfo,
}

/// POST /api/auth/login — authenticate with username/password, returns JWT
pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let user = users_repo::find_by_username(&state.db_pool, &body.username)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Invalid username or password".to_string()))?;

    if !user_auth::verify_password(&body.password, &user.password_hash) {
        return Err(AppError::Unauthorized(
            "Invalid username or password".to_string(),
        ));
    }

    let token = user_auth::generate_user_jwt(user.id, &user.username, &user.role)
        .map_err(|e| AppError::Internal(format!("Failed to generate token: {}", e)))?;

    tracing::info!(username = %user.username, "🔐 [Auth] User logged in");

    Ok(Json(LoginResponse {
        token,
        user: user.into(),
    }))
}

/// GET /api/auth/me — return current user info from JWT
pub async fn me(
    _auth: AuthGuard,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<UserInfo>, AppError> {
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::Unauthorized("Missing token".to_string()))?;

    let claims = user_auth::decode_user_jwt(token)
        .ok_or_else(|| AppError::Unauthorized("Invalid user token".to_string()))?;

    let user = users_repo::find_by_username(&state.db_pool, &claims.username)
        .await?
        .ok_or_else(|| AppError::Unauthorized("User no longer exists".to_string()))?;

    Ok(Json(user.into()))
}

#[derive(Deserialize)]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
}

/// POST /api/auth/setup — create initial admin account (only when no users exist)
pub async fn setup(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SetupRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let count = users_repo::count_users(&state.db_pool).await?;
    if count > 0 {
        return Err(AppError::BadRequest(
            "Setup already completed. Use login instead.".to_string(),
        ));
    }

    if body.username.is_empty() || body.password.len() < 6 {
        return Err(AppError::BadRequest(
            "Username is required and password must be at least 6 characters".to_string(),
        ));
    }

    let password_hash = user_auth::hash_password(&body.password)
        .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))?;

    let user = users_repo::create_user(&state.db_pool, &body.username, &password_hash, "admin")
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create user: {}", e)))?;

    let token = user_auth::generate_user_jwt(user.id, &user.username, &user.role)
        .map_err(|e| AppError::Internal(format!("Failed to generate token: {}", e)))?;

    tracing::info!(username = %user.username, "🔐 [Auth] Initial admin account created");

    Ok(Json(LoginResponse {
        token,
        user: user.into(),
    }))
}

/// GET /api/auth/status — check if setup is needed (no auth required)
pub async fn auth_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let count = users_repo::count_users(&state.db_pool).await?;
    Ok(Json(serde_json::json!({
        "setup_required": count == 0,
    })))
}
