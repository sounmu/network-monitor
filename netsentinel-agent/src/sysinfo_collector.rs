//! OS-level metric collection via the `sysinfo` crate.
//!
//! Runs on `spawn_blocking` because sysinfo's refresh APIs are synchronous
//! and the CPU delta sampling requires deliberate sleeps. GPU collection
//! runs in parallel on its own blocking task so its sampling window
//! overlaps with the CPU delta sleeps.

use std::time::Duration;
use sysinfo::{Components, Disks, Networks, System};

use crate::gpu;
use crate::models::{
    DiskInfo, LoadAverage, NetworkTotal, ProcessInfo, SysinfoResult, TemperatureInfo,
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

#[tracing::instrument]
pub(crate) async fn collect_sysinfo() -> SysinfoResult {
    // GPU collection runs on its own blocking thread — its ~200ms sampling window
    // overlaps with the CPU/process delta sleeps, hiding the latency entirely.
    let gpu_handle = tokio::task::spawn_blocking(gpu::collect_gpu_info);

    // sysinfo collection runs on a separate blocking thread.
    let sys_handle = tokio::task::spawn_blocking(|| {
        let mut sys = System::new();

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
        let memory_total_mb = sys.total_memory() / 1024 / 1024;
        let memory_used_mb = sys.used_memory() / 1024 / 1024;
        let memory_usage_percent = if sys.total_memory() > 0 {
            (sys.used_memory() as f64 / sys.total_memory() as f64 * 100.0) as f32
        } else {
            0.0
        };

        // Disks
        let disks_raw = Disks::new_with_refreshed_list();
        let disks = disks_raw
            .iter()
            .map(|disk| {
                let total_bytes = disk.total_space();
                let available_bytes = disk.available_space();
                let used_bytes = total_bytes.saturating_sub(available_bytes);
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
                }
            })
            .collect();

        // Aggregate physical interface traffic.
        let nets = Networks::new_with_refreshed_list();
        let network = nets
            .iter()
            .filter(|(name, _)| is_physical_interface(name))
            .fold(NetworkTotal::default(), |mut acc, (_, data)| {
                acc.total_rx_bytes += data.total_received();
                acc.total_tx_bytes += data.total_transmitted();
                acc
            });

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
        process_list.sort_by(|a, b| {
            b.cpu_usage
                .partial_cmp(&a.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        process_list.truncate(10);

        // Temperature sensors
        let components = Components::new_with_refreshed_list();
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
            memory_total_mb,
            memory_used_mb,
            memory_usage_percent,
            disks,
            process_list,
            temperatures,
            network,
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
        memory_total_mb,
        memory_used_mb,
        memory_usage_percent,
        disks,
        processes,
        temperatures,
        network,
        load_average,
    ) = sys_result.unwrap_or_else(|e| {
        tracing::error!(err = ?e, "❌ [Sysinfo] spawn_blocking panicked, returning defaults");
        (
            0.0,
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
            LoadAverage {
                one_min: 0.0,
                five_min: 0.0,
                fifteen_min: 0.0,
            },
        )
    });

    SysinfoResult {
        cpu_usage,
        memory_total_mb,
        memory_used_mb,
        memory_usage_percent,
        disks,
        processes,
        temperatures,
        gpus,
        network,
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
