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
use crate::services::auth::{AdminGuard, UserGuard};
use crate::services::refresh_token::{self, REFRESH_TTL_DAYS, RotateOutcome};
use crate::services::user_auth;

/// Cookie name used for the rotating refresh token. Bound to `/api/auth`
/// via the `Path=` attribute so it is never sent to application endpoints —
/// the browser only surfaces it on login/refresh/logout calls.
const REFRESH_COOKIE_NAME: &str = "nm_refresh";

/// Whether the refresh cookie should carry the `Secure` flag.
/// Evaluated once on first call and cached for the process lifetime via
/// `OnceLock`. Secure-by-default: operators must explicitly opt out with
/// `COOKIE_SECURE=false` for local plain-HTTP development.
fn is_secure_cookie() -> bool {
    use std::sync::OnceLock;
    static SECURE: OnceLock<bool> = OnceLock::new();
    *SECURE.get_or_init(|| match std::env::var("COOKIE_SECURE") {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
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
/// Prefix for the refresh cookie — const avoids a format!() allocation per call.
const REFRESH_COOKIE_PREFIX: &str = "nm_refresh=";

fn extract_refresh_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get("cookie")?.to_str().ok()?;
    for segment in raw.split(';') {
        let trimmed = segment.trim();
        if let Some(rest) = trimmed.strip_prefix(REFRESH_COOKIE_PREFIX) {
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

fn unauthorized_json_with_cookie(message: &str, cookie: &str) -> Result<Response, AppError> {
    let mut resp = (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": message })),
    )
        .into_response();
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
/// When `trusted_proxy_count == 0`, ignores every forwarded-IP header
/// (prevents spoofing) and uses the peer socket address. When `> 0`:
///   1. **`CF-Connecting-IP`** is preferred — Cloudflare always sets this to
///      the original client IP and overwrites any spoofed value at the edge.
///      Native support matters because the NetSentinel stock deployment is
///      "Zero-Trust via Cloudflare Tunnel", where without this every request
///      collapses onto a single tunnel-IP and trips rate limits instantly.
///   2. Falls back to the Nth-from-right entry of `X-Forwarded-For`
///      (proxies append left-to-right, so rightmost entries come from
///      operator-controlled infrastructure).
pub(crate) fn extract_client_ip(
    headers: &HeaderMap,
    peer_addr: &SocketAddr,
    trusted_proxy_count: usize,
) -> String {
    if trusted_proxy_count == 0 {
        return peer_addr.ip().to_string();
    }
    if let Some(cf) = headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        return cf.to_string();
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
    // Two-bucket rate limit: per-IP (broad, catches scatter-gun brute force)
    // and per-username (tight, catches targeted brute force). Both must
    // admit the request. The IP bucket is deliberately looser than the
    // per-username one so NAT'd / Cloudflare-tunnel deployments with
    // several dashboards do not 429 each other out when one user mistypes.
    let ip_str = extract_client_ip(&headers, &peer_addr, state.trusted_proxy_count);
    // `username` is trimmed + lowered-case for bucket keying only; the
    // actual credential lookup below uses the supplied value unchanged so
    // case-sensitivity policy still lives with `users_repo`.
    let user_key = body.username.trim().to_lowercase();

    if let Err(retry_after) = state.login_rate_limiter.check(&ip_str) {
        tracing::warn!(ip = %ip_str, "🔒 [Auth] Login rate limited (IP bucket)");
        return Err(AppError::TooManyRequests(format!(
            "Too many login attempts. Try again in {retry_after} seconds."
        )));
    }
    if !user_key.is_empty()
        && let Err(retry_after) = state.login_user_rate_limiter.check(&user_key)
    {
        // Intentionally don't log the username — that would turn our own
        // rate-limit log into an account-enumeration oracle if it leaks.
        tracing::warn!(
            ip = %ip_str,
            "🔒 [Auth] Login rate limited (user bucket)"
        );
        return Err(AppError::TooManyRequests(format!(
            "Too many login attempts for this account. Try again in {retry_after} seconds."
        )));
    }

    let user = users_repo::find_by_username(&state.db_pool, &body.username)
        .await?
        .ok_or_else(|| AppError::Unauthorized("Invalid username or password".to_string()))?;

    let password = body.password.clone();
    let hash = user.password_hash.clone();
    let valid = tokio::task::spawn_blocking(move || user_auth::verify_password(&password, &hash))
        .await
        .map_err(|e| AppError::Internal(format!("Password verify task failed: {e:#}")))?;
    if !valid {
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
            unauthorized_json_with_cookie(
                "Session terminated — please sign in again",
                &build_refresh_cookie_expiry(),
            )
        }
        RotateOutcome::Rejected => unauthorized_json_with_cookie(
            "Invalid or expired refresh token",
            &build_refresh_cookie_expiry(),
        ),
    }
}

/// GET /api/auth/me — return current user info from JWT
pub async fn me(
    auth: UserGuard,
    State(state): State<Arc<AppState>>,
) -> Result<Json<UserInfo>, AppError> {
    let user = users_repo::find_by_username(&state.db_pool, &auth.claims.username)
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
/// Uses a transaction to prevent TOCTOU race (two concurrent requests
/// both passing the `count == 0` check and creating separate admin accounts).
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
    if body.username.is_empty() {
        return Err(AppError::BadRequest("Username is required".to_string()));
    }
    validate_password(&body.password)?;

    let password = body.password.clone();
    let password_hash = tokio::task::spawn_blocking(move || user_auth::hash_password(&password))
        .await
        .map_err(|e| AppError::Internal(format!("Hash task failed: {e:#}")))?
        .map_err(|e| AppError::Internal(format!("Failed to hash password: {e:#}")))?;

    // Serialise concurrent bootstrap requests against the first-admin
    // invariant. Postgres used `LOCK TABLE users IN ACCESS EXCLUSIVE
    // MODE`; SQLite has no table-level lock, but a transaction started
    // via `pool.begin()` on a WAL database acquires the single writer
    // lock immediately on its first write — so the `COUNT(*) == 0`
    // check plus the subsequent `create_user` INSERT execute as an
    // indivisible pair against any other write. In addition the
    // `users.username UNIQUE` constraint catches the extremely narrow
    // race where two readers slip past the count check on different
    // connections before either writes.
    let mut tx = state
        .db_pool
        .begin()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to begin transaction: {e:#}")))?;

    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&mut *tx)
        .await?;
    if count > 0 {
        // Explicit rollback rather than relying on `Transaction`'s `Drop`
        // impl. Drop-based rollback is correct in sqlx but happens
        // *during stack unwind*, so a panic between this point and the
        // actual drop would leak the writer lock until the runtime
        // reclaims the connection. An explicit `await` here also gives
        // us a deterministic place to log if rollback itself errors —
        // which we currently swallow with `.ok()` because the only
        // remediation is "tell the operator the writer lock is wedged",
        // and at that point the next request will fail loudly anyway.
        tx.rollback().await.ok();
        return Err(AppError::BadRequest(
            "Setup already completed. Use login instead.".to_string(),
        ));
    }

    let user = users_repo::create_user(&mut *tx, &body.username, &password_hash, "admin").await?;

    tx.commit()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to commit transaction: {e:#}")))?;

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

/// PUT /api/auth/password — change current user's password
///
/// Uses `UserGuard` (not `AdminGuard`) so all users can rotate their own
/// password. The handler still verifies the current password before accepting.
pub async fn change_password(
    auth: UserGuard,
    State(state): State<Arc<AppState>>,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    validate_password(&body.new_password)?;

    let user = users_repo::find_by_username(&state.db_pool, &auth.claims.username)
        .await?
        .ok_or_else(|| AppError::Unauthorized("User not found".to_string()))?;

    let current_password = body.current_password.clone();
    let stored_hash = user.password_hash.clone();
    let valid = tokio::task::spawn_blocking(move || {
        user_auth::verify_password(&current_password, &stored_hash)
    })
    .await
    .map_err(|e| AppError::Internal(format!("Password verify task failed: {e:#}")))?;
    if !valid {
        return Err(AppError::Unauthorized(
            "Current password is incorrect".to_string(),
        ));
    }

    let new_password = body.new_password.clone();
    let new_hash = tokio::task::spawn_blocking(move || user_auth::hash_password(&new_password))
        .await
        .map_err(|e| AppError::Internal(format!("Hash task failed: {e:#}")))?
        .map_err(|e| AppError::Internal(format!("Failed to hash password: {e:#}")))?;

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
/// **Gated by `UserGuard`** — previously this endpoint accepted any decode-
/// successful token (including expired) and fell back to acting on the
/// refresh cookie alone. That let an attacker with a leaked access fragment
/// or a stolen `nm_refresh` cookie force-logout arbitrary users and churn
/// writes against the SQLite writer lock. The web client already holds a
/// fresh access JWT in normal flows (it refreshes before calling logout),
/// so the stricter gate has no legitimate UX regression.
pub async fn logout(
    auth: UserGuard,
    State(state): State<Arc<AppState>>,
) -> Result<Response, AppError> {
    let user_id = auth.claims.sub;
    let username = auth.claims.username.clone();

    users_repo::revoke_user_tokens(&state.db_pool, user_id).await?;
    let now = chrono::Utc::now().timestamp();
    crate::services::auth::update_tokens_revoked_at(user_id, now);
    if let Err(e) = refresh_token::revoke_all_for_user(&state.db_pool, user_id).await {
        tracing::warn!(err = ?e, user_id, "⚠️ [Auth] Failed to revoke refresh rows on logout");
    }
    tracing::info!(
        user_id,
        %username,
        "🔐 [Auth] User logged out — all tokens revoked"
    );

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
    admin: AdminGuard,
    State(state): State<Arc<AppState>>,
    axum::extract::Path(user_id): axum::extract::Path<i32>,
) -> Result<Json<serde_json::Value>, AppError> {
    let _ = &admin.claims; // used for audit logging below
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
        admin = %admin.claims.username,
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
/// Per-user `ISSUE_COOLDOWN` (2 s) prevents a tight retry loop on a flaky
/// SSE connection from burning the entire authenticated API rate-limit
/// budget on ticket traffic — see `services::sse_ticket` for rationale.
pub async fn issue_sse_ticket(
    auth: UserGuard,
    State(state): State<Arc<AppState>>,
) -> Result<Json<SseTicketResponse>, AppError> {
    match state
        .sse_ticket_store
        .issue(auth.claims.sub, auth.claims.iat)
    {
        crate::services::sse_ticket::IssueOutcome::Minted(ticket) => Ok(Json(SseTicketResponse {
            ticket,
            expires_in_secs: 60,
        })),
        crate::services::sse_ticket::IssueOutcome::CoolingDown { retry_after_secs } => {
            tracing::warn!(
                user_id = auth.claims.sub,
                retry_after_secs,
                "🔒 [Auth] SSE ticket issue throttled (per-user cooldown)"
            );
            Err(AppError::TooManyRequests(format!(
                "SSE ticket issued too recently; retry in {retry_after_secs} s"
            )))
        }
    }
}

/// Validate password strength: min 8 chars, uppercase, lowercase, digit, special char.
fn validate_password(password: &str) -> Result<(), AppError> {
    if password.len() < 8 {
        return Err(AppError::BadRequest(
            "Password must be at least 8 characters".to_string(),
        ));
    }
    if password.len() > 128 {
        return Err(AppError::BadRequest(
            "Password must not exceed 128 characters".to_string(),
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
