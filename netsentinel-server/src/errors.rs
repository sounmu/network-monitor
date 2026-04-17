use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// Application-wide error type
#[derive(Debug)]
pub enum AppError {
    /// Internal server error (500) — DB errors, lock failures, etc.
    Internal(String),
    /// Requested resource not found (404)
    NotFound(String),
    /// Invalid request format or parameters (400)
    BadRequest(String),
    /// Authentication required or credentials invalid (401).
    /// Use this **only** for session/token problems so the web client can
    /// trigger its silent refresh → re-login flow. For role/permission
    /// failures use `Forbidden` instead — returning 401 would cause the
    /// frontend's 401-handler to log out a perfectly-authenticated user.
    Unauthorized(String),
    /// Authenticated but lacks permission for the resource (403).
    Forbidden(String),
    /// Too many requests / rate limited (429)
    TooManyRequests(String),
    /// Request conflicts with current server state, e.g. duplicate key (409)
    Conflict(String),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Internal(msg) => write!(f, "Internal Error: {}", msg),
            AppError::NotFound(msg) => write!(f, "Not Found: {}", msg),
            AppError::BadRequest(msg) => write!(f, "Bad Request: {}", msg),
            AppError::Unauthorized(msg) => write!(f, "Unauthorized: {}", msg),
            AppError::Forbidden(msg) => write!(f, "Forbidden: {}", msg),
            AppError::TooManyRequests(msg) => write!(f, "Too Many Requests: {}", msg),
            AppError::Conflict(msg) => write!(f, "Conflict: {}", msg),
        }
    }
}

impl std::error::Error for AppError {}

/// Automatically convert standard fmt errors into AppError::Internal
impl From<std::fmt::Error> for AppError {
    fn from(err: std::fmt::Error) -> Self {
        AppError::Internal(format!("{err:#}"))
    }
}

/// Automatically convert sqlx DB errors into AppError::Internal.
/// Uses `{err:#}` (alternate Display) to include the full error chain.
impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        AppError::Internal(format!("Database error: {err:#}"))
    }
}

/// Allow axum to convert AppError into an HTTP response
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::Internal(msg) => {
                tracing::error!(error = %msg, status = 500, "Internal Server Error");
                // Return generic message — never expose internal details (DB errors, paths, etc.)
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error").into_response()
            }
            AppError::NotFound(msg) => {
                tracing::warn!(error = %msg, status = 404, "Not Found");
                (StatusCode::NOT_FOUND, msg).into_response()
            }
            AppError::BadRequest(msg) => {
                tracing::warn!(error = %msg, status = 400, "Bad Request");
                (StatusCode::BAD_REQUEST, msg).into_response()
            }
            AppError::Unauthorized(msg) => {
                tracing::warn!(error = %msg, status = 401, "Unauthorized");
                (StatusCode::UNAUTHORIZED, msg).into_response()
            }
            AppError::Forbidden(msg) => {
                tracing::warn!(error = %msg, status = 403, "Forbidden");
                (StatusCode::FORBIDDEN, msg).into_response()
            }
            AppError::TooManyRequests(msg) => {
                tracing::warn!(error = %msg, status = 429, "Too Many Requests");
                (StatusCode::TOO_MANY_REQUESTS, msg).into_response()
            }
            AppError::Conflict(msg) => {
                tracing::warn!(error = %msg, status = 409, "Conflict");
                (StatusCode::CONFLICT, msg).into_response()
            }
        }
    }
}
