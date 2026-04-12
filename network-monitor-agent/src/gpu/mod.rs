#[cfg(feature = "gpu-apple")]
mod apple;
#[cfg(feature = "gpu-nvidia")]
mod nvidia;

use crate::models::GpuInfo;

/// Collect GPU metrics from all enabled backends.
/// Returns empty vec when no GPU or no supported backend is available.
pub fn collect_gpu_info() -> Vec<GpuInfo> {
    #[allow(unused_mut)]
    let mut gpus = Vec::new();

    #[cfg(feature = "gpu-nvidia")]
    gpus.extend(nvidia::collect());

    #[cfg(feature = "gpu-apple")]
    gpus.extend(apple::collect());

    gpus
}
