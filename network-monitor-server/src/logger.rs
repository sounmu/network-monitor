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

pub fn init_tracing() -> WorkerGuard {
    let file_appender = tracing_appender::rolling::daily("logs", "app.log");
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
