use crate::models::GpuInfo;

/// Collect NVIDIA GPU metrics via NVML. Returns empty vec if no NVIDIA GPU or driver is available.
pub fn collect() -> Vec<GpuInfo> {
    let nvml = match nvml_wrapper::Nvml::init() {
        Ok(n) => n,
        Err(_) => return vec![],
    };
    let count = match nvml.device_count() {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    (0..count)
        .filter_map(|i| {
            let device = nvml.device_by_index(i).ok()?;
            let name = device.name().unwrap_or_default();
            let utilization = device.utilization_rates().ok()?;
            let memory = device.memory_info().ok()?;
            let temp = device
                .temperature(nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu)
                .unwrap_or(0);
            let power_mw = device.power_usage().ok();
            Some(GpuInfo {
                name,
                gpu_usage_percent: utilization.gpu,
                memory_used_mb: memory.used / 1024 / 1024,
                memory_total_mb: memory.total / 1024 / 1024,
                temperature_c: temp,
                power_watts: power_mw.map(|mw| mw as f32 / 1000.0),
                frequency_mhz: device
                    .clock_info(nvml_wrapper::enum_wrappers::device::Clock::Graphics)
                    .ok(),
            })
        })
        .collect()
}
