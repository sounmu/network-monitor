//! SQLite database entry point.
//!
//! The process opens exactly one SQLite pool and runs the embedded
//! migrations against it. `main.rs` calls `connect()` and
//! `run_migrations()` and stores the result on `AppState` through
//! the `DbPool` alias. Retiring the Postgres path collapses a dozen
//! branching `#[cfg]` blocks per repo — the single-backend tree is
//! strictly easier to reason about and is what self-hosters actually
//! deploy.

use anyhow::Context;

/// Pool alias reserved for the day we ever grow a second backend —
/// today it is always a SQLite pool. Downstream modules import
/// `crate::db::DbPool` rather than `sqlx::SqlitePool` so future
/// swaps remain a one-line change in this file.
pub type DbPool = sqlx::SqlitePool;

/// Open the SQLite database, applying the pragma set chosen in
/// docs/SQLITE_MIGRATION.md §3. The file is created if missing —
/// first-run UX assumes an operator runs the binary against an
/// empty path and expects it to "just work".
pub async fn connect(
    database_url: &str,
    max_connections: u32,
    _statement_timeout_secs: u64,
) -> anyhow::Result<DbPool> {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    // Env-configurable SQLite pragmas — operators can tune per-host.
    // Defaults target mid-tier Raspberry Pi / small VPS:
    //   - mmap_size    64 MiB (SQLITE_MMAP_SIZE,     bytes)
    //   - cache_size   8 MiB  (SQLITE_CACHE_SIZE_KB, KiB; SQLite pragma takes negative = KiB)
    //   - temp_store   MEMORY (SQLITE_TEMP_STORE: DEFAULT | FILE | MEMORY)
    // Raise mmap_size / cache_size for hosts with more RAM; temp_store=MEMORY
    // avoids disk spill for long-range queries (>14d window).
    let mmap_size = std::env::var("SQLITE_MMAP_SIZE")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(67_108_864);
    let cache_size_kib = std::env::var("SQLITE_CACHE_SIZE_KB")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(8_192);
    let temp_store = std::env::var("SQLITE_TEMP_STORE")
        .ok()
        .and_then(|v| {
            let upper = v.to_ascii_uppercase();
            match upper.as_str() {
                "DEFAULT" | "FILE" | "MEMORY" => Some(upper),
                _ => None,
            }
        })
        .unwrap_or_else(|| "MEMORY".to_string());

    // SQLite's `busy_timeout` is how long a single connection waits for the
    // writer lock before giving up; `acquire_timeout` is how long a caller
    // waits to check out a connection from the pool. Making them equal
    // (both 5 s historically) sets up a cascading failure: when a writer
    // genuinely holds the lock for ~5 s (e.g. first cold `retention_worker`
    // purge + a concurrent `rollup_worker` bucket), every in-flight query
    // exhausts `busy_timeout` *and* every new handler exhausts
    // `acquire_timeout` at the same instant. Bumping the pool timeout above
    // the query timeout lets the query failure surface first while fresh
    // handlers keep queueing — the server degrades instead of double-failing.
    const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
    const ACQUIRE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(7);

    let connect_options = SqliteConnectOptions::from_str(database_url)
        .context("Invalid DATABASE_URL — SQLite expects `sqlite://path` or `sqlite::memory:`")?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
        .busy_timeout(BUSY_TIMEOUT)
        .pragma("mmap_size", mmap_size.to_string())
        .pragma("cache_size", format!("-{cache_size_kib}"))
        .pragma("temp_store", temp_store)
        .pragma("wal_autocheckpoint", "1000");

    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .min_connections(1)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .connect_with(connect_options)
        .await
        .context("Failed to open SQLite database")?;

    tracing::info!("✅ [DB] Connected to SQLite (WAL mode).");
    Ok(pool)
}

pub async fn run_migrations(pool: &DbPool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("Failed to run SQLite migrations")?;
    tracing::info!("✅ [DB] SQLite migrations applied.");
    Ok(())
}
