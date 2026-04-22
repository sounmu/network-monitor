//! OS-level metric collection via the `sysinfo` crate.
//!
//! Runs on `spawn_blocking` because sysinfo's refresh APIs are synchronous
//! and the CPU delta sampling requires deliberate sleeps. GPU collection
//! runs in parallel on its own blocking task so its sampling window
//! overlaps with the CPU delta sleeps.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use sysinfo::{Components, DiskUsage, Disks, Networks, System};

use crate::gpu;
use crate::models::{
    DiskInfo, LoadAverage, NetworkInterfaceInfo, NetworkTotal, ProcessInfo, SysinfoResult,
    TemperatureInfo,
};

/// Virtual/dummy interface prefix list.
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
    "awdl",   // Apple Wireless Direct Link (AirDrop)
    "llw",    // Low-latency WLAN (iPhone tethering)
    "gif",    // Generic Tunnel Interface
    "stf",    // IPv6-in-IPv4 (6to4) tunnel
    "anpi",   // Apple Network Proxy Interface
    "ap",     // Apple internal wireless AP
];

fn is_physical_interface(name: &str) -> bool {
    !FILTERED_PREFIXES.iter().any(|p| name.starts_with(p))
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
static COLLECT_GATE: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

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
        tracing::warn!("DISK_IO_PREV mutex was poisoned, recovering");
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
        tracing::warn!("NET_PREV mutex was poisoned, recovering");
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
    // Hold the async gate for the duration of this collection — prevents a
    // second concurrent caller from spawning a blocking task that would then
    // block on SYS for the full ~200 ms sample window.
    let _gate = COLLECT_GATE.lock().await;

    // GPU collection runs on its own blocking thread — its ~200ms sampling window
    // overlaps with the CPU/process delta sleeps, hiding the latency entirely.
    let gpu_handle = tokio::task::spawn_blocking(gpu::collect_gpu_info);

    // sysinfo collection runs on a separate blocking thread.
    let sys_handle = tokio::task::spawn_blocking(|| {
        let mut sys = SYS.lock().unwrap_or_else(|e| e.into_inner());

        // Three-phase CPU delta measurement.
        // macOS (and some Linux kernels) require three refresh_processes calls
        // to produce non-zero per-process cpu_usage() values:
        //   Phase 1: populate process list
        //   Phase 2: establish per-process CPU tick baseline
        //   Phase 3: compute delta from Phase 2
        // An earlier attempt to reduce this to two calls broke top-10 process
        // reporting on macOS — do not remove the middle call.
        sys.refresh_cpu_usage();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        std::thread::sleep(Duration::from_millis(100));

        sys.refresh_cpu_usage();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        std::thread::sleep(Duration::from_millis(100));

        sys.refresh_cpu_usage();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        sys.refresh_memory();

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
        let mut nets = NETS.lock().unwrap_or_else(|e| e.into_inner());
        nets.refresh(true); // refresh in place instead of creating new
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

        // Top 10 processes by CPU usage (already refreshed twice above for accurate delta)
        let mut process_list: Vec<ProcessInfo> = sys
            .processes()
            .values()
            .map(|p| ProcessInfo {
                pid: p.pid().as_u32(),
                name: p.name().to_string_lossy().into_owned(),
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
        let mut components = COMPS.lock().unwrap_or_else(|e| e.into_inner());
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

    SysinfoResult {
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
    }
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

    #[test]
    fn filtered_awdl() {
        assert!(!is_physical_interface("awdl0"));
    }

    #[test]
    fn filtered_bridge() {
        assert!(!is_physical_interface("br-abc123"));
    }

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

    #[test]
    fn filtered_anpi() {
        assert!(!is_physical_interface("anpi0"));
    }

    #[test]
    fn filtered_ap() {
        assert!(!is_physical_interface("ap1"));
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
