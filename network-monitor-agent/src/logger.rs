use std::path::PathBuf;

use chrono::Utc;
use chrono_tz::Asia::Seoul;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[derive(Clone, Debug)]
struct KstTime;

impl FormatTime for KstTime {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        let now = Utc::now().with_timezone(&Seoul);
        write!(w, "{}", now.format("%Y-%m-%d %H:%M:%S.%3f %Z"))
    }
}

// ──────────────────────────────────────────────
// Log directory selection by build mode
// ──────────────────────────────────────────────

/// Returns the absolute path to the log directory for the current build mode.
///
/// - **Debug** (`#[cfg(debug_assertions)]`): `./logs/` under the project root
/// - **Release** (`#[cfg(not(debug_assertions))]`): `/var/log/network-monitor-agent/`
///
/// The directory is created automatically if it does not exist.
/// If creation fails due to insufficient permissions in release mode,
/// a clear error message is printed and the process exits.
pub fn get_log_dir() -> PathBuf {
    let log_dir = get_log_dir_path();

    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            eprintln!(
                "\n\x1b[1;31m[ERROR] Permission denied: cannot create log directory '{}'.\x1b[0m\n\
                 \x1b[33m  → Release mode requires root privileges (sudo).\x1b[0m\n\
                 \x1b[33m  → Run with: sudo ./network-monitor-agent\x1b[0m\n",
                log_dir.display()
            );
            std::process::exit(1);
        }
        eprintln!(
            "[WARN] Failed to create log directory '{}': {} — logs will go to stdout only",
            log_dir.display(),
            e
        );
        return log_dir;
    }

    log_dir
}

#[cfg(debug_assertions)]
fn get_log_dir_path() -> PathBuf {
    // CARGO_MANIFEST_DIR points to the project root at compile time.
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    project_root.join("logs")
}

#[cfg(not(debug_assertions))]
fn get_log_dir_path() -> PathBuf {
    PathBuf::from("/var/log/network-monitor-agent")
}

// ──────────────────────────────────────────────
// tracing initialisation
// ──────────────────────────────────────────────

pub fn init_tracing() -> WorkerGuard {
    let log_dir = get_log_dir();
    let file_appender = tracing_appender::rolling::daily(&log_dir, "app.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let default_level = if cfg!(debug_assertions) {
        "debug"
    } else {
        "info"
    };

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    let console_layer = fmt::layer()
        .with_timer(KstTime)
        .with_ansi(true)
        .with_target(false)
        .pretty();

    let file_layer = fmt::layer()
        .with_timer(KstTime)
        .with_ansi(false)
        .with_writer(non_blocking)
        .json();

    tracing_subscriber::registry()
        .with(env_filter)
        .with(console_layer)
        .with(file_layer)
        .init();

    guard
}
