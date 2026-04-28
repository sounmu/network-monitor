use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
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

/// RAII guard that decrements the global SSE connection counter when the
/// stream is dropped (client disconnects, server shutdown, handler panics).
///
/// The counter is the single source of truth for `MAX_SSE_CONNECTIONS`.
/// Pairing increment with a `Drop` guard guarantees we never leak a slot
/// — the previous design had no accounting at all, so a slow-loris-style
/// client that opened thousands of `EventSource` connections would pin
/// one `broadcast::Receiver` per stream in perpetuity.
struct SseConnectionGuard {
    counter: Arc<AtomicUsize>,
}

impl Drop for SseConnectionGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

struct StreamState {
    initial_events: std::vec::IntoIter<Event>,
    rx: tokio::sync::broadcast::Receiver<SseBroadcast>,
    auth_check: tokio::time::Interval,
    user_id: i32,
    issued_at: usize,
    state: Arc<AppState>,
    /// Released on stream drop. Kept so `_guard` is not dropped early.
    _guard: SseConnectionGuard,
}

/// GET /api/stream — SSE stream endpoint
///
/// Flow:
/// 1. Atomically consume the single-use ticket from the `?key=` query parameter.
/// 2. Reserve a slot against `MAX_SSE_CONNECTIONS` — refuse with 429 on overflow.
/// 3. Immediately send the current status payload for all known hosts (initial state sync).
/// 4. Subscribe to the broadcast channel and stream subsequent metrics/status events in real time.
/// 5. Re-check session revocation periodically so logout/admin revoke/password change
///    terminates long-lived streams rather than only blocking new handshakes.
/// 6. On `Lagged`, re-snapshot `last_known_status` so the client catches up
///    instead of silently drifting.
pub async fn sse_handler(
    Query(params): Query<SseQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let ticket = state
        .sse_ticket_store
        .consume(params.key.as_deref().unwrap_or(""))
        .ok_or_else(|| AppError::Unauthorized("Invalid or expired SSE ticket".to_string()))?;

    // Reserve a connection slot before we start allocating receivers + buffers.
    // `fetch_add` + post-hoc rollback is a racy "compare-and-increment" that
    // is safe enough for a cap on memory — worst case we overshoot by a
    // handful of concurrent handshakes before rejections start.
    let prev = state.sse_connections.fetch_add(1, Ordering::Relaxed);
    if prev >= state.max_sse_connections {
        state.sse_connections.fetch_sub(1, Ordering::Relaxed);
        tracing::warn!(
            current = prev,
            max = state.max_sse_connections,
            "🚦 [SSE] Connection limit reached — rejecting handshake"
        );
        return Err(AppError::TooManyRequests(
            "Too many concurrent SSE connections".to_string(),
        ));
    }
    let guard = SseConnectionGuard {
        counter: Arc::clone(&state.sse_connections),
    };

    let initial_events = build_initial_events(&state)?;

    let mut auth_check = tokio::time::interval(Duration::from_secs(15));
    auth_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let stream = stream::unfold(
        StreamState {
            initial_events: initial_events.into_iter(),
            rx: state.sse_tx.subscribe(),
            auth_check,
            user_id: ticket.user_id,
            issued_at: ticket.issued_at,
            state: Arc::clone(&state),
            _guard: guard,
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
                                if let Ok(json) = serde_json::to_string(&*payload) {
                                    let event = Event::default().event("metrics").data(json);
                                    return Some((Ok(event), stream_state));
                                }
                            }
                            Ok(SseBroadcast::Status(payload)) => {
                                if let Ok(json) = serde_json::to_string(&*payload) {
                                    let event = Event::default().event("status").data(json);
                                    return Some((Ok(event), stream_state));
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                // Slow consumer: the broadcast ring overflowed. Rather than silently
                                // drop skipped events (the old behaviour), re-seed `initial_events`
                                // with a fresh snapshot of every host status so the client's view
                                // reconverges with truth on its next poll.
                                tracing::warn!(
                                    skipped,
                                    user_id = stream_state.user_id,
                                    "🚦 [SSE] Lagged — resyncing initial state snapshot"
                                );
                                if let Ok(snapshot) = build_initial_events(&stream_state.state) {
                                    stream_state.initial_events = snapshot.into_iter();
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                        }
                    }
                }
            }
        },
    );

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// Serialize the current `last_known_status` map into an `Event` list.
/// Extracted so both the handshake path and the `Lagged` recovery path can
/// reuse the same snapshot logic.
///
/// Drains the map into a `Vec<Arc<HostStatusPayload>>` under the read lock
/// — each `Arc::clone` is an atomic refcount bump, not a deep copy — then
/// releases the lock and performs the O(hosts × payload-size) JSON
/// serialization outside the critical section. This matters on the `Lagged`
/// recovery path: a slow consumer that triggers resync must not stall the
/// scraper's `last_known_status.write()` while `serde_json` walks every host.
fn build_initial_events(state: &AppState) -> Result<Vec<Event>, AppError> {
    let snapshot: Vec<Arc<crate::models::sse_payloads::HostStatusPayload>> = {
        let lks = state
            .last_known_status
            .read()
            .map_err(|e| AppError::Internal(format!("last_known_status lock: {e}")))?;
        lks.values().map(Arc::clone).collect()
    };
    Ok(snapshot
        .into_iter()
        .filter_map(|payload| {
            serde_json::to_string(&*payload)
                .ok()
                .map(|json| Event::default().event("status").data(json))
        })
        .collect())
}
