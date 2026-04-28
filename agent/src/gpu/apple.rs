use std::sync::LazyLock;

use crate::models::GpuInfo;

/// Lazily cached `SocInfo` — `chip_name` / `gpu_cores` are hardware-fixed
/// and never change at runtime, so re-querying IOKit on every 10 s scrape
/// was wasted work. This is plain data (String + Vec<u32> + u8) and
/// therefore `Send + Sync`, which lets it live in a `static LazyLock`.
///
/// `macmon::Sampler` cannot be cached the same way: it wraps
/// `*const IOReportSubscription` + a `*mut __CFDictionary`, neither of
/// which implements `Send`, so it must continue being constructed per
/// call on whichever thread `spawn_blocking` picks. Keep that path short.
static SOC_INFO: LazyLock<Option<macmon::SocInfo>> = LazyLock::new(|| macmon::SocInfo::new().ok());

/// Collect Apple Silicon GPU metrics via macmon (IOReport).
/// Returns empty vec on non-Apple-Silicon hardware or if macmon fails.
pub fn collect() -> Vec<GpuInfo> {
    collect_inner().unwrap_or_default()
}

fn collect_inner() -> Option<Vec<GpuInfo>> {
    let soc = SOC_INFO.as_ref()?;
    let mut sampler = macmon::Sampler::new().ok()?;
    // Sample for 200ms to match the CPU delta measurement duration in collect_sysinfo
    let metrics = sampler.get_metrics(200).ok()?;

    let (freq_mhz, usage_pct) = metrics.gpu_usage;
    let gpu_name = format!("{} GPU ({}cores)", soc.chip_name, soc.gpu_cores);

    Some(vec![GpuInfo {
        name: gpu_name,
        gpu_usage_percent: (usage_pct.clamp(0.0, 100.0)) as u32,
        // Apple Silicon uses unified memory — dedicated VRAM metrics don't apply
        memory_used_mb: 0,
        memory_total_mb: 0,
        temperature_c: (metrics.temp.gpu_temp_avg.max(0.0)) as u32,
        power_watts: Some(metrics.gpu_power),
        frequency_mhz: Some(freq_mhz),
    }])
}
