//! OS-level metric collection via the `sysinfo` crate.
//!
//! Runs on `spawn_blocking` because sysinfo's refresh APIs are synchronous
//! and the CPU delta sampling requires deliberate sleeps. GPU collection
//! runs in parallel on its own blocking task so its sampling window
//! overlaps with the CPU delta sleeps.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Tracks whether `collect_sysinfo` has ever run to completion. The first
/// CPU-delta measurement is extra-susceptible to "all zeroes" because no
/// per-process tick baseline exists yet; giving it a longer sleep between
/// refresh phases stabilises the output at the cost of ~200 ms on the
/// very first scrape only. Subsequent scrapes revert to the tight loop.
static CPU_WARMED_UP: AtomicBool = AtomicBool::new(false);

/// One-shot guards for the "lock was poisoned, recovering" warn lines.
/// Without these, every scrape after a panic emits a fresh WARN, flooding
/// logs at 6 lines/min/host indefinitely. Once the operator has been
/// notified once, subsequent recoveries are a silent no-op (still
/// recovered, just not re-logged).
static DISK_IO_POISON_LOGGED: AtomicBool = AtomicBool::new(false);
static NET_PREV_POISON_LOGGED: AtomicBool = AtomicBool::new(false);
static SYS_POISON_LOGGED: AtomicBool = AtomicBool::new(false);
static NETS_POISON_LOGGED: AtomicBool = AtomicBool::new(false);
static COMPS_POISON_LOGGED: AtomicBool = AtomicBool::new(false);
use sysinfo::{Components, DiskUsage, Disks, Networks, System};

use crate::gpu;
use crate::models::{
    DiskInfo, LoadAverage, NetworkInterfaceInfo, NetworkTotal, ProcessInfo, SysinfoResult,
    TemperatureInfo,
};

/// Virtual/dummy interface prefix list shared by every host OS.
///
/// Filtering on the agent side because:
/// - It reduces the payload sent to the server (there can be many veth/docker interfaces).
/// - The server does not need to maintain a separate filter list.
const FILTERED_PREFIXES: &[&str] = &[
    "lo",     // loopback (lo, lo0, lo1)
    "docker", // Docker default bridge
    "br-",    // container user-defined bridge
    "veth",   // per-container virtual ethernet
    "utun",   // userspace tunnel (VPN, etc.)
    "gif",    // Generic Tunnel Interface (macOS + BSD)
    "stf",    // IPv6-in-IPv4 (6to4) tunnel (macOS + BSD)
];

/// Apple-only prefixes. OpenWRT exposes `apcli0` (AP-client mode) which
/// starts with `ap` and would be silently dropped if this list was
/// unconditionally merged into `FILTERED_PREFIXES`. Gate it behind
/// `#[cfg(target_os = "macos")]` so Linux keeps its real Wi-Fi client
/// interface visible.
#[cfg(target_os = "macos")]
const APPLE_FILTERED_PREFIXES: &[&str] = &[
    "awdl", // Apple Wireless Direct Link (AirDrop)
    "llw",  // Low-latency WLAN (iPhone tethering)
    "anpi", // Apple Network Proxy Interface
    "ap",   // Apple internal wireless AP
];

#[cfg(not(target_os = "macos"))]
const APPLE_FILTERED_PREFIXES: &[&str] = &[];

fn is_physical_interface(name: &str) -> bool {
    !FILTERED_PREFIXES.iter().any(|p| name.starts_with(p))
        && !APPLE_FILTERED_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Previous disk I/O counters for delta calculation, keyed by device name.
type DiskIoPrev = HashMap<String, (u64, u64, Instant)>;
static DISK_IO_PREV: LazyLock<Mutex<DiskIoPrev>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Previous aggregate network counters (rx, tx, sampled-at) for computing
/// bandwidth in bytes-per-second on the agent side. Kept next to
/// `DISK_IO_PREV` because the sampling contract is identical — both are
/// cumulative kernel counters the agent differentiates so the rest of the
/// stack stores / graphs a rate directly.
static NET_PREV: LazyLock<Mutex<Option<(u64, u64, Instant)>>> = LazyLock::new(|| Mutex::new(None));

/// Cached sysinfo instances — reused across collection cycles instead of creating fresh each time.
static SYS: LazyLock<Mutex<System>> = LazyLock::new(|| Mutex::new(System::new()));
static NETS: LazyLock<Mutex<Networks>> =
    LazyLock::new(|| Mutex::new(Networks::new_with_refreshed_list()));
static COMPS: LazyLock<Mutex<Components>> =
    LazyLock::new(|| Mutex::new(Components::new_with_refreshed_list()));

/// Serializes `collect_sysinfo` calls at the async layer so concurrent scrapes
/// cooperate instead of piling up on `SYS`'s `std::sync::Mutex` inside blocking
/// threads. Without this, a server retry or health-check arriving during the
/// ~200 ms CPU-delta window would wake a second blocking task that stalls on
/// the std mutex for the full sample duration — wasting a blocking-pool slot
/// and giving the caller a stale-but-slow response.
///
/// The caller uses `try_lock` and falls back to `COLLECT_CACHE` on contention,
/// so the gate never blocks the second concurrent request — it short-circuits
/// to the most recent snapshot instead. This turns what used to be an N-way
/// serial stall (a DoS amplifier when a retry storm arrives) into N−1
/// immediate cache hits plus one in-flight collection.
static COLLECT_GATE: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

/// Most recent successful `collect_sysinfo` result, published under the gate.
/// Readers are only contended callers — the fast path (uncontended gate)
/// doesn't touch the cache at all, so this lock stays cold.
///
/// A `std::sync::RwLock` is deliberate: the cache is accessed exclusively
/// from async contexts here but never across an `.await`, so there's no
/// reason to pay for `tokio::sync::RwLock`'s wake-up machinery.
static COLLECT_CACHE: LazyLock<std::sync::RwLock<Option<(Instant, SysinfoResult)>>> =
    LazyLock::new(|| std::sync::RwLock::new(None));

/// How long a cached snapshot remains acceptable as a fallback for a
/// contended caller. 20 s = roughly 2 × the default scrape interval, so a
/// stale snapshot is at most one cycle behind. Beyond that we force the
/// caller to wait on the gate rather than returning data an operator would
/// reasonably call "stale".
const COLLECT_CACHE_MAX_AGE: Duration = Duration::from_secs(20);

fn fresh_cached_sysinfo() -> Option<SysinfoResult> {
    let cache = COLLECT_CACHE.read().ok()?;
    let (at, value) = cache.as_ref()?;
    (at.elapsed() < COLLECT_CACHE_MAX_AGE).then(|| value.clone())
}

/// Compute per-partition read/write bytes per second using sysinfo's
/// cumulative counters.
///
/// The cache is keyed by the **raw partition name** (`/dev/sda1`,
/// `/dev/nvme0n1p2`, etc.) — each partition carries an independent
/// `/proc/diskstats` counter on Linux and its own cumulative usage on
/// macOS/Windows, so collapsing partitions to a base block device (as an
/// earlier version did) destroyed the temporal baseline:
///
/// 1. First partition iter inserts current bytes under key `"sda"` with
///    `Instant::now()`.
/// 2. A few μs later the next partition on the same device hits the same
///    key, reads `elapsed ≈ 10 μs`, subtracts the first partition's
///    counter from this partition's counter, and divides by the tiny
///    elapsed → rates in the 100 GB/s–TB/s range for ordinary desktop I/O.
///
/// Per-partition rates are what `DiskInfo { name, mount_point, … }`
/// downstream wants anyway (one row per mount in the UI), so there is no
/// value in aggregating here.
fn compute_disk_io(dev_key: &str, usage: &DiskUsage) -> (f64, f64) {
    let read_bytes = usage.total_read_bytes;
    let write_bytes = usage.total_written_bytes;
    let now = Instant::now();

    let mut prev_map = DISK_IO_PREV.lock().unwrap_or_else(|e| {
        if !DISK_IO_POISON_LOGGED.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                "DISK_IO_PREV mutex was poisoned, recovering (further \
                 occurrences suppressed for the lifetime of this process)"
            );
        }
        e.into_inner()
    });

    // Minimum elapsed floor — sysinfo on some platforms can return cached
    // counter values that cause back-to-back reads to produce a sub-
    // millisecond elapsed. Even with the correct per-partition keying
    // above, any future bug that collapses keys would re-introduce the
    // division-by-microsecond hazard; a 500 ms floor caps worst-case rate
    // at 2× the real value in that degenerate case instead of 1000×+.
    const MIN_ELAPSED_SECS: f64 = 0.5;

    match prev_map.get_mut(dev_key) {
        Some(prev) => {
            compute_rate_with_baseline(prev, read_bytes, write_bytes, now, MIN_ELAPSED_SECS)
        }
        None => {
            prev_map.insert(dev_key.to_string(), (read_bytes, write_bytes, now));
            (0.0, 0.0)
        }
    }
}

/// Compute aggregate network bandwidth (bytes/sec) from the cumulative
/// rx/tx counters. Mirrors `compute_disk_io` — shares the same
/// minimum-elapsed guard so sub-millisecond clock quirks can't blow up
/// the rate.
///
/// Returns `(0.0, 0.0)` on the very first call (no baseline yet) and on
/// counter resets (e.g. post-reboot `rx` < `prev_rx` — `saturating_sub`
/// pins the delta to 0 instead of producing a nonsense spike).
fn compute_network_rate(total_rx: u64, total_tx: u64) -> (f64, f64) {
    let now = Instant::now();

    let mut prev = NET_PREV.lock().unwrap_or_else(|e| {
        if !NET_PREV_POISON_LOGGED.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                "NET_PREV mutex was poisoned, recovering (further \
                 occurrences suppressed for the lifetime of this process)"
            );
        }
        e.into_inner()
    });

    const MIN_ELAPSED_SECS: f64 = 0.5;

    match prev.as_mut() {
        Some(prev) => compute_rate_with_baseline(prev, total_rx, total_tx, now, MIN_ELAPSED_SECS),
        None => {
            *prev = Some((total_rx, total_tx, now));
            (0.0, 0.0)
        }
    }
}

/// Upper bound on how long a baseline can sit before it is considered
/// stale and the rate calculation is abandoned. Without this, a laptop
/// that wakes from sleep after an hour would divide a large counter
/// delta by `3600 s` and under-report throughput (or, conversely,
/// divide a `saturating_sub`-collapsed 0 delta by the same window and
/// under-report until the next cycle). Abandoning the sample and
/// refreshing the baseline restores accurate rates on the next tick.
///
/// 60 s is comfortably above the default 10 s scrape interval and any
/// reasonable per-host `scrape_interval_secs` override, so legitimate
/// scrapes never hit it; only clock jumps or suspend/resume do.
const MAX_ELAPSED_SECS: f64 = 60.0;

fn compute_rate_with_baseline(
    prev: &mut (u64, u64, Instant),
    current_a: u64,
    current_b: u64,
    now: Instant,
    min_elapsed_secs: f64,
) -> (f64, f64) {
    let elapsed = now.duration_since(prev.2).as_secs_f64();
    if elapsed < min_elapsed_secs {
        return (0.0, 0.0);
    }
    if elapsed > MAX_ELAPSED_SECS {
        // Baseline is stale (suspend/resume, clock jump, server lull).
        // Refresh it so the next scrape computes over a fresh window —
        // returning the last-known counters paired with the current time.
        *prev = (current_a, current_b, now);
        return (0.0, 0.0);
    }

    let rate = (
        current_a.saturating_sub(prev.0) as f64 / elapsed,
        current_b.saturating_sub(prev.1) as f64 / elapsed,
    );
    *prev = (current_a, current_b, now);
    rate
}

/// Remove stale entries from the disk I/O delta cache (hot-swapped / unmounted devices).
/// Keys match `compute_disk_io` — raw partition name as reported by sysinfo.
fn prune_disk_io_cache(disks: &Disks) {
    use std::collections::HashSet;
    let current_keys: HashSet<String> = disks
        .iter()
        .map(|d| d.name().to_string_lossy().into_owned())
        .collect();
    let mut prev_map = DISK_IO_PREV.lock().unwrap_or_else(|e| e.into_inner());
    prev_map.retain(|k, _| current_keys.contains(k));
}

#[tracing::instrument]
pub(crate) async fn collect_sysinfo() -> SysinfoResult {
    // Fast path: grab the gate non-blockingly. On contention (another caller
    // is already mid-collection) fall back to the most recent cached snapshot
    // instead of piling up on the lock — this is what prevents retry-storm
    // DoS amplification (CLAUDE.md §Graceful degradation). Only when no fresh
    // cache exists do we wait on the gate.
    let _gate = match COLLECT_GATE.try_lock() {
        Ok(g) => g,
        Err(_) => {
            if let Some(cached) = fresh_cached_sysinfo() {
                return cached;
            }
            COLLECT_GATE.lock().await
        }
    };

    // GPU collection runs on its own blocking thread — its ~200ms sampling window
    // overlaps with the CPU/process delta sleeps, hiding the latency entirely.
    let gpu_handle = tokio::task::spawn_blocking(gpu::collect_gpu_info);

    // sysinfo collection runs on a separate blocking thread.
    let sys_handle = tokio::task::spawn_blocking(|| {
        let mut sys = SYS.lock().unwrap_or_else(|e| {
            if !SYS_POISON_LOGGED.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    "SYS mutex was poisoned, recovering (further \
                     occurrences suppressed for the lifetime of this process)"
                );
            }
            e.into_inner()
        });

        // Three-phase CPU delta measurement.
        // macOS (and some Linux kernels) require three refresh_processes calls
        // to produce non-zero per-process cpu_usage() values:
        //   Phase 1: populate process list
        //   Phase 2: establish per-process CPU tick baseline
        //   Phase 3: compute delta from Phase 2
        // An earlier attempt to reduce this to two calls broke top-10 process
        // reporting on macOS — do not remove the middle call.
        //
        // Warm-up window: the very first scrape uses 200 ms sleeps instead of
        // the steady-state 100 ms, because there is no prior baseline to
        // differentiate against and the 100 ms → 200 ms bump materially
        // lowers the rate of "all zeroes" first-scrape payloads observed on
        // busy-but-jittery Linux VMs. Subsequent scrapes always have a
        // warm baseline from the previous cycle and run fast.
        let sleep_ms = if CPU_WARMED_UP.load(Ordering::Relaxed) {
            100
        } else {
            200
        };

        sys.refresh_cpu_usage();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        std::thread::sleep(Duration::from_millis(sleep_ms));

        sys.refresh_cpu_usage();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        std::thread::sleep(Duration::from_millis(sleep_ms));

        sys.refresh_cpu_usage();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        sys.refresh_memory();
        CPU_WARMED_UP.store(true, Ordering::Relaxed);

        let cpu_usage = sys.global_cpu_usage();
        let cpu_cores: Vec<f32> = sys.cpus().iter().map(|c| c.cpu_usage()).collect();
        let memory_total_mb = sys.total_memory() / 1024 / 1024;
        let memory_used_mb = sys.used_memory() / 1024 / 1024;
        let memory_usage_percent = if sys.total_memory() > 0 {
            (sys.used_memory() as f64 / sys.total_memory() as f64 * 100.0) as f32
        } else {
            0.0
        };

        // Disks (capacity + I/O)
        let disks_raw = Disks::new_with_refreshed_list();
        let disks: Vec<DiskInfo> = disks_raw
            .iter()
            .map(|disk| {
                let total_bytes = disk.total_space();
                let available_bytes = disk.available_space();
                let used_bytes = total_bytes.saturating_sub(available_bytes);
                let dev_key = disk.name().to_string_lossy().into_owned();
                let (read_bps, write_bps) = compute_disk_io(&dev_key, &disk.usage());
                DiskInfo {
                    name: disk.name().to_string_lossy().into_owned(),
                    mount_point: disk.mount_point().to_string_lossy().into_owned(),
                    total_gb: total_bytes as f64 / 1_073_741_824.0,
                    available_gb: available_bytes as f64 / 1_073_741_824.0,
                    usage_percent: if total_bytes > 0 {
                        (used_bytes as f64 / total_bytes as f64 * 100.0) as f32
                    } else {
                        0.0
                    },
                    read_bytes_per_sec: read_bps,
                    write_bytes_per_sec: write_bps,
                }
            })
            .collect();
        // Prune stale entries from disk I/O cache (hot-swapped devices)
        prune_disk_io_cache(&disks_raw);

        // Aggregate physical interface traffic + per-interface breakdown.
        let mut nets = NETS.lock().unwrap_or_else(|e| {
            if !NETS_POISON_LOGGED.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    "NETS mutex was poisoned, recovering (further \
                     occurrences suppressed for the lifetime of this process)"
                );
            }
            e.into_inner()
        });
        // `refresh(true)` keeps known interfaces and just updates their
        // counters; it does NOT remove interfaces that have disappeared
        // since the previous refresh. After a suspend/resume cycle (or
        // a Wi-Fi card teardown) `nets` will keep emitting stale rows
        // for the dropped interface with frozen counters until the
        // process restarts. Acceptable because:
        //   1. Counters are cumulative — a frozen counter contributes 0
        //      to the bandwidth delta, just slightly inflated totals.
        //   2. Real fix (`Networks::new_with_refreshed_list()` per
        //      cycle) re-allocates the entire interface list and shows
        //      up as a measurable per-scrape allocation; not worth it
        //      for a once-per-suspend edge case.
        nets.refresh(true);
        let mut network = NetworkTotal::default();
        let mut network_interfaces = Vec::new();
        for (name, data) in nets.iter().filter(|(name, _)| is_physical_interface(name)) {
            let rx = data.total_received();
            let tx = data.total_transmitted();
            network.total_rx_bytes += rx;
            network.total_tx_bytes += tx;
            network_interfaces.push(NetworkInterfaceInfo {
                name: name.to_string(),
                rx_bytes: rx,
                tx_bytes: tx,
            });
        }
        // Differentiate the aggregate counters into a real bandwidth so
        // "Network Bandwidth" in the UI is a rate (matches the
        // `DiskInfo.read_bytes_per_sec` contract). Server/frontend can
        // still read the raw `total_*_bytes` counters for alerting or
        // daily-total use cases.
        let (rx_bps, tx_bps) = compute_network_rate(network.total_rx_bytes, network.total_tx_bytes);
        network.rx_bytes_per_sec = rx_bps;
        network.tx_bytes_per_sec = tx_bps;

        // Load average (Linux/macOS — returns 0.0 on Windows)
        let la = System::load_average();
        let load_average = LoadAverage {
            one_min: la.one,
            five_min: la.five,
            fifteen_min: la.fifteen,
        };

        // Top 10 processes by CPU usage (already refreshed twice above for accurate delta).
        //
        // Process names are truncated to 128 UTF-8 bytes. Long-name offenders
        // exist in the wild: Electron/Chromium apps embed full command-line
        // args in `comm`, docker-proxy can carry multi-kB port maps, and
        // Java servers sometimes splat their full classpath into argv[0].
        // Unbounded names bloat the bincode payload, the DB `processes`
        // JSON column, and the dashboard UI row heights with no payoff.
        // 128 bytes accommodates every well-behaved binary name and stops
        // there.
        const PROCESS_NAME_MAX_BYTES: usize = 128;
        let truncate_name = |s: String| -> String {
            if s.len() <= PROCESS_NAME_MAX_BYTES {
                s
            } else {
                // Walk backwards to the nearest UTF-8 char boundary so we
                // don't split a multi-byte sequence and produce invalid UTF-8.
                let mut cut = PROCESS_NAME_MAX_BYTES;
                while cut > 0 && !s.is_char_boundary(cut) {
                    cut -= 1;
                }
                let mut truncated = s;
                truncated.truncate(cut);
                truncated.push('…');
                truncated
            }
        };
        let mut process_list: Vec<ProcessInfo> = sys
            .processes()
            .values()
            .map(|p| ProcessInfo {
                pid: p.pid().as_u32(),
                name: truncate_name(p.name().to_string_lossy().into_owned()),
                cpu_usage: p.cpu_usage(),
                memory_mb: p.memory() / 1024 / 1024,
            })
            .collect();
        if process_list.len() > 10 {
            process_list.select_nth_unstable_by(9, |a, b| {
                b.cpu_usage
                    .partial_cmp(&a.cpu_usage)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            process_list.truncate(10);
            process_list.sort_by(|a, b| {
                b.cpu_usage
                    .partial_cmp(&a.cpu_usage)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        } else {
            process_list.sort_by(|a, b| {
                b.cpu_usage
                    .partial_cmp(&a.cpu_usage)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }

        // Temperature sensors
        let mut components = COMPS.lock().unwrap_or_else(|e| {
            if !COMPS_POISON_LOGGED.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    "COMPS mutex was poisoned, recovering (further \
                     occurrences suppressed for the lifetime of this process)"
                );
            }
            e.into_inner()
        });
        components.refresh(true); // refresh in place
        let temperatures: Vec<TemperatureInfo> = components
            .iter()
            .filter_map(|c| {
                let temp = c.temperature()?;
                if temp.is_finite() {
                    Some(TemperatureInfo {
                        label: c.label().to_string(),
                        temperature_c: temp,
                    })
                } else {
                    None
                }
            })
            .collect();

        (
            cpu_usage,
            cpu_cores,
            memory_total_mb,
            memory_used_mb,
            memory_usage_percent,
            disks,
            process_list,
            temperatures,
            network,
            network_interfaces,
            load_average,
        )
    });

    // Await both blocking tasks concurrently — GPU sampling overlaps with CPU delta sleeps.
    let (gpu_result, sys_result) = tokio::join!(gpu_handle, sys_handle);
    let gpus = gpu_result.unwrap_or_else(|e| {
        tracing::warn!(err = ?e, "⚠️ [GPU] spawn_blocking panicked, returning empty");
        vec![]
    });
    let (
        cpu_usage,
        cpu_cores,
        memory_total_mb,
        memory_used_mb,
        memory_usage_percent,
        disks,
        processes,
        temperatures,
        network,
        network_interfaces,
        load_average,
    ) = sys_result.unwrap_or_else(|e| {
        tracing::error!(err = ?e, "❌ [Sysinfo] spawn_blocking panicked, returning defaults");
        (
            0.0,
            vec![],
            0,
            0,
            0.0,
            vec![],
            vec![],
            vec![],
            NetworkTotal::default(),
            vec![],
            LoadAverage {
                one_min: 0.0,
                five_min: 0.0,
                fifteen_min: 0.0,
            },
        )
    });

    let result = SysinfoResult {
        cpu_usage,
        cpu_cores,
        memory_total_mb,
        memory_used_mb,
        memory_usage_percent,
        disks,
        processes,
        temperatures,
        gpus,
        network,
        network_interfaces,
        load_average,
    };

    // Publish to the fallback cache so concurrent callers during the next
    // collection window short-circuit to this snapshot. A write-lock poisoned
    // by an earlier panic is swallowed — the cache is a best-effort fast path,
    // not correctness-critical, so a stale cache degrades gracefully into a
    // gate wait rather than propagating the poison.
    if let Ok(mut cache) = COLLECT_CACHE.write() {
        *cache = Some((Instant::now(), result.clone()));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_interface_en0() {
        assert!(is_physical_interface("en0"));
    }

    #[test]
    fn physical_interface_eth0() {
        assert!(is_physical_interface("eth0"));
    }

    #[test]
    fn filtered_loopback_lo() {
        assert!(!is_physical_interface("lo"));
    }

    #[test]
    fn filtered_docker0() {
        assert!(!is_physical_interface("docker0"));
    }

    #[test]
    fn filtered_veth() {
        assert!(!is_physical_interface("veth123"));
    }

    #[test]
    fn filtered_utun() {
        assert!(!is_physical_interface("utun0"));
    }

    // Apple-only prefixes (`awdl`, `llw`, `anpi`, `ap`) are filtered ONLY on
    // macOS — on Linux/BSD the same prefixes might appear on real interfaces
    // (e.g. OpenWRT's `apcli0` AP-client interface starts with `ap`). The
    // four tests below run only on macOS so the filter contract is verified
    // on the platform that actually applies it; running them on Linux CI
    // would assert the wrong invariant.
    #[cfg(target_os = "macos")]
    #[test]
    fn filtered_awdl() {
        assert!(!is_physical_interface("awdl0"));
    }

    #[test]
    fn filtered_bridge() {
        assert!(!is_physical_interface("br-abc123"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn filtered_llw() {
        assert!(!is_physical_interface("llw0"));
    }

    #[test]
    fn filtered_gif() {
        assert!(!is_physical_interface("gif0"));
    }

    #[test]
    fn filtered_stf() {
        assert!(!is_physical_interface("stf0"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn filtered_anpi() {
        assert!(!is_physical_interface("anpi0"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn filtered_ap() {
        assert!(!is_physical_interface("ap1"));
    }

    /// On Linux the OpenWRT `apcli0` AP-client interface must NOT be
    /// filtered — the L-A1 fix specifically narrows the `ap` prefix
    /// to macOS so this regression is impossible. Pin the contract.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn linux_keeps_apcli0_visible() {
        assert!(is_physical_interface("apcli0"));
    }

    #[test]
    fn physical_interface_wlan0() {
        assert!(is_physical_interface("wlan0"));
    }

    #[test]
    fn rate_baseline_first_sample_sets_prev_and_reports_zero() {
        let start = Instant::now();
        let mut prev = None;

        let result = match prev.as_mut() {
            Some(prev) => compute_rate_with_baseline(prev, 100, 50, start, 0.5),
            None => {
                prev = Some((100, 50, start));
                (0.0, 0.0)
            }
        };

        assert_eq!(result, (0.0, 0.0));
        assert_eq!(prev, Some((100, 50, start)));
    }

    #[test]
    fn rate_baseline_preserved_when_elapsed_guard_fails() {
        let start = Instant::now();
        let mut prev = (100, 50, start);

        let result = compute_rate_with_baseline(
            &mut prev,
            200,
            100,
            start + Duration::from_millis(100),
            0.5,
        );

        assert_eq!(result, (0.0, 0.0));
        assert_eq!(prev, (100, 50, start));
    }

    #[test]
    fn rate_baseline_updates_after_valid_elapsed() {
        let start = Instant::now();
        let mut prev = (100, 50, start);
        let now = start + Duration::from_millis(700);

        let result = compute_rate_with_baseline(&mut prev, 800, 400, now, 0.5);

        assert!((result.0 - 1000.0).abs() < 0.01);
        assert!((result.1 - 500.0).abs() < 0.01);
        assert_eq!(prev, (800, 400, now));
    }
}
