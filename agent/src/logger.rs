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
/// - **Release** (`#[cfg(not(debug_assertions))]`): `/var/log/netsentinel-agent/`
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
                 \x1b[33m  → Run with: sudo ./netsentinel-agent\x1b[0m\n",
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
    PathBuf::from("/var/log/netsentinel-agent")
}

// ──────────────────────────────────────────────
// tracing initialisation
// ──────────────────────────────────────────────

/// Default daily log retention. Overridable via `LOG_RETENTION_DAYS` env var.
///
/// Was 180 days, which under a 10 s scrape cadence with per-cycle INFO lines
/// produced ~14 GB of self-generated log pressure over the retention window —
/// enough for a monitor agent to trip its own disk alarms. 14 days is plenty
/// for postmortem investigations while keeping the on-host footprint bounded.
const DEFAULT_LOG_RETENTION_DAYS: usize = 14;

fn resolve_log_retention_days() -> usize {
    std::env::var("LOG_RETENTION_DAYS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_LOG_RETENTION_DAYS)
}

pub fn init_tracing() -> WorkerGuard {
    let log_dir = get_log_dir();
    let retention_days = resolve_log_retention_days();
    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("app")
        .filename_suffix("log")
        .max_log_files(retention_days)
        .build(&log_dir)
        .expect("failed to create rolling file appender");
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
