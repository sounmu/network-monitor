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

/// Compute per-disk read/write bytes per second using sysinfo's cumulative counters.
/// Cross-platform: works on Linux, macOS, and Windows via `Disk::usage()`.
fn compute_disk_io(dev_name: &str, usage: &DiskUsage) -> (f64, f64) {
    let read_bytes = usage.total_read_bytes;
    let write_bytes = usage.total_written_bytes;
    let now = Instant::now();

    let mut prev_map = DISK_IO_PREV.lock().unwrap_or_else(|e| {
        tracing::warn!("DISK_IO_PREV mutex was poisoned, recovering");
        e.into_inner()
    });

    let result = if let Some((prev_r, prev_w, prev_t)) = prev_map.get(dev_name) {
        let elapsed = now.duration_since(*prev_t).as_secs_f64();
        if elapsed > 0.0 {
            (
                read_bytes.saturating_sub(*prev_r) as f64 / elapsed,
                write_bytes.saturating_sub(*prev_w) as f64 / elapsed,
            )
        } else {
            (0.0, 0.0)
        }
    } else {
        (0.0, 0.0)
    };

    prev_map.insert(dev_name.to_string(), (read_bytes, write_bytes, now));
    result
}

/// Remove stale entries from the disk I/O delta cache (hot-swapped devices).
fn prune_disk_io_cache(disks: &Disks) {
    use std::collections::HashSet;
    let current_devs: HashSet<String> = disks
        .iter()
        .map(|d| extract_block_device(&d.name().to_string_lossy()))
        .collect();
    let mut prev_map = DISK_IO_PREV.lock().unwrap_or_else(|e| e.into_inner());
    prev_map.retain(|k, _| current_devs.contains(k));
}

/// Extract the block device name from a disk path (e.g., "/dev/sda1" → "sda").
/// Strips partition number suffix and returns the base device for sysfs lookup.
fn extract_block_device(disk_name: &str) -> String {
    let name = disk_name.strip_prefix("/dev/").unwrap_or(disk_name);
    // Strip trailing digits for partition suffix (sda1 → sda, nvme0n1p1 → nvme0n1)
    if name.starts_with("nvme") {
        // NVMe: nvme0n1p1 → nvme0n1 (let chain: collapsible if)
        if let Some(idx) = name.rfind('p')
            && !name[idx + 1..].is_empty()
            && name[idx + 1..].chars().all(|c| c.is_ascii_digit())
            && idx > 0
        {
            name[..idx].to_string()
        } else {
            name.to_string()
        }
    } else {
        // SATA/SCSI: sda1 → sda
        name.trim_end_matches(|c: char| c.is_ascii_digit())
            .to_string()
    }
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
                let dev_name = extract_block_device(&disk.name().to_string_lossy());
                let (read_bps, write_bps) = compute_disk_io(&dev_name, &disk.usage());
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
            NetworkTotal {
                total_rx_bytes: 0,
                total_tx_bytes: 0,
            },
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
}
