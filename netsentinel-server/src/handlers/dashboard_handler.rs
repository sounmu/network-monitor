use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::errors::AppError;
use crate::models::app_state::AppState;
use crate::repositories::dashboard_repo::{self, DashboardLayout};
use crate::services::auth::UserGuard;

/// GET /api/dashboard — get current user's dashboard layout
pub async fn get_dashboard(
    auth: UserGuard,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let layout = dashboard_repo::get_layout(&state.db_pool, auth.claims.sub).await?;
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
///
/// Validates that the widget payload is not excessively large (max 64 KB).
pub async fn save_dashboard(
    auth: UserGuard,
    State(state): State<Arc<AppState>>,
    Json(body): Json<SaveDashboardRequest>,
) -> Result<Json<DashboardLayout>, AppError> {
    // SS-11: Validate widget payload size to prevent abuse
    let serialized = serde_json::to_string(&body.widgets)
        .map_err(|e| AppError::BadRequest(format!("Invalid widgets JSON: {e}")))?;
    if serialized.len() > 64 * 1024 {
        return Err(AppError::BadRequest(
            "Dashboard layout too large (max 64 KB)".to_string(),
        ));
    }

    let layout =
        dashboard_repo::upsert_layout(&state.db_pool, auth.claims.sub, &body.widgets).await?;
    Ok(Json(layout))
}
