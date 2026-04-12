use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::{self, Stream, StreamExt};
use serde::Deserialize;
use tokio_stream::wrappers::BroadcastStream;

use crate::models::app_state::AppState;
use crate::models::sse_payloads::SseBroadcast;

#[derive(Deserialize)]
pub struct SseQuery {
    /// Single-use opaque ticket issued by `POST /api/auth/sse-ticket`.
    /// Browsers cannot set custom headers on `EventSource`, so the token is
    /// passed here — but it is **not** the long-lived JWT. See
    /// `services::sse_ticket` for the full rationale.
    pub key: Option<String>,
}

/// GET /api/stream — SSE stream endpoint
///
/// Flow:
/// 1. Atomically consume the single-use ticket from the `?key=` query parameter.
/// 2. Immediately send the current status payload for all known hosts (initial state sync).
/// 3. Subscribe to the broadcast channel and stream subsequent metrics/status events in real time.
pub async fn sse_handler(
    Query(params): Query<SseQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    // Consume the single-use ticket. A missing, unknown, expired, or
    // already-consumed ticket is indistinguishable from the client's side.
    let _user_id = state
        .sse_ticket_store
        .consume(params.key.as_deref().unwrap_or(""))
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // ── Collect initial status snapshot ──
    // When a new client connects, immediately send the current state for all known hosts.
    let initial_events: Vec<Result<Event, Infallible>> = {
        let lks = state
            .last_known_status
            .read()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        lks.values()
            .filter_map(|payload| {
                serde_json::to_string(payload)
                    .ok()
                    .map(|json| Ok(Event::default().event("status").data(json)))
            })
            .collect()
    };

    // ── Subscribe to the broadcast channel ──
    // subscribe() receives only events sent after this point.
    let rx = state.sse_tx.subscribe();
    let broadcast_stream = BroadcastStream::new(rx).filter_map(|result| async move {
        match result {
            Ok(SseBroadcast::Metrics(payload)) => serde_json::to_string(&payload)
                .ok()
                .map(|json| Ok(Event::default().event("metrics").data(json))),
            Ok(SseBroadcast::Status(payload)) => serde_json::to_string(&payload)
                .ok()
                .map(|json| Ok(Event::default().event("status").data(json))),
            // Lagged: client fell behind the broadcast buffer — skip the missed events.
            Err(_) => None,
        }
    });

    // Prepend the initial snapshot, then switch to the live broadcast stream.
    let stream = stream::iter(initial_events).chain(broadcast_stream);

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
