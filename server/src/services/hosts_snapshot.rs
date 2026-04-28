//! In-memory cache of the `hosts` table + resolved `alert_configs` map.
//!
//! The scraper used to run `SELECT * FROM hosts` + `SELECT * FROM alert_configs`
//! every 10 s — hundreds of round-trips per hour for data that changes, at
//! most, when an admin mutates it. Top-10 review finding #10.
//!
//! # Design
//!
//! `Arc<RwLock<Arc<HostsSnapshot>>>` (option A of the three candidates in
//! `docs/review-20260417.md`):
//!
//! - **Read path** (scraper, any handler): acquire the outer `RwLock::read`,
//!   `Arc::clone` the inner snapshot, release the guard. ~20 ns, no clone of
//!   the underlying Vec/HashMap.
//! - **Write path** (on DB mutation handlers + 60 s background tick):
//!   build a fresh `HostsSnapshot` from the DB, acquire `RwLock::write`,
//!   replace the inner Arc, release.
//!
//! # Consistency
//!
//! Freshness SLA: **immediate** for handlers that go through this crate
//! (they call `refresh` after writing to the DB), plus a **60 s ceiling**
//! via the background tick as a backstop for missed invalidations, concurrent
//! external DB writers, or startup ordering issues. Readers always see a
//! coherent snapshot (the swap is atomic under the write guard — never a
//! half-updated state).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

use chrono::Utc;

use crate::models::agent_metrics::SystemInfoResponse;
use crate::models::app_state::AlertConfig;
use crate::repositories::alert_configs_repo;
use crate::repositories::hosts_repo::{self, HostRow};

/// Point-in-time view of the hosts + alert-configs tables used by the scraper.
///
/// Both fields are read-only after construction. Producers build a fresh
/// snapshot and swap the whole `Arc` rather than mutating in place — readers
/// that already hold an `Arc::clone` keep seeing their consistent view.
#[derive(Debug, Clone, Default)]
pub struct HostsSnapshot {
    pub hosts: Vec<HostRow>,
    /// host_key → resolved AlertConfig (host override → global fallback).
    /// `"__global__"` key holds the global defaults for hosts with no override.
    pub alert_map: HashMap<String, AlertConfig>,
}

/// Type alias for the snapshot cell stored in `AppState`. Kept as a type so
/// the `Arc<RwLock<Arc<_>>>` nesting never leaks into handler signatures.
pub type SharedHostsSnapshot = Arc<RwLock<Arc<HostsSnapshot>>>;

/// Build an empty snapshot. Used at startup before the first DB load.
pub fn empty() -> SharedHostsSnapshot {
    Arc::new(RwLock::new(Arc::new(HostsSnapshot::default())))
}

/// Read the current snapshot cheaply (atomic refcount bump + guard release).
///
/// Returning `Arc<HostsSnapshot>` lets callers deref into Vec/HashMap without
/// any clone of the underlying containers. On lock poisoning, returns the
/// last valid snapshot via `into_inner` so a writer panic does not bring
/// down the entire scrape cycle.
///
/// Poisoning is a real bug, not background noise — emit `error!` (not
/// `warn!`) so an observability stack with severity routing actually pages
/// somebody. `RwLock::into_inner` clears the poison flag, leaving the
/// in-memory state potentially inconsistent with the DB; the caller of
/// `refresh()` (or the 60 s background tick) will reseed from disk on the
/// next pass, but until then writers may overwrite a recovered-from-panic
/// snapshot silently. Callers that have a `DbPool` available should prefer
/// `load_or_reseed` so a panicked writer is followed by an immediate DB
/// reseed instead of waiting up to 60 s.
pub fn load(cell: &SharedHostsSnapshot) -> Arc<HostsSnapshot> {
    match cell.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => {
            tracing::error!(
                "❌ [HostsSnapshot] RwLock poisoned on read, recovering with stale snapshot — \
                 background tick will reseed within 60s"
            );
            poisoned.into_inner().clone()
        }
    }
}

/// Same as [`load`] but, on poison, schedules an immediate DB reseed in the
/// background instead of waiting on the 60 s ticker. The returned snapshot
/// is the recovered (possibly stale) one — the reseed lands asynchronously.
pub fn load_or_reseed(pool: &crate::db::DbPool, cell: &SharedHostsSnapshot) -> Arc<HostsSnapshot> {
    match cell.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => {
            tracing::error!(
                "❌ [HostsSnapshot] RwLock poisoned on read, recovering and triggering immediate reseed"
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

/// Update a single host's cached static system info without a full DB reload.
/// Used after a successful `/system-info` fetch so the scraper does not keep
/// seeing a stale `system_info_updated_at` until the 60 s background refresh.
pub fn apply_system_info(cell: &SharedHostsSnapshot, host_key: &str, info: &SystemInfoResponse) {
    let current = load(cell);
    let mut hosts = current.hosts.clone();
    let mut updated = false;

    for host in &mut hosts {
        if host.host_key == host_key {
            host.os_info = Some(info.os.clone());
            host.cpu_model = Some(info.cpu_model.clone());
            host.memory_total_mb = Some(info.memory_total_mb as i64);
            host.boot_time = Some(info.boot_time as i64);
            host.ip_address = Some(info.ip_address.clone());
            host.system_info_updated_at = Some(Utc::now());
            updated = true;
            break;
        }
    }

    if !updated {
        return;
    }

    let new_snapshot = Arc::new(HostsSnapshot {
        hosts,
        alert_map: current.alert_map.clone(),
    });
    match cell.write() {
        Ok(mut guard) => *guard = new_snapshot,
        Err(poisoned) => {
            tracing::error!("❌ [HostsSnapshot] RwLock poisoned on apply_system_info, recovering");
            *poisoned.into_inner() = new_snapshot;
        }
    }
}

/// Rebuild the snapshot from the DB and atomically swap it in.
///
/// Called on every mutation handler (create/update/delete host, upsert/delete
/// alert config) and periodically from the background tick. DB errors are
/// logged but non-fatal — the existing snapshot continues to serve reads.
pub async fn refresh(pool: &crate::db::DbPool, cell: &SharedHostsSnapshot) {
    let (hosts_res, alert_map_res) = tokio::join!(
        hosts_repo::list_hosts(pool),
        alert_configs_repo::load_all_as_map(pool),
    );
    let hosts = match hosts_res {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(err = ?e, "⚠️ [HostsSnapshot] refresh failed (hosts), keeping previous snapshot");
            return;
        }
    };
    let alert_map = alert_map_res.unwrap_or_else(|e| {
        tracing::warn!(err = ?e, "⚠️ [HostsSnapshot] alert_configs load failed, using empty map");
        HashMap::new()
    });
    let new_snapshot = Arc::new(HostsSnapshot { hosts, alert_map });
    match cell.write() {
        Ok(mut guard) => *guard = new_snapshot,
        Err(poisoned) => {
            tracing::error!("❌ [HostsSnapshot] RwLock poisoned on write, recovering");
            *poisoned.into_inner() = new_snapshot;
        }
    }
}

/// Spawn a 60 s background refresher as a safety net for missed invalidations
/// (concurrent external DB writers, handler bugs, startup ordering races).
///
/// The mutation handlers all call `refresh` synchronously on success, so this
/// is deliberately coarse — 60 s is the worst-case staleness the product can
/// tolerate and also saves DB round-trips under steady-state no-mutation load.
pub fn spawn_background_refresher(pool: crate::db::DbPool, cell: SharedHostsSnapshot) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Skip the immediate first tick — the caller already seeded the
        // snapshot synchronously before spawning us.
        interval.tick().await;
        loop {
            interval.tick().await;
            refresh(&pool, &cell).await;
        }
    });
}
