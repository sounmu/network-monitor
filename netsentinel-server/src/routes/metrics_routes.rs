use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};

use crate::handlers::{
    alert_configs_handler, alert_history_handler, auth_handler, dashboard_handler, hosts_handler,
    metrics_handler, monitors_handler, notification_channels_handler, sse_handler,
};
use crate::models::app_state::AppState;

/// Assemble and return the full application router.
pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(metrics_handler::root_handler))
        // Prometheus metrics export (no auth — designed for Prometheus scraper)
        .route("/metrics", get(metrics_handler::prometheus_metrics))
        // Metrics query (batch must be registered before {host_key} to avoid capture)
        .route("/api/metrics/batch", post(metrics_handler::batch_metrics))
        .route(
            "/api/metrics/{host_key}",
            get(metrics_handler::get_metrics_by_host_key),
        )
        // Auth (login/setup are unauthenticated)
        .route("/api/auth/login", post(auth_handler::login))
        .route("/api/auth/setup", post(auth_handler::setup))
        .route("/api/auth/status", get(auth_handler::auth_status))
        .route("/api/auth/me", get(auth_handler::me))
        .route(
            "/api/auth/password",
            axum::routing::put(auth_handler::change_password),
        )
        // SSE ticket — mint a single-use ticket for the /api/stream handshake.
        // Requires a valid user JWT; the ticket replaces exposing the long-lived
        // JWT on a query string. See services::sse_ticket for details.
        .route("/api/auth/sse-ticket", post(auth_handler::issue_sse_ticket))
        // Refresh — rotate the httpOnly refresh cookie and mint a fresh
        // short-lived access JWT. Requires no bearer — session continuity
        // is proved via the cookie. See services::refresh_token.
        .route("/api/auth/refresh", post(auth_handler::refresh))
        // Logout — caller revokes all of their own JWTs.
        .route("/api/auth/logout", post(auth_handler::logout))
        // Admin kill-switch — force-revoke every session for a target user.
        .route(
            "/api/admin/users/{id}/revoke-sessions",
            post(auth_handler::admin_revoke_user_sessions),
        )
        // Health check (no auth — for load balancers and deploy verification)
        .route("/api/health", get(metrics_handler::health_check))
        // Dashboard layout
        .route(
            "/api/dashboard",
            get(dashboard_handler::get_dashboard).put(dashboard_handler::save_dashboard),
        )
        // Uptime
        .route("/api/uptime/{host_key}", get(metrics_handler::get_uptime))
        // Public status page (no auth required)
        .route("/api/public/status", get(metrics_handler::public_status))
        // Host CRUD
        .route(
            "/api/hosts",
            get(hosts_handler::list_hosts).post(hosts_handler::create_host),
        )
        .route(
            "/api/hosts/{host_key}",
            get(hosts_handler::get_host)
                .put(hosts_handler::update_host)
                .delete(hosts_handler::delete_host),
        )
        // Alert config CRUD
        .route(
            "/api/alert-configs",
            get(alert_configs_handler::get_global_configs)
                .put(alert_configs_handler::update_global_configs),
        )
        .route(
            "/api/alert-configs/bulk",
            post(alert_configs_handler::bulk_update_host_configs),
        )
        .route(
            "/api/alert-configs/{host_key}",
            get(alert_configs_handler::get_host_configs)
                .put(alert_configs_handler::update_host_configs)
                .delete(alert_configs_handler::delete_host_configs),
        )
        // Active alerts (currently firing, computed from alert_history)
        .route(
            "/api/alerts/active",
            get(alert_history_handler::get_active_alerts),
        )
        // Notification channels CRUD
        .route(
            "/api/notification-channels",
            get(notification_channels_handler::list_channels)
                .post(notification_channels_handler::create_channel),
        )
        .route(
            "/api/notification-channels/{id}",
            axum::routing::put(notification_channels_handler::update_channel)
                .delete(notification_channels_handler::delete_channel),
        )
        .route(
            "/api/notification-channels/{id}/test",
            post(notification_channels_handler::test_channel),
        )
        // HTTP monitors
        .route(
            "/api/http-monitors",
            get(monitors_handler::list_http_monitors).post(monitors_handler::create_http_monitor),
        )
        .route(
            "/api/http-monitors/summaries",
            get(monitors_handler::get_http_summaries),
        )
        .route(
            "/api/http-monitors/{id}",
            axum::routing::put(monitors_handler::update_http_monitor)
                .delete(monitors_handler::delete_http_monitor),
        )
        .route(
            "/api/http-monitors/{id}/results",
            get(monitors_handler::get_http_results),
        )
        // Ping monitors
        .route(
            "/api/ping-monitors",
            get(monitors_handler::list_ping_monitors).post(monitors_handler::create_ping_monitor),
        )
        .route(
            "/api/ping-monitors/summaries",
            get(monitors_handler::get_ping_summaries),
        )
        .route(
            "/api/ping-monitors/{id}",
            axum::routing::put(monitors_handler::update_ping_monitor)
                .delete(monitors_handler::delete_ping_monitor),
        )
        .route(
            "/api/ping-monitors/{id}/results",
            get(monitors_handler::get_ping_results),
        )
        // Alert history
        .route(
            "/api/alert-history",
            get(alert_history_handler::get_alert_history),
        )
        // SSE real-time stream
        .route("/api/stream", get(sse_handler::sse_handler))
        .layer(DefaultBodyLimit::max(256 * 1024)) // 256 KB — prevents JSON DoS
        .with_state(state)
}
