//! Custom Axum middleware: request-correlation IDs and API rate limiting.
//!
//! **Request ID** — every inbound request gets a unique `request_id` field
//! attached to its tracing span and echoed back via `X-Request-Id`.
//!
//! **API rate limit** — per-IP sliding window that caps the total request
//! rate to any endpoint. Protects against runaway clients or accidental
//! infinite-loop polling. Configured via `API_RATE_LIMIT_MAX` (default 200)
//! and `API_RATE_LIMIT_WINDOW_SECS` (default 60).

use std::net::SocketAddr;
use std::sync::Arc;

use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use tracing::Instrument;

use crate::models::app_state::AppState;

/// Monotonic counter mixed with a millisecond timestamp to produce compact,
/// collision-free, roughly time-ordered request IDs without requiring a
/// CSPRNG (this is not a security-sensitive value).
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn generate_request_id() -> String {
    let count = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    // 48-bit timestamp hex + 16-bit counter → "18f3a2b1c4d-00a3"
    format!("{:x}-{:04x}", ts, count & 0xFFFF)
}

/// Middleware that assigns a unique request ID to every inbound request.
///
/// 1. Generates a compact hex ID (timestamp + counter).
/// 2. Opens a `tracing::info_span!("http", ...)` that wraps the entire
///    handler chain — every `tracing::info!` / `tracing::error!` inside
///    the handler automatically inherits the `request_id`, `method`, and
///    `path` fields.
/// 3. Appends an `X-Request-Id` response header so the caller can quote
///    it in bug reports or support tickets.
pub async fn request_id(request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let request_id = generate_request_id();
    let method = request.method().clone();
    let path = request.uri().path().to_owned();

    let span = tracing::info_span!(
        "http",
        request_id = %request_id,
        method = %method,
        path = %path,
    );

    async move {
        let mut response = next.run(request).await;
        if let Ok(val) = HeaderValue::from_str(&request_id) {
            response.headers_mut().insert("X-Request-Id", val);
        }
        response
    }
    .instrument(span)
    .await
}

/// Public endpoints that are reachable **without** authentication. These
/// share a tighter per-IP bucket than the authenticated SPA path so abusive
/// unauthenticated traffic cannot exhaust the generous `API_RATE_LIMIT_MAX`
/// budget the browser client needs for its SWR + SSE + dashboard polling.
///
/// Note on `/api/auth/refresh`: although the endpoint takes no `Authorization`
/// header, it is *credential-bearing* — the httpOnly refresh cookie is the
/// credential. Counting its requests against the strict 30/min public
/// bucket caused NAT / Cloudflare-tunnel deployments with more than a
/// handful of concurrent dashboards to 429 each other out whenever an
/// access-token-expiry wave hit (all browsers refresh at roughly the
/// same second). It lives in the authenticated 200/min bucket instead;
/// a client without a valid refresh cookie still gets a cheap 401.
fn is_public_path(path: &str) -> bool {
    matches!(
        path,
        "/api/auth/login"
            | "/api/auth/setup"
            | "/api/auth/status"
            | "/api/public/status"
            | "/api/health"
            | "/metrics"
    )
}

/// Per-IP API rate limiter middleware.
///
/// Two independent sliding-window buckets per IP, selected by request path:
///
/// * **Public bucket** (`PUBLIC_API_RATE_LIMIT_MAX`, default 30/min) —
///   unauthenticated endpoints. Kept small so a misbehaving anonymous
///   client cannot drain the authenticated budget. Login uses its own
///   even-tighter `login_rate_limiter` in addition to this check.
/// * **Authenticated bucket** (`API_RATE_LIMIT_MAX`, default 200/min) —
///   everything else. Generous enough that normal browser polling
///   (SWR 5 s + SSE retry) stays well within bounds, but a misconfigured
///   script or accidental infinite loop is caught before it saturates
///   the SQLite writer lock.
///
/// Both return `429 Too Many Requests` + `Retry-After` on overflow.
pub async fn api_rate_limit(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let ip = crate::handlers::auth_handler::extract_client_ip(
        request.headers(),
        &peer_addr,
        state.trusted_proxy_count,
    );
    let limiter = if is_public_path(request.uri().path()) {
        &state.public_api_rate_limiter
    } else {
        &state.api_rate_limiter
    };
    if let Err(retry_after) = limiter.check(&ip) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(
                axum::http::header::RETRY_AFTER,
                HeaderValue::from_str(&retry_after.to_string())
                    .unwrap_or_else(|_| HeaderValue::from_static("60")),
            )],
            "Too many requests",
        )
            .into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_request_id_returns_non_empty() {
        let id = generate_request_id();
        assert!(!id.is_empty(), "request ID should not be empty");
    }

    #[test]
    fn generate_request_id_returns_unique_values() {
        let id1 = generate_request_id();
        let id2 = generate_request_id();
        assert_ne!(id1, id2, "consecutive request IDs should differ");
    }

    #[test]
    fn generate_request_id_contains_separator() {
        let id = generate_request_id();
        assert!(
            id.contains('-'),
            "request ID should contain a dash separator, got: {id}"
        );
    }
}
