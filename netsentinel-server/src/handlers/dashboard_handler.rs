use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;

use crate::errors::AppError;
use crate::models::app_state::AppState;
use crate::repositories::dashboard_repo::{self, DashboardLayout};
use crate::services::auth::AuthGuard;
use crate::services::user_auth;

/// Extract user_id from the Authorization header (user JWT only)
fn extract_user_id(headers: &HeaderMap) -> Result<i32, AppError> {
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::Unauthorized("Missing token".to_string()))?;

    let claims = user_auth::decode_user_jwt(token).ok_or_else(|| {
        AppError::BadRequest("Dashboard requires user authentication (not API key)".to_string())
    })?;

    Ok(claims.sub)
}

/// GET /api/dashboard — get current user's dashboard layout
pub async fn get_dashboard(
    _auth: AuthGuard,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = extract_user_id(&headers)?;
    let layout = dashboard_repo::get_layout(&state.db_pool, user_id).await?;
    match layout {
        Some(l) => Ok(Json(l.widgets)),
        None => Ok(Json(serde_json::json!([]))),
    }
}

#[derive(serde::Deserialize)]
pub struct SaveDashboardRequest {
    pub widgets: serde_json::Value,
}

/// PUT /api/dashboard — save current user's dashboard layout
pub async fn save_dashboard(
    _auth: AuthGuard,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<SaveDashboardRequest>,
) -> Result<Json<DashboardLayout>, AppError> {
    let user_id = extract_user_id(&headers)?;
    let layout = dashboard_repo::upsert_layout(&state.db_pool, user_id, &body.widgets).await?;
    Ok(Json(layout))
}
