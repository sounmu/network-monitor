//! In-memory cache of the enabled HTTP / Ping monitor sets.
//!
//! `monitor_scraper` used to issue `SELECT ... WHERE enabled = 1` against
//! both `http_monitors` and `ping_monitors` on every 10 s sweep — twice per
//! cycle, regardless of how rarely an admin actually mutates the monitor
//! set. Mirrors the `hosts_snapshot` design (Top-10 review #9): atomic
//! `Arc<RwLock<Arc<MonitorsSnapshot>>>` swap on mutation handlers + 60 s
//! background tick as a backstop.
//!
//! # Why not reuse `hosts_snapshot`'s code path
//!
//! `HostsSnapshot` carries a derived alert-config map that this snapshot
//! has no analogue for, so a generic shared module would hide the shape
//! difference behind a generic that complicated both call sites. A small
//! parallel module is cheaper to read; the duplication is ~30 lines of
//! mechanical boilerplate, not logic.

use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

use crate::repositories::{
    http_monitors_repo::{self, HttpMonitor},
    ping_monitors_repo::{self, PingMonitor},
};

/// Point-in-time view of the enabled monitor sets.
#[derive(Debug, Clone, Default)]
pub struct MonitorsSnapshot {
    pub http: Vec<HttpMonitor>,
    pub ping: Vec<PingMonitor>,
}

pub type SharedMonitorsSnapshot = Arc<RwLock<Arc<MonitorsSnapshot>>>;

/// Build an empty snapshot. Used at startup before the first DB load.
pub fn empty() -> SharedMonitorsSnapshot {
    Arc::new(RwLock::new(Arc::new(MonitorsSnapshot::default())))
}

/// Read the current snapshot cheaply (atomic refcount bump + guard release).
///
/// Poison fallback strategy mirrors `hosts_snapshot::load_or_reseed`: log
/// at `error!` (this is a real bug, not noise) and fire-and-forget a DB
/// reseed so the recovered-but-possibly-stale snapshot is replaced
/// promptly instead of waiting up to 60 s for the periodic tick.
pub fn load(pool: &crate::db::DbPool, cell: &SharedMonitorsSnapshot) -> Arc<MonitorsSnapshot> {
    match cell.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => {
            tracing::error!(
                "❌ [MonitorsSnapshot] RwLock poisoned on read, recovering and triggering immediate reseed"
            );
            let recovered = poisoned.into_inner().clone();
            let pool = pool.clone();
            let cell = cell.clone();
            tokio::spawn(async move {
                refresh(&pool, &cell).await;
            });
            recovered
        }
    }
}

/// Rebuild the snapshot from the DB and atomically swap it in.
///
/// Called synchronously on every monitor mutation handler (POST/PUT/DELETE
/// of either monitor flavour) and also by the 60 s background tick as a
/// backstop. DB errors are logged but non-fatal — the existing snapshot
/// continues to serve reads.
pub async fn refresh(pool: &crate::db::DbPool, cell: &SharedMonitorsSnapshot) {
    let (http_res, ping_res) = tokio::join!(
        http_monitors_repo::get_enabled(pool),
        ping_monitors_repo::get_enabled(pool),
    );
    let http = match http_res {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(err = ?e, "⚠️ [MonitorsSnapshot] http refresh failed, keeping previous snapshot");
            return;
        }
    };
    let ping = match ping_res {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(err = ?e, "⚠️ [MonitorsSnapshot] ping refresh failed, keeping previous snapshot");
            return;
        }
    };
    let new_snapshot = Arc::new(MonitorsSnapshot { http, ping });
    match cell.write() {
        Ok(mut guard) => *guard = new_snapshot,
        Err(poisoned) => {
            tracing::error!("❌ [MonitorsSnapshot] RwLock poisoned on write, recovering");
            *poisoned.into_inner() = new_snapshot;
        }
    }
}

/// Spawn the 60 s safety-net refresher. Mutation handlers refresh
/// synchronously on success; this task only catches concurrent external
/// DB writers, handler bugs, or startup ordering races.
pub fn spawn_background_refresher(pool: crate::db::DbPool, cell: SharedMonitorsSnapshot) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Skip the immediate first tick — caller seeded synchronously.
        interval.tick().await;
        loop {
            interval.tick().await;
            refresh(&pool, &cell).await;
        }
    });
}
