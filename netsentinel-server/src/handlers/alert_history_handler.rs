use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};

use crate::errors::AppError;
use crate::models::app_state::AppState;
use crate::repositories::alert_history_repo::{
    self, AlertHistoryPage, AlertHistoryQuery, AlertHistoryRow,
};
use crate::services::auth::UserGuard;

/// GET /api/alert-history — list alert history with optional type / host_key /
/// time-range filters and pagination metadata.
pub async fn get_alert_history(
    _auth: UserGuard,
    State(state): State<Arc<AppState>>,
    Query(query): Query<AlertHistoryQuery>,
) -> Result<Json<AlertHistoryPage>, AppError> {
    let page = alert_history_repo::get_alert_history_page(&state.db_pool, &query).await?;
    Ok(Json(page))
}

/// GET /api/alerts/active — list alerts whose latest event is still an
/// overload/down (no subsequent recovery).
pub async fn get_active_alerts(
    _auth: UserGuard,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<AlertHistoryRow>>, AppError> {
    let rows = alert_history_repo::get_active_alerts(&state.db_pool).await?;
    Ok(Json(rows))
}
