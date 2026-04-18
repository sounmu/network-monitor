use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::{self, Stream};
use serde::Deserialize;

use crate::errors::AppError;
use crate::models::app_state::AppState;
use crate::models::sse_payloads::SseBroadcast;
use crate::services::auth;

#[derive(Deserialize)]
pub struct SseQuery {
    /// Single-use opaque ticket issued by `POST /api/auth/sse-ticket`.
    /// Browsers cannot set custom headers on `EventSource`, so the token is
    /// passed here — but it is **not** the long-lived JWT. See
    /// `services::sse_ticket` for the full rationale.
    pub key: Option<String>,
}

struct StreamState {
    initial_events: std::vec::IntoIter<Event>,
    rx: tokio::sync::broadcast::Receiver<SseBroadcast>,
    auth_check: tokio::time::Interval,
    user_id: i32,
    issued_at: usize,
}

/// GET /api/stream — SSE stream endpoint
///
/// Flow:
/// 1. Atomically consume the single-use ticket from the `?key=` query parameter.
/// 2. Immediately send the current status payload for all known hosts (initial state sync).
/// 3. Subscribe to the broadcast channel and stream subsequent metrics/status events in real time.
/// 4. Re-check session revocation periodically so logout/admin revoke/password change
///    terminates long-lived streams rather than only blocking new handshakes.
pub async fn sse_handler(
    Query(params): Query<SseQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let ticket = state
        .sse_ticket_store
        .consume(params.key.as_deref().unwrap_or(""))
        .ok_or_else(|| AppError::Unauthorized("Invalid or expired SSE ticket".to_string()))?;

    let initial_events: Vec<Event> = {
        let lks = state
            .last_known_status
            .read()
            .map_err(|e| AppError::Internal(format!("last_known_status lock: {e}")))?;
        lks.values()
            .filter_map(|payload| {
                serde_json::to_string(payload)
                    .ok()
                    .map(|json| Event::default().event("status").data(json))
            })
            .collect()
    };

    let mut auth_check = tokio::time::interval(Duration::from_secs(15));
    auth_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let stream = stream::unfold(
        StreamState {
            initial_events: initial_events.into_iter(),
            rx: state.sse_tx.subscribe(),
            auth_check,
            user_id: ticket.user_id,
            issued_at: ticket.issued_at,
        },
        |mut stream_state| async move {
            loop {
                if !auth::is_token_iat_still_valid(stream_state.user_id, stream_state.issued_at) {
                    return None;
                }

                if let Some(event) = stream_state.initial_events.next() {
                    return Some((Ok(event), stream_state));
                }

                tokio::select! {
                    _ = stream_state.auth_check.tick() => {
                        if !auth::is_token_iat_still_valid(stream_state.user_id, stream_state.issued_at) {
                            return None;
                        }
                    }
                    result = stream_state.rx.recv() => {
                        match result {
                            Ok(SseBroadcast::Metrics(payload)) => {
                                if let Ok(json) = serde_json::to_string(&payload) {
                                    let event = Event::default().event("metrics").data(json);
                                    return Some((Ok(event), stream_state));
                                }
                            }
                            Ok(SseBroadcast::Status(payload)) => {
                                if let Ok(json) = serde_json::to_string(&payload) {
                                    let event = Event::default().event("status").data(json);
                                    return Some((Ok(event), stream_state));
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                        }
                    }
                }
            }
        },
    );

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
