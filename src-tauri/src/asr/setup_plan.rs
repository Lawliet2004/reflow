//! Hardware interpretation shared by first-run UI and the managed installer.
use serde::Serialize;

use super::python_setup::{pin_for, target_triple, torch_variant, TorchVariant};
use crate::capability::{Capabilities, GpuVendor};

fn version(value: Option<&str>) -> Option<(u32, u32)> {
    let mut parts = value?.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// Match the adapter the sidecar targets (cuda:0), rather than any faster GPU.
/// CUDA 13 wheels require Turing+; CUDA 12.6 retains Maxwell/Pascal/Volta.
/// Unknown hardware stays on CPU instead of downloading unusable GPU wheels.
pub(crate) fn compatible_variant(target: &str, caps: &Capabilities) -> TorchVariant {
    if target.contains("apple-darwin") {
        return torch_variant(target, false);
    }
    let gpu = caps
        .gpus
        .iter()
        .filter(|gpu| gpu.vendor == GpuVendor::Nvidia)
        .min_by_key(|gpu| gpu.index);
    let driver = version(caps.cuda.driver_cuda_version.as_deref());
    let compute = gpu.and_then(|gpu| version(gpu.compute_capability.as_deref()));
    match (driver, compute) {
        (Some(driver), Some(compute)) if driver >= (13, 0) && compute >= (7, 5) => {
            TorchVariant::Cuda
        }
        // Legacy wheels are published for x86_64, not Linux ARM.
        (Some(driver), Some(compute))
            if target.starts_with("x86_64")
                && driver >= (12, 6)
                && ((5, 0)..=(9, 0)).contains(&compute) =>
        {
            TorchVariant::CudaLegacy
        }
        _ => TorchVariant::Cpu,
    }
}

pub(crate) fn cpu_has_avx2() -> Option<bool> {
    #[cfg(target_arch = "x86_64")]
    {
        Some(std::arch::is_x86_feature_detected!("avx2"))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        None
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SpeechSetupPlan {
    pub hardware: Capabilities,
    pub architecture: String,
    pub cpu_has_avx2: Option<bool>,
    pub torch_variant: String,
    pub acceleration: String,
    pub explanation: String,
    pub required_disk_bytes: u64,
    pub available_disk_bytes: Option<u64>,
    pub supported_models: Vec<String>,
    pub blocker: Option<String>,
}

pub(crate) fn disk_estimate(variant: TorchVariant) -> u64 {
    // Includes the interpreter, installed packages, pip cache and staging.
    match variant {
        TorchVariant::Cuda | TorchVariant::CudaLegacy => 14_000_000_000,
        TorchVariant::None_ => 2_000_000_000,
        _ => 5_000_000_000,
    }
}

#[tauri::command]
pub async fn get_speech_setup_plan() -> Result<SpeechSetupPlan, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let caps = crate::capability::capabilities_uncached();
        let variant = super::python_setup::resolved_torch_variant_for(&caps);
        let target = target_triple();
        let avx2 = cpu_has_avx2();
        let app_dir = crate::platform::PlatformSys::get_app_dir();
        let disks = sysinfo::Disks::new_with_refreshed_list();
        let available = disks.iter().filter(|disk| app_dir.starts_with(disk.mount_point()))
            .max_by_key(|disk| disk.mount_point().as_os_str().len()).map(|disk| disk.available_space());
        let required = disk_estimate(variant);
        let blocker = if pin_for(target).is_none() {
            Some("Automatic speech setup is not available for this operating system and processor yet.".into())
        } else if available.is_some_and(|free| free < required + required / 10) {
            Some("There is not enough storage for speech setup. Free some space, then check again. Model downloads need additional space.".into())
        } else { None };
        let (acceleration, explanation) = match variant {
            TorchVariant::Cuda | TorchVariant::CudaLegacy => ("NVIDIA GPU", "Your graphics card and driver support GPU speech. Setup includes the matching acceleration libraries; no separate CUDA toolkit is needed."),
            TorchVariant::None_ => ("CPU", "This Intel Mac supports Zipformer speech. The other Python speech models require Apple Silicon on macOS."),
            _ if caps.gpus.iter().any(|gpu| gpu.vendor == GpuVendor::Amd) => ("CPU", "AMD graphics detected. Automatic AMD acceleration is not supported by this speech runtime yet. CPU dictation works without extra GPU software."),
            _ if caps.cuda.nvidia_gpu_present => ("CPU", "NVIDIA graphics detected, but a compatible GPU and driver could not be verified. Setup uses CPU packages so you can get started. You can check again after updating your graphics driver."),
            _ => ("CPU", "Speech will run on your processor. No extra graphics software is needed."),
        };
        SpeechSetupPlan {
            hardware: caps, architecture: std::env::consts::ARCH.into(), cpu_has_avx2: avx2,
            torch_variant: variant.receipt_name().into(), acceleration: acceleration.into(), explanation: explanation.into(),
            required_disk_bytes: required, available_disk_bytes: available,
            supported_models: if variant == TorchVariant::None_ { vec!["zipformer-20m".into()] } else {
                ["0.6b", "1.7b", "phonon-2", "zipformer-20m"].map(String::from).to_vec()
            }, blocker,
        }
    }).await.map_err(|error| format!("Could not check this computer: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selects_only_compatible_gpu_packages() {
        let mut caps: Capabilities = serde_json::from_value(serde_json::json!({
            "gpus": [{"index":0,"name":"GPU","vendor":"nvidia","total_vram_mb":8000.0,"used_vram_mb":0.0,"free_vram_mb":8000.0,"driver_version":null,"compute_capability":"8.6"}],
            "cuda":{"nvidia_gpu_present":true,"driver_present":true,"driver_cuda_version":"13.0","torch_cuda_available":false,"torch_cuda_version":null},
            "vulkan":{"available":false,"api_version":null,"devices":[]},
            "cpu":{"model":"CPU","physical_cores":4,"logical_cores":8,"load_pct":0.0},
            "ram":{"total_mb":16000.0,"used_mb":0.0,"available_mb":16000.0,"app_mb":0.0,"asr_mb":0.0,"refinement_mb":0.0},"os_name":"Linux","probed_at":""
        })).unwrap();
        let target = "x86_64-unknown-linux-gnu";
        assert_eq!(compatible_variant(target, &caps), TorchVariant::Cuda);
        caps.cuda.driver_cuda_version = Some("12.8".into());
        assert_eq!(compatible_variant(target, &caps), TorchVariant::CudaLegacy);
        caps.gpus[0].compute_capability = Some("6.1".into());
        caps.cuda.driver_cuda_version = Some("13.0".into());
        assert_eq!(compatible_variant(target, &caps), TorchVariant::CudaLegacy);
        assert_eq!(
            compatible_variant("aarch64-unknown-linux-gnu", &caps),
            TorchVariant::Cpu
        );
        caps.cuda.driver_cuda_version = Some("12.4".into());
        assert_eq!(compatible_variant(target, &caps), TorchVariant::Cpu);
        caps.gpus[0].vendor = GpuVendor::Amd;
        assert_eq!(compatible_variant(target, &caps), TorchVariant::Cpu);
        caps.gpus.clear();
        assert_eq!(compatible_variant(target, &caps), TorchVariant::Cpu);
        assert_eq!(
            compatible_variant("x86_64-apple-darwin", &caps),
            TorchVariant::None_
        );
        assert_eq!(
            compatible_variant("aarch64-apple-darwin", &caps),
            TorchVariant::Pypi
        );
    }
}
