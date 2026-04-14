use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};

use crate::errors::AppError;
use crate::models::app_state::AppState;
use crate::repositories::alert_history_repo::{self, AlertHistoryQuery, AlertHistoryRow};
use crate::services::auth::AuthGuard;

/// GET /api/alert-history?host_key=&limit=&offset= — list alert history
pub async fn get_alert_history(
    _auth: AuthGuard,
    State(state): State<Arc<AppState>>,
    Query(query): Query<AlertHistoryQuery>,
) -> Result<Json<Vec<AlertHistoryRow>>, AppError> {
    let rows = alert_history_repo::get_alert_history(&state.db_pool, &query).await?;
    Ok(Json(rows))
}
