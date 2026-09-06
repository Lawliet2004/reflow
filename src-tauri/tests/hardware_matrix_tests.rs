//! Milestone 11 / Task 46: Hardware Validation Matrix (Classes A through E).
//!
//! Validates resolution, safety boundaries, fallback paths, and memory policy
//! across all 5 reference hardware tiers:
//! - Class A: CPU-only, 4 cores, 8 GB RAM.
//! - Class B: RTX 2050 (4 GB VRAM), 6 cores, 16 GB RAM (Reference machine).
//! - Class C: 6 GB GPU, 8 cores, 16 GB RAM.
//! - Class D: 8 GB GPU, 12 cores, 32 GB RAM.
//! - Class E: 24 GB GPU (RTX 3090/4090), 16 cores, 64 GB RAM.

use reflow_lib::capability::probe::{
    Capabilities, CpuInfo, CudaStatus, GpuInfo, GpuVendor, RamInfo, VulkanStatus,
};
use reflow_lib::profile::{
    no_measurements, resolve_profile, Device, Precision, Preset, ProfileOverrides,
};

fn make_caps(
    cpu_cores: usize,
    ram_gb: f32,
    gpu: Option<(&str, f32, f32)>, // name, total_vram_mb, free_vram_mb
) -> Capabilities {
    let gpus = if let Some((name, total_mb, free_mb)) = gpu {
        vec![GpuInfo {
            index: 0,
            name: name.into(),
            vendor: GpuVendor::Nvidia,
            total_vram_mb: total_mb,
            used_vram_mb: total_mb - free_mb,
            free_vram_mb: free_mb,
            driver_version: Some("551.86".into()),
            compute_capability: Some("8.6".into()),
        }]
    } else {
        Vec::new()
    };

    let cuda = CudaStatus {
        nvidia_gpu_present: gpu.is_some(),
        driver_present: gpu.is_some(),
        driver_cuda_version: if gpu.is_some() {
            Some("12.4".into())
        } else {
            None
        },
        torch_cuda_available: gpu.is_some(),
        torch_cuda_version: if gpu.is_some() {
            Some("12.4".into())
        } else {
            None
        },
    };

    Capabilities {
        os_name: "Windows 11".into(),
        probed_at: "2026-08-28T00:00:00Z".into(),
        cpu: CpuInfo {
            model: "Test CPU".into(),
            physical_cores: Some(cpu_cores),
            logical_cores: cpu_cores,
            load_pct: 5.0,
        },
        ram: RamInfo {
            total_mb: ram_gb * 1024.0,
            used_mb: (ram_gb * 0.4) * 1024.0,
            available_mb: (ram_gb * 0.6) * 1024.0,
            app_mb: 200.0,
            asr_mb: 0.0,
            refinement_mb: 0.0,
        },
        gpus,
        cuda,
        vulkan: VulkanStatus {
            available: false,
            api_version: None,
            devices: Vec::new(),
        },
    }
}

#[test]
fn validate_class_a_cpu_only() {
    let caps = make_caps(4, 8.0, None);
    let profile = resolve_profile(
        &caps,
        Preset::Auto,
        &ProfileOverrides::default(),
        &no_measurements,
    );

    // Class A must pick CPU ASR and no GPU refinement
    assert_eq!(profile.asr_device, Device::Cpu);
    assert_eq!(profile.asr_precision, Precision::Fp32);
    assert_eq!(profile.refinement_gpu_layers, 0);
}

#[test]
fn validate_class_b_rtx_2050_4gb() {
    let caps = make_caps(6, 16.0, Some(("NVIDIA GeForce RTX 2050", 4096.0, 3400.0)));
    let profile = resolve_profile(
        &caps,
        Preset::Auto,
        &ProfileOverrides::default(),
        &no_measurements,
    );

    // Class B 4GB must select 0.6B BF16 on CUDA without spilling
    assert_eq!(profile.asr_device, Device::Cuda);
    assert_eq!(profile.asr_model, "0.6b");
    assert_eq!(profile.asr_precision, Precision::Bf16);
}

#[test]
fn validate_class_c_6gb_gpu() {
    let caps = make_caps(
        8,
        16.0,
        Some(("NVIDIA GeForce RTX 3060 Mobile", 6144.0, 5600.0)),
    );
    let profile = resolve_profile(
        &caps,
        Preset::Accurate,
        &ProfileOverrides::default(),
        &no_measurements,
    );

    assert_eq!(profile.asr_device, Device::Cuda);
    assert_eq!(profile.asr_model, "1.7b");
    assert_eq!(profile.asr_precision, Precision::Bf16);
}

#[test]
fn validate_class_d_8gb_gpu() {
    let caps = make_caps(12, 32.0, Some(("NVIDIA GeForce RTX 4070", 8192.0, 7200.0)));
    let profile = resolve_profile(
        &caps,
        Preset::Accurate,
        &ProfileOverrides::default(),
        &no_measurements,
    );

    assert_eq!(profile.asr_device, Device::Cuda);
    assert_eq!(profile.asr_model, "1.7b");
}

#[test]
fn validate_class_e_24gb_workstation() {
    let caps = make_caps(
        16,
        64.0,
        Some(("NVIDIA GeForce RTX 4090", 24576.0, 22000.0)),
    );
    let profile = resolve_profile(
        &caps,
        Preset::Accurate,
        &ProfileOverrides::default(),
        &no_measurements,
    );

    assert_eq!(profile.asr_device, Device::Cuda);
    assert_eq!(profile.asr_model, "1.7b");
    assert_eq!(profile.asr_precision, Precision::Bf16);
}
