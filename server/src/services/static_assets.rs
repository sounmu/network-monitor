use std::path::Path;

use axum::Json;
use axum::Router;
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use tower_http::services::{ServeDir, ServeFile};

/// Mount the web static export bundle onto the given router.
///
/// The web tier used to run as a separate Next.js `output: 'standalone'`
/// Node.js server on port 3001. From v0.3.6 it is built via
/// `output: 'export'` (plain HTML + JS) and served directly by Axum so
/// the homelab deployment collapses to a single container without the
/// ~35 MB Node.js runtime.
///
/// Expected layout under `dir`:
///   - `index.html`, `agents/index.html`, `alerts/index.html`, …
///   - `host/index.html` — the detail page. The actual `host_key` is
///     passed as a `?key=<value>` query parameter (a URL-native fit for
///     runtime data that `output: 'export'` can't bake into the route).
///   - `404.html` — generic not-found page for unmatched paths.
///
/// After mounting, anything not already claimed by the API is served
/// from `dir`, falling back to `404.html` when no file matches.
///
/// **API-404 contract**: unmatched `/api/*` or `/metrics/*` requests never
/// fall through to the HTML `404.html` — they receive a JSON 404 so that
/// programmatic clients (the SPA, Prometheus, etc.) can parse the response
/// without choking on `<!DOCTYPE html>`. Axum's routing prefers the more
/// specific existing handlers (`/api/auth/login`, `/metrics`, …), so the
/// catch-all below only fires when no concrete handler matched.
pub fn mount(router: Router, dir: &Path) -> Router {
    let not_found = ServeFile::new(dir.join("404.html"));
    let general = ServeDir::new(dir).fallback(not_found);

    router
        .route("/api", any(api_not_found))
        .route("/api/{*rest}", any(api_not_found))
        .route("/metrics/{*rest}", any(api_not_found))
        .fallback_service(general)
}

async fn api_not_found(uri: Uri) -> Response {
    let body = serde_json::json!({
        "error": format!("API endpoint not found: {}", uri.path()),
    });
    (StatusCode::NOT_FOUND, Json(body)).into_response()
}
