use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::header::{HeaderValue, SET_COOKIE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::errors::AppError;
use crate::models::app_state::AppState;
use crate::repositories::users_repo::{self, UserInfo};
use crate::services::auth::{AdminGuard, AuthGuard};
use crate::services::refresh_token::{self, REFRESH_TTL_DAYS, RotateOutcome};
use crate::services::user_auth;

/// Cookie name used for the rotating refresh token. Bound to `/api/auth`
/// via the `Path=` attribute so it is never sent to application endpoints —
/// the browser only surfaces it on login/refresh/logout calls.
const REFRESH_COOKIE_NAME: &str = "nm_refresh";

/// Whether the refresh cookie should carry the `Secure` flag.
/// Evaluated once on first call and cached for the process lifetime via
/// `OnceLock`. Derived from `ALLOWED_ORIGINS`: if the variable is set
/// AND every configured origin uses `https://`, we set `Secure`;
/// otherwise (unset, empty, or any `http://` origin) we omit it so the
/// browser stores the cookie over plain HTTP in development.
fn is_secure_cookie() -> bool {
    use std::sync::OnceLock;
    static SECURE: OnceLock<bool> = OnceLock::new();
    *SECURE.get_or_init(|| {
        let raw = std::env::var("ALLOWED_ORIGINS").unwrap_or_default();
        let origins: Vec<&str> = raw
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        !origins.is_empty() && origins.iter().all(|o| o.starts_with("https://"))
    })
}

/// Build a `Set-Cookie` header value that installs a fresh refresh token.
fn build_refresh_cookie(plaintext: &str) -> String {
    let max_age_secs = REFRESH_TTL_DAYS * 24 * 60 * 60;
    let secure = if is_secure_cookie() { "; Secure" } else { "" };
    format!(
        "{name}={value}; HttpOnly; SameSite=Strict; Path=/api/auth; Max-Age={max_age}{secure}",
        name = REFRESH_COOKIE_NAME,
        value = plaintext,
        max_age = max_age_secs
    )
}

/// Build a `Set-Cookie` header value that deletes the refresh cookie.
fn build_refresh_cookie_expiry() -> String {
    let secure = if is_secure_cookie() { "; Secure" } else { "" };
    format!(
        "{name}=; HttpOnly; SameSite=Strict; Path=/api/auth; Max-Age=0{secure}",
        name = REFRESH_COOKIE_NAME,
    )
}

/// Pull the refresh token plaintext out of a `Cookie` request header.
fn extract_refresh_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get("cookie")?.to_str().ok()?;
    for segment in raw.split(';') {
        let trimmed = segment.trim();
        if let Some(rest) = trimmed.strip_prefix(&format!("{REFRESH_COOKIE_NAME}=")) {
            return Some(rest.to_string());
        }
    }
    None
}

/// Read the `User-Agent` request header as an owned `String`, if present.
fn extract_user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// Build a JSON response that also carries a `Set-Cookie` header.
fn json_with_cookie<T: serde::Serialize>(body: &T, cookie: &str) -> Result<Response, AppError> {
    let mut resp = Json(body).into_response();
    let header_value = HeaderValue::from_str(cookie)
        .map_err(|e| AppError::Internal(format!("Invalid Set-Cookie header value: {e}")))?;
    resp.headers_mut().append(SET_COOKIE, header_value);
    Ok(resp)
}

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

/// Extract the client IP address, accounting for trusted reverse proxies.
///
/// When `trusted_proxy_count == 0`, ignores X-Forwarded-For (prevents spoofing)
/// and uses the peer socket address. When `> 0`, takes the Nth IP from the
/// **right** of X-Forwarded-For (proxies append left-to-right, so the rightmost
/// entries are from infrastructure the operator controls).
fn extract_client_ip(
    headers: &HeaderMap,
    peer_addr: &SocketAddr,
    trusted_proxy_count: usize,
) -> String {
    if trusted_proxy_count == 0 {
        return peer_addr.ip().to_string();
    }
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let ips: Vec<&str> = xff.split(',').map(|s| s.trim()).collect();
        if ips.len() >= trusted_proxy_count {
            return ips[ips.len() - trusted_proxy_count].to_string();
        }
    }
    peer_addr.ip().to_string()
}

/// POST /api/auth/login — authenticate and install a fresh refresh cookie.
///
/// Returns a short-lived access token in the JSON body (consumed in memory
/// by the browser) AND installs a `Set-Cookie` header with a rotating
/// refresh token (HttpOnly/Secure/SameSite=Strict/Path=/api/auth). The two
/// tokens live separately by design — the access token cannot be read by
/// JavaScript if an attacker only has XSS, and the refresh cookie cannot
/// be exfiltrated because it is not reachable from script.
pub async fn login(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Result<Response, AppError> {
    // Rate limit by client IP (secure extraction, immune to X-Forwarded-For spoofing)
    let ip_str = extract_client_ip(&headers, &peer_addr, state.trusted_proxy_count);

    if let Err(retry_after) = state.login_rate_limiter.check(&ip_str) {
        tracing::warn!(ip = %ip_str, "🔒 [Auth] Login rate limited");
        return Err(AppError::BadRequest(format!(
            "Too many login attempts. Try again in {retry_after} seconds."
        )));
    }

    let user = users_repo::find_by_username(&state.db_pool, &body.username)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Invalid username or password".to_string()))?;

    if !user_auth::verify_password(&body.password, &user.password_hash) {
        return Err(AppError::Unauthorized(
            "Invalid username or password".to_string(),
        ));
    }

    let access_token = user_auth::generate_user_jwt(user.id, &user.username, &user.role)?;
    let user_agent = extract_user_agent(&headers);
    let refresh = refresh_token::issue_new_family(
        &state.db_pool,
        user.id,
        user_agent.as_deref(),
        Some(&ip_str),
    )
    .await?;

    tracing::info!(username = %user.username, "🔐 [Auth] User logged in");

    let body = LoginResponse {
        token: access_token,
        user: user.into(),
    };
    json_with_cookie(&body, &build_refresh_cookie(&refresh.plaintext))
}

/// POST /api/auth/refresh — rotate the refresh cookie and hand out a new access token.
///
/// Requires no bearer header — the caller proves session continuity via
/// the httpOnly refresh cookie. On success, a new cookie replaces the old
/// one and a fresh access JWT is returned. On failure the response is
/// 401 with an explicit cookie-deletion header so the client releases any
/// stale state.
pub async fn refresh(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
) -> Result<Response, AppError> {
    let presented = extract_refresh_cookie(&headers)
        .ok_or_else(|| AppError::Unauthorized("No refresh cookie".to_string()))?;
    let user_agent = extract_user_agent(&headers);
    let ip_str = extract_client_ip(&headers, &peer_addr, state.trusted_proxy_count);

    match refresh_token::rotate(
        &state.db_pool,
        &presented,
        user_agent.as_deref(),
        Some(&ip_str),
    )
    .await?
    {
        RotateOutcome::Rotated(new) => {
            // Look up user info for the fresh access token claims.
            let user = users_repo::find_by_id(&state.db_pool, new.user_id)
                .await?
                .ok_or_else(|| AppError::Unauthorized("User no longer exists".to_string()))?;
            let access_token = user_auth::generate_user_jwt(user.id, &user.username, &user.role)?;
            let body = LoginResponse {
                token: access_token,
                user: user.into(),
            };
            json_with_cookie(&body, &build_refresh_cookie(&new.plaintext))
        }
        RotateOutcome::ReuseDetected { user_id } => {
            // Already handled inside rotate(): family revoked, cutoff raised.
            // Tell the browser to drop its (now-worthless) cookie.
            tracing::warn!(
                user_id,
                "🚨 [Auth] /refresh returning 401 after reuse detection"
            );
            let mut resp = (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Session terminated — please sign in again"
                })),
            )
                .into_response();
            resp.headers_mut().append(
                SET_COOKIE,
                HeaderValue::from_str(&build_refresh_cookie_expiry())
                    .map_err(|e| AppError::Internal(format!("header build: {e}")))?,
            );
            Ok(resp)
        }
        RotateOutcome::Rejected => {
            let mut resp = (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Invalid or expired refresh token" })),
            )
                .into_response();
            resp.headers_mut().append(
                SET_COOKIE,
                HeaderValue::from_str(&build_refresh_cookie_expiry())
                    .map_err(|e| AppError::Internal(format!("header build: {e}")))?,
            );
            Ok(resp)
        }
    }
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
///
/// Parallel to `login`: returns an access JWT in the body AND installs a
/// refresh cookie, so the first admin is immediately logged in with the
/// full rotating-session contract active.
pub async fn setup(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    Json(body): Json<SetupRequest>,
) -> Result<Response, AppError> {
    let count = users_repo::count_users(&state.db_pool).await?;
    if count > 0 {
        return Err(AppError::BadRequest(
            "Setup already completed. Use login instead.".to_string(),
        ));
    }

    if body.username.is_empty() {
        return Err(AppError::BadRequest("Username is required".to_string()));
    }
    validate_password(&body.password)?;

    let password_hash = user_auth::hash_password(&body.password)
        .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))?;

    let user = users_repo::create_user(&state.db_pool, &body.username, &password_hash, "admin")
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create user: {}", e)))?;

    let access_token = user_auth::generate_user_jwt(user.id, &user.username, &user.role)?;
    let user_agent = extract_user_agent(&headers);
    let ip_str = extract_client_ip(&headers, &peer_addr, state.trusted_proxy_count);
    let refresh = refresh_token::issue_new_family(
        &state.db_pool,
        user.id,
        user_agent.as_deref(),
        Some(&ip_str),
    )
    .await?;

    tracing::info!(username = %user.username, "🔐 [Auth] Initial admin account created");

    let body = LoginResponse {
        token: access_token,
        user: user.into(),
    };
    json_with_cookie(&body, &build_refresh_cookie(&refresh.plaintext))
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

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// PUT /api/auth/password — change current user's password (admin only)
pub async fn change_password(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    validate_password(&body.new_password)?;

    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::Unauthorized("Missing token".to_string()))?;

    let claims = user_auth::decode_user_jwt(token)
        .ok_or_else(|| AppError::Unauthorized("Invalid token".to_string()))?;

    let user = users_repo::find_by_username(&state.db_pool, &claims.username)
        .await?
        .ok_or_else(|| AppError::Unauthorized("User not found".to_string()))?;

    if !user_auth::verify_password(&body.current_password, &user.password_hash) {
        return Err(AppError::Unauthorized(
            "Current password is incorrect".to_string(),
        ));
    }

    let new_hash = user_auth::hash_password(&body.new_password)
        .map_err(|e| AppError::Internal(format!("Failed to hash password: {e}")))?;

    users_repo::update_password(&state.db_pool, user.id, &new_hash).await?;

    // Invalidate all existing tokens by raising the JWT cutoff and burning
    // every live refresh row for this user. Both matter: the access JWTs
    // die on the cutoff check, and active refresh cookies stop being able
    // to mint replacements.
    let now = chrono::Utc::now().timestamp();
    crate::services::auth::update_password_changed_at(user.id, now);
    if let Err(e) = refresh_token::revoke_all_for_user(&state.db_pool, user.id).await {
        tracing::warn!(err = ?e, user_id = user.id, "⚠️ [Auth] Failed to revoke refresh rows on password change");
    }

    tracing::info!(username = %user.username, "🔐 [Auth] Password changed — existing tokens revoked");
    Ok(Json(serde_json::json!({ "success": true })))
}

/// POST /api/auth/logout — revoke every token for the caller and clear the cookie.
///
/// Three side effects:
///   1. Stamp `users.tokens_revoked_at` → in-memory cutoff raised → every
///      existing access JWT for this user is rejected immediately.
///   2. Mark every live row in `refresh_tokens` for this user as revoked.
///   3. Reply with a `Set-Cookie` that deletes `nm_refresh` from the
///      browser.
///
/// Idempotent and tolerant of a missing / invalid access token: even a
/// best-effort logout from a stale session still results in the cookie
/// being cleaned up.
pub async fn logout(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // Try to identify the caller from the bearer token. If that fails,
    // we still wipe the cookie so the client walks away cleanly.
    let user_id_opt = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .and_then(user_auth::decode_user_jwt)
        .map(|claims| (claims.sub, claims.username));

    if let Some((user_id, username)) = user_id_opt {
        users_repo::revoke_user_tokens(&state.db_pool, user_id).await?;
        let now = chrono::Utc::now().timestamp();
        crate::services::auth::update_tokens_revoked_at(user_id, now);
        if let Err(e) = refresh_token::revoke_all_for_user(&state.db_pool, user_id).await {
            tracing::warn!(err = ?e, user_id, "⚠️ [Auth] Failed to revoke refresh rows on logout");
        }
        tracing::info!(
            user_id,
            username = %username,
            "🔐 [Auth] User logged out — all tokens revoked"
        );
    } else if let Some(cookie) = extract_refresh_cookie(&headers) {
        // No usable bearer, but we still have a refresh cookie — revoke
        // just that single refresh row. This handles the "access JWT
        // already expired; browser clicking logout" case cleanly.
        if let Err(e) = refresh_token::revoke_single(&state.db_pool, &cookie).await {
            tracing::warn!(err = ?e, "⚠️ [Auth] Failed to revoke single refresh on logout");
        }
    }

    let mut resp = Json(serde_json::json!({ "success": true })).into_response();
    resp.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&build_refresh_cookie_expiry())
            .map_err(|e| AppError::Internal(format!("header build: {e}")))?,
    );
    Ok(resp)
}

/// POST /api/admin/users/{id}/revoke-sessions — operator kill-switch.
///
/// Lets an administrator terminate every active session for a target user
/// without needing their password. Use cases: stolen laptop, offboarded
/// employee, incident response. The admin's own session is unaffected
/// unless they pass their own user id.
pub async fn admin_revoke_user_sessions(
    _admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    axum::extract::Path(user_id): axum::extract::Path<i32>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Confirm the target user exists so callers get a 404 instead of a
    // silent success against a non-existent id.
    if users_repo::find_by_id(&state.db_pool, user_id)
        .await?
        .is_none()
    {
        return Err(AppError::NotFound(format!("User {user_id} not found")));
    }

    users_repo::revoke_user_tokens(&state.db_pool, user_id).await?;
    let now = chrono::Utc::now().timestamp();
    crate::services::auth::update_tokens_revoked_at(user_id, now);
    if let Err(e) = refresh_token::revoke_all_for_user(&state.db_pool, user_id).await {
        tracing::warn!(err = ?e, target_user_id = user_id, "⚠️ [Auth] Failed to revoke refresh rows on admin kill");
    }

    tracing::warn!(
        target_user_id = user_id,
        "🔐 [Auth] Admin force-revoked all sessions for user"
    );
    Ok(Json(
        serde_json::json!({ "success": true, "user_id": user_id }),
    ))
}

#[derive(serde::Serialize)]
pub struct SseTicketResponse {
    pub ticket: String,
    /// TTL hint for the client so it can pre-refresh without probing the server.
    /// Kept in sync with `services::sse_ticket::TICKET_TTL`.
    pub expires_in_secs: u64,
}

/// POST /api/auth/sse-ticket — mint a short-lived single-use ticket for `GET /api/stream`.
///
/// Requires a valid user JWT on the `Authorization` header. The returned ticket is
/// bound to the caller's `user_id` and is consumed atomically on the SSE handshake.
/// See `services::sse_ticket` for rationale.
pub async fn issue_sse_ticket(
    _auth: AuthGuard,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<SseTicketResponse>, AppError> {
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::Unauthorized("Missing token".to_string()))?;

    let claims = user_auth::decode_user_jwt(token)
        .ok_or_else(|| AppError::Unauthorized("Invalid user token".to_string()))?;

    let ticket = state.sse_ticket_store.issue(claims.sub);

    Ok(Json(SseTicketResponse {
        ticket,
        expires_in_secs: 60,
    }))
}

/// Validate password strength: min 8 chars, uppercase, lowercase, digit, special char.
fn validate_password(password: &str) -> Result<(), AppError> {
    if password.len() < 8 {
        return Err(AppError::BadRequest(
            "Password must be at least 8 characters".to_string(),
        ));
    }
    let has_upper = password.chars().any(|c| c.is_uppercase());
    let has_lower = password.chars().any(|c| c.is_lowercase());
    let has_digit = password.chars().any(|c| c.is_ascii_digit());
    let has_special = password.chars().any(|c| !c.is_alphanumeric());
    if !has_upper || !has_lower || !has_digit || !has_special {
        return Err(AppError::BadRequest(
            "Password must contain uppercase, lowercase, digit, and special character".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_password ──

    #[test]
    fn validate_password_rejects_short() {
        let result = validate_password("Aa1!xyz");
        assert!(result.is_err(), "7-char password should be rejected");
    }

    #[test]
    fn validate_password_rejects_no_uppercase() {
        let result = validate_password("abcdefg1!");
        assert!(
            result.is_err(),
            "password without uppercase should be rejected"
        );
    }

    #[test]
    fn validate_password_rejects_no_lowercase() {
        let result = validate_password("ABCDEFG1!");
        assert!(
            result.is_err(),
            "password without lowercase should be rejected"
        );
    }

    #[test]
    fn validate_password_rejects_no_digit() {
        let result = validate_password("Abcdefgh!");
        assert!(result.is_err(), "password without digit should be rejected");
    }

    #[test]
    fn validate_password_rejects_no_special() {
        let result = validate_password("Abcdefg1");
        assert!(
            result.is_err(),
            "password without special char should be rejected"
        );
    }

    #[test]
    fn validate_password_accepts_valid() {
        let result = validate_password("StrongP@ss1");
        assert!(result.is_ok(), "valid complex password should be accepted");
    }

    // ── build_refresh_cookie ──

    #[test]
    fn build_refresh_cookie_contains_expected_attributes() {
        let cookie = build_refresh_cookie("tok_abc123");
        assert!(
            cookie.contains("nm_refresh=tok_abc123"),
            "cookie should contain name=value"
        );
        assert!(cookie.contains("HttpOnly"), "cookie should be HttpOnly");
        assert!(
            cookie.contains("SameSite=Strict"),
            "cookie should be SameSite=Strict"
        );
        assert!(
            cookie.contains("Path=/api/auth"),
            "cookie should be scoped to /api/auth"
        );
        assert!(cookie.contains("Max-Age="), "cookie should have Max-Age");
        // Verify Max-Age is REFRESH_TTL_DAYS in seconds
        let expected_max_age = REFRESH_TTL_DAYS * 24 * 60 * 60;
        assert!(
            cookie.contains(&format!("Max-Age={expected_max_age}")),
            "Max-Age should be {expected_max_age}, got: {cookie}"
        );
    }

    // ── build_refresh_cookie_expiry ──

    #[test]
    fn build_refresh_cookie_expiry_sets_max_age_zero() {
        let cookie = build_refresh_cookie_expiry();
        assert!(
            cookie.contains("Max-Age=0"),
            "expiry cookie should have Max-Age=0, got: {cookie}"
        );
        assert!(
            cookie.contains("nm_refresh=;")
                || cookie.contains("nm_refresh= ;")
                || cookie.contains("nm_refresh="),
            "expiry cookie should clear the value"
        );
    }

    // ── extract_refresh_cookie ──

    #[test]
    fn extract_refresh_cookie_parses_single_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert("cookie", HeaderValue::from_static("nm_refresh=abc123"));
        let result = extract_refresh_cookie(&headers);
        assert_eq!(result, Some("abc123".to_string()));
    }

    #[test]
    fn extract_refresh_cookie_parses_among_multiple() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "cookie",
            HeaderValue::from_static("other=foo; nm_refresh=mytoken; session=bar"),
        );
        let result = extract_refresh_cookie(&headers);
        assert_eq!(result, Some("mytoken".to_string()));
    }

    #[test]
    fn extract_refresh_cookie_returns_none_when_missing() {
        let mut headers = HeaderMap::new();
        headers.insert("cookie", HeaderValue::from_static("other=foo; session=bar"));
        let result = extract_refresh_cookie(&headers);
        assert!(result.is_none(), "should return None when cookie is absent");
    }

    #[test]
    fn extract_refresh_cookie_returns_none_when_no_cookie_header() {
        let headers = HeaderMap::new();
        let result = extract_refresh_cookie(&headers);
        assert!(result.is_none(), "should return None with no cookie header");
    }
}
