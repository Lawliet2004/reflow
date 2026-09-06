//! Milestone 1 / Task 8: `resolve_profile` is a pure function, so every hardware
//! class in the validation matrix is reproducible from a fixture with no
//! hardware present.
//!
//! The fixtures below correspond to the five system classes in the plan plus the
//! two CPU-only variants.

use std::collections::HashMap;

use reflow_lib::capability::probe::{
    Capabilities, CpuInfo, CudaStatus, GpuInfo, GpuVendor, RamInfo, VulkanStatus,
};
use reflow_lib::profile::{
    asr_manifest, no_measurements, refinement_manifest, resolve_profile, vram_reserve_mb, Device,
    MeasuredPeaks, Precision, Preset, ProfileOverrides, ResolvedProfile,
};

// ---------------------------------------------------------------------------
// Hardware fixtures
// ---------------------------------------------------------------------------

fn cpu(model: &str, physical: usize, logical: usize) -> CpuInfo {
    CpuInfo {
        model: model.into(),
        physical_cores: Some(physical),
        logical_cores: logical,
        load_pct: 10.0,
    }
}

fn ram(total_gb: f32, available_gb: f32) -> RamInfo {
    RamInfo {
        total_mb: total_gb * 1024.0,
        used_mb: (total_gb - available_gb) * 1024.0,
        available_mb: available_gb * 1024.0,
        app_mb: 220.0,
        asr_mb: 0.0,
        refinement_mb: 0.0,
    }
}

fn nvidia(name: &str, total_mb: f32, free_mb: f32) -> GpuInfo {
    GpuInfo {
        index: 0,
        name: name.into(),
        vendor: GpuVendor::Nvidia,
        total_vram_mb: total_mb,
        used_vram_mb: total_mb - free_mb,
        free_vram_mb: free_mb,
        driver_version: Some("551.86".into()),
        compute_capability: Some("8.6".into()),
    }
}

fn with_cuda(caps: Capabilities) -> Capabilities {
    Capabilities {
        cuda: CudaStatus {
            nvidia_gpu_present: true,
            driver_present: true,
            driver_cuda_version: Some("12.4".into()),
            torch_cuda_available: true,
            torch_cuda_version: Some("12.4".into()),
        },
        ..caps
    }
}

/// Class A: CPU-only, 4 cores, 8 GB.
fn class_a_cpu_4core() -> Capabilities {
    Capabilities {
        cpu: cpu("Intel Core i5-8250U", 4, 8),
        ram: ram(8.0, 4.5),
        os_name: "Windows 11".into(),
        ..Default::default()
    }
}

/// CPU-only, 6 cores, 16 GB.
fn cpu_only_6core() -> Capabilities {
    Capabilities {
        cpu: cpu("AMD Ryzen 5 5600H", 6, 12),
        ram: ram(16.0, 10.0),
        os_name: "Windows 11".into(),
        ..Default::default()
    }
}

/// Class B: the primary target. RTX 2050, 4 GB, 16 GB RAM.
fn class_b_rtx_2050() -> Capabilities {
    with_cuda(Capabilities {
        gpus: vec![nvidia("NVIDIA GeForce RTX 2050", 4096.0, 3600.0)],
        cpu: cpu("AMD Ryzen 5 5600H", 6, 12),
        ram: ram(16.0, 9.0),
        os_name: "Windows 11".into(),
        ..Default::default()
    })
}

/// Class C: 6 GB discrete.
fn class_c_6gb() -> Capabilities {
    with_cuda(Capabilities {
        gpus: vec![nvidia("NVIDIA GeForce RTX 3060 Laptop", 6144.0, 5600.0)],
        cpu: cpu("Intel Core i7-11800H", 8, 16),
        ram: ram(16.0, 10.0),
        os_name: "Windows 11".into(),
        ..Default::default()
    })
}

/// Class D: 8 GB discrete.
fn class_d_8gb() -> Capabilities {
    with_cuda(Capabilities {
        gpus: vec![nvidia("NVIDIA GeForce RTX 4060", 8188.0, 7600.0)],
        cpu: cpu("AMD Ryzen 7 7840HS", 8, 16),
        ram: ram(32.0, 22.0),
        os_name: "Windows 11".into(),
        ..Default::default()
    })
}

/// Class E: 12 GB+ desktop.
fn class_e_24gb() -> Capabilities {
    with_cuda(Capabilities {
        gpus: vec![nvidia("NVIDIA GeForce RTX 4090", 24564.0, 23000.0)],
        cpu: cpu("AMD Ryzen 9 7950X", 16, 32),
        ram: ram(64.0, 48.0),
        os_name: "Windows 11".into(),
        ..Default::default()
    })
}

/// AMD, Vulkan only, no CUDA.
fn amd_vulkan() -> Capabilities {
    Capabilities {
        gpus: vec![GpuInfo {
            index: 0,
            name: "AMD Radeon RX 6600".into(),
            vendor: GpuVendor::Amd,
            total_vram_mb: 8192.0,
            used_vram_mb: 0.0,
            free_vram_mb: 0.0,
            driver_version: None,
            compute_capability: None,
        }],
        vulkan: VulkanStatus {
            available: true,
            api_version: Some("1.3.280".into()),
            devices: vec!["AMD Radeon RX 6600".into()],
        },
        cpu: cpu("AMD Ryzen 5 5600", 6, 12),
        ram: ram(16.0, 11.0),
        os_name: "Windows 11".into(),
        ..Default::default()
    }
}

/// Hybrid graphics: an Intel iGPU drives the display, an NVIDIA dGPU computes.
fn hybrid_graphics() -> Capabilities {
    with_cuda(Capabilities {
        gpus: vec![
            GpuInfo {
                index: 0,
                name: "Intel(R) Iris(R) Xe Graphics".into(),
                vendor: GpuVendor::Intel,
                total_vram_mb: 1024.0,
                used_vram_mb: 400.0,
                free_vram_mb: 624.0,
                driver_version: None,
                compute_capability: None,
            },
            nvidia("NVIDIA GeForce RTX 2050", 4096.0, 4000.0),
        ],
        cpu: cpu("Intel Core i7-1260P", 12, 16),
        ram: ram(16.0, 9.0),
        os_name: "Windows 11".into(),
        ..Default::default()
    })
}

/// A GPU that exists but whose torch build is CPU-only.
fn gpu_without_cuda() -> Capabilities {
    Capabilities {
        gpus: vec![nvidia("NVIDIA GeForce RTX 2050", 4096.0, 3600.0)],
        cuda: CudaStatus {
            nvidia_gpu_present: true,
            driver_present: true,
            driver_cuda_version: Some("12.4".into()),
            torch_cuda_available: false,
            torch_cuda_version: None,
        },
        cpu: cpu("AMD Ryzen 5 5600H", 6, 12),
        ram: ram(16.0, 9.0),
        os_name: "Windows 11".into(),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// A measurement table, for the paths that require measured data.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Measurements {
    entries: HashMap<(String, Device, Precision), MeasuredPeaks>,
}

impl Measurements {
    fn with(
        mut self,
        model: &str,
        device: Device,
        precision: Precision,
        peaks: MeasuredPeaks,
    ) -> Self {
        self.entries
            .insert((model.to_string(), device, precision), peaks);
        self
    }

    fn lookup(&self) -> impl Fn(&str, Device, Precision) -> MeasuredPeaks + '_ {
        move |model, device, precision| {
            self.entries
                .get(&(model.to_string(), device, precision))
                .copied()
                .unwrap_or_default()
        }
    }
}

fn resolve(caps: &Capabilities, preset: Preset) -> ResolvedProfile {
    resolve_profile(caps, preset, &ProfileOverrides::default(), &no_measurements)
}

// ---------------------------------------------------------------------------
// Golden expectations per hardware class
// ---------------------------------------------------------------------------

#[test]
fn class_a_cpu_only_4core_runs_everything_on_cpu() {
    let profile = resolve(&class_a_cpu_4core(), Preset::Balanced);
    assert_eq!(profile.asr_device, Device::Cpu);
    assert_eq!(profile.asr_precision, Precision::Fp32);
    assert_eq!(
        profile.asr_model, "0.6b",
        "a 4-core / 8 GB machine must not be handed the 1.7B model"
    );
    assert_eq!(profile.refinement_device, Device::Cpu);
    assert_eq!(profile.refinement_gpu_layers, 0);
    // Three of four cores; one is always held back for audio, UI and the OS.
    assert_eq!(profile.inference_threads, 7);
    assert!(
        !profile.streaming_enabled,
        "unmeasured RTF must not enable it"
    );
    assert!(profile.has_reason("asr_device_cpu"));
    assert!(profile.has_reason("refinement_cpu_no_cuda"));
}

/// Plenty of RAM does not buy the larger model on CPU: throughput is the
/// binding constraint there, not memory.
#[test]
fn cpu_only_6core_still_takes_the_small_model_despite_spare_ram() {
    let profile = resolve(&cpu_only_6core(), Preset::Accurate);
    assert_eq!(profile.asr_device, Device::Cpu);
    assert_eq!(profile.asr_precision, Precision::Fp32);
    assert_eq!(profile.inference_threads, 11);
    assert_eq!(
        profile.asr_model, "0.6b",
        "1.7B fp32 on CPU would transcribe several times slower than real time"
    );
    // And the rung it does offer must actually fit.
    assert_eq!(profile.asr_attempts.len(), 1);
    assert!(profile.asr_attempts[0].expected_peak_mb < caps_available_mb(&cpu_only_6core()));
}

fn caps_available_mb(caps: &Capabilities) -> f32 {
    caps.ram.available_mb
}

/// Class B is the primary target and the whole reason for this milestone.
#[test]
fn class_b_rtx_2050_picks_a_configuration_that_fits_4gb() {
    let caps = class_b_rtx_2050();
    let profile = resolve(&caps, Preset::Balanced);

    assert_eq!(profile.asr_device, Device::Cuda);
    assert_eq!(
        profile.asr_model, "0.6b",
        "1.7B does not fit 4 GB alongside the reserve"
    );
    assert_eq!(
        profile.asr_precision,
        Precision::Bf16,
        "BF16 fits, and INT8 at batch size 1 only adds dequant overhead"
    );

    // The refiner stays on the CPU until offload has actually been measured.
    assert_eq!(profile.refinement_model.as_deref(), Some("qwen3.5-0.8b"));
    assert_eq!(profile.refinement_device, Device::Cpu);
    assert_eq!(profile.refinement_gpu_layers, 0);
    assert!(profile.has_reason("refinement_cpu_unmeasured"));

    // Every attempt must fit the budget.
    let reserve = vram_reserve_mb(&caps);
    let budget = caps.free_vram_mb() - reserve - 256.0;
    for attempt in &profile.asr_attempts {
        assert!(
            attempt.expected_peak_mb <= budget,
            "{:?} peak {} exceeds budget {budget}",
            attempt.precision,
            attempt.expected_peak_mb
        );
    }
}

#[test]
fn class_b_never_selects_the_configuration_that_spills() {
    let profile = resolve(&class_b_rtx_2050(), Preset::Accurate);
    // 1.7B BF16 is the load that spills on a 4 GB card. It must not appear as a
    // rung at all, let alone as the chosen one.
    let has_large_bf16 = profile.asr_model == "1.7b"
        && profile
            .asr_attempts
            .iter()
            .any(|a| a.precision == Precision::Bf16 && a.device == Device::Cuda);
    assert!(!has_large_bf16, "1.7B BF16 must never be offered on 4 GB");
}

#[test]
fn class_c_6gb_can_take_the_larger_model() {
    let caps = class_c_6gb();
    let profile = resolve(&caps, Preset::Accurate);
    assert_eq!(profile.asr_device, Device::Cuda);
    assert_eq!(profile.asr_model, "1.7b", "the larger model fits 6 GB");
    // BF16 fits with room to spare, so there is no reason to pay INT8's
    // dequantization overhead.
    assert_eq!(profile.asr_precision, Precision::Bf16);
    // INT8 remains below it as a fallback rung.
    assert!(profile
        .asr_attempts
        .iter()
        .any(|a| a.precision == Precision::Int8));
}

/// INT4 must never be selected automatically: its accuracy cost is unmeasured,
/// so choosing it on the user's behalf trades transcription quality for memory
/// without anyone agreeing to that.
#[test]
fn int4_is_never_chosen_automatically() {
    for caps in [
        class_a_cpu_4core(),
        class_b_rtx_2050(),
        class_c_6gb(),
        class_d_8gb(),
        class_e_24gb(),
    ] {
        for preset in [
            Preset::Auto,
            Preset::Fast,
            Preset::Balanced,
            Preset::Accurate,
        ] {
            let profile = resolve(&caps, preset);
            assert_ne!(profile.asr_precision, Precision::Int4);
            assert!(profile
                .asr_attempts
                .iter()
                .all(|a| a.precision != Precision::Int4));
        }
    }

    // It is still available when asked for explicitly.
    let overrides = ProfileOverrides {
        asr_model: Some("1.7b".into()),
        asr_precision: Some(Precision::Int4),
        ..Default::default()
    };
    let profile = resolve_profile(
        &class_b_rtx_2050(),
        Preset::Custom,
        &overrides,
        &no_measurements,
    );
    assert_eq!(profile.asr_precision, Precision::Int4);
}

#[test]
fn class_d_8gb_prefers_bf16_for_the_larger_model() {
    let profile = resolve(&class_d_8gb(), Preset::Accurate);
    assert_eq!(profile.asr_model, "1.7b");
    // With room to spare, BF16 avoids dequantization overhead entirely.
    assert_eq!(profile.asr_precision, Precision::Bf16);
    // The ladder still lists INT8 below it as a fallback.
    assert!(profile
        .asr_attempts
        .iter()
        .any(|a| a.precision == Precision::Int8));
}

#[test]
fn class_e_24gb_offers_the_full_ladder() {
    let profile = resolve(&class_e_24gb(), Preset::Accurate);
    assert_eq!(profile.asr_model, "1.7b");
    assert_eq!(profile.asr_precision, Precision::Bf16);
    assert_eq!(profile.inference_threads, 31);
    // Both automatically-selectable precisions fit, so the ladder has a real
    // fallback rung. INT4 is excluded by policy, not by memory.
    assert_eq!(
        profile.asr_attempts.len(),
        2,
        "expected BF16 then INT8: {:?}",
        profile.asr_attempts
    );
    assert_eq!(profile.asr_attempts[0].precision, Precision::Bf16);
    assert_eq!(profile.asr_attempts[1].precision, Precision::Int8);
}

#[test]
fn amd_vulkan_machine_runs_asr_on_cpu_and_never_claims_cuda() {
    let caps = amd_vulkan();
    assert!(caps.can_use_vulkan());
    assert!(!caps.can_use_cuda());

    let profile = resolve(&caps, Preset::Balanced);
    // The Python ASR runtime has no Vulkan path, so ASR is CPU here.
    assert_eq!(profile.asr_device, Device::Cpu);
    assert!(profile.has_reason("asr_device_cpu"));
    // Refinement could use Vulkan via llama.cpp, but only after a measured
    // benchmark. Until then it stays on the CPU.
    assert_eq!(profile.refinement_device, Device::Cpu);
    assert!(profile.has_reason("refinement_cpu_no_cuda"));
}

#[test]
fn hybrid_graphics_reserves_less_because_the_igpu_drives_the_display() {
    let hybrid = hybrid_graphics();
    let single = class_b_rtx_2050();
    // Same dGPU, but a second adapter is present, so the floor is lower.
    assert!(
        vram_reserve_mb(&hybrid) < vram_reserve_mb(&single),
        "hybrid {} should reserve less than single {}",
        vram_reserve_mb(&hybrid),
        vram_reserve_mb(&single)
    );
    let profile = resolve(&hybrid, Preset::Balanced);
    assert_eq!(profile.asr_device, Device::Cuda);
}

/// A CPU-only torch wheel on an NVIDIA machine must not silently produce a
/// "GPU" profile.
#[test]
fn gpu_present_but_cuda_unusable_resolves_to_cpu_with_an_explanation() {
    let profile = resolve(&gpu_without_cuda(), Preset::Balanced);
    assert_eq!(profile.asr_device, Device::Cpu);
    assert!(profile.has_reason("asr_device_cpu"));
    let reason = profile
        .reasons
        .iter()
        .find(|r| r.code == "asr_device_cpu")
        .expect("reason");
    assert!(
        reason.detail.contains("CUDA is unavailable"),
        "the explanation must name the real cause: {}",
        reason.detail
    );
}

// ---------------------------------------------------------------------------
// Preset behaviour
// ---------------------------------------------------------------------------

#[test]
fn fast_preset_is_asr_only_and_that_is_not_a_failure() {
    let profile = resolve(&class_e_24gb(), Preset::Fast);
    assert!(profile.is_asr_only());
    assert_eq!(profile.refinement_model, None);
    assert!(profile.has_reason("refinement_off_fast_preset"));
    // Even on a 24 GB card, Fast takes the smallest model for latency.
    assert_eq!(profile.asr_model, "0.6b");
}

#[test]
fn balanced_and_accurate_both_enable_refinement() {
    for preset in [Preset::Balanced, Preset::Accurate, Preset::Auto] {
        let profile = resolve(&class_d_8gb(), preset);
        assert!(!profile.is_asr_only(), "{preset:?} must include refinement");
    }
}

#[test]
fn accurate_preset_selects_the_larger_refinement_model() {
    let balanced = resolve(&class_e_24gb(), Preset::Balanced);
    let accurate = resolve(&class_e_24gb(), Preset::Accurate);
    assert_eq!(balanced.refinement_model.as_deref(), Some("qwen3.5-0.8b"));
    assert_eq!(accurate.refinement_model.as_deref(), Some("qwen3.5-2b"));
}

// ---------------------------------------------------------------------------
// Custom must never be silently overwritten
// ---------------------------------------------------------------------------

#[test]
fn an_explicit_custom_choice_is_never_overwritten() {
    // Deliberately the "wrong" answer for this hardware: 1.7B INT8 on a 4 GB
    // card with the refiner pinned to the GPU. The resolver must honour it.
    let overrides = ProfileOverrides {
        asr_model: Some("1.7b".into()),
        asr_device: Some(Device::Cuda),
        asr_precision: Some(Precision::Int8),
        refinement_model: Some("qwen3.5-2b".into()),
        refinement_device: Some(Device::Cuda),
        refinement_gpu_layers: Some(12),
        language: Some("en".into()),
        force_streaming: Some(true),
    };
    let profile = resolve_profile(
        &class_b_rtx_2050(),
        Preset::Custom,
        &overrides,
        &no_measurements,
    );

    assert_eq!(profile.preset, Preset::Custom);
    assert_eq!(profile.asr_model, "1.7b");
    assert_eq!(profile.asr_device, Device::Cuda);
    assert_eq!(profile.asr_precision, Precision::Int8);
    assert_eq!(profile.refinement_model.as_deref(), Some("qwen3.5-2b"));
    assert_eq!(profile.refinement_device, Device::Cuda);
    assert_eq!(profile.refinement_gpu_layers, 12);
    assert!(profile.streaming_enabled, "an explicit override wins");

    assert!(profile.has_reason("asr_model_user_choice"));
    assert!(profile.has_reason("asr_device_user_choice"));
    assert!(profile.has_reason("refinement_device_user_choice"));
    assert!(profile.has_reason("streaming_forced"));
    assert!(
        !profile.has_reason("asr_model_auto"),
        "the resolver must not have chosen the model"
    );
}

#[test]
fn overrides_are_honoured_under_every_preset_not_just_custom() {
    for preset in [
        Preset::Auto,
        Preset::Fast,
        Preset::Balanced,
        Preset::Accurate,
    ] {
        let overrides = ProfileOverrides {
            asr_model: Some("1.7b".into()),
            ..Default::default()
        };
        let profile = resolve_profile(&class_e_24gb(), preset, &overrides, &no_measurements);
        assert_eq!(
            profile.asr_model, "1.7b",
            "{preset:?} discarded an explicit model choice"
        );
    }
}

#[test]
fn an_explicit_refinement_of_none_turns_it_off_cleanly() {
    let overrides = ProfileOverrides {
        refinement_model: Some("none".into()),
        ..Default::default()
    };
    let profile = resolve_profile(
        &class_e_24gb(),
        Preset::Balanced,
        &overrides,
        &no_measurements,
    );
    assert!(profile.is_asr_only());
    assert!(profile.has_reason("refinement_off_user_choice"));
}

#[test]
fn requesting_cuda_without_cuda_reports_it_instead_of_pretending() {
    let overrides = ProfileOverrides {
        asr_device: Some(Device::Cuda),
        ..Default::default()
    };
    let profile = resolve_profile(
        &gpu_without_cuda(),
        Preset::Custom,
        &overrides,
        &no_measurements,
    );
    assert_eq!(profile.asr_device, Device::Cpu);
    assert!(profile.has_reason("asr_cuda_requested_unavailable"));
    let reason = profile
        .reasons
        .iter()
        .find(|r| r.code == "asr_cuda_requested_unavailable")
        .expect("reason");
    assert!(reason.detail.contains("CPU-only torch build"));
}

// ---------------------------------------------------------------------------
// Measurements override estimates
// ---------------------------------------------------------------------------

#[test]
fn a_measured_peak_overrides_the_estimate() {
    // Measure 0.6B BF16 as much larger than estimated. It must stop fitting.
    let measurements = Measurements::default().with(
        "0.6b",
        Device::Cuda,
        Precision::Bf16,
        MeasuredPeaks {
            vram_mb: Some(3900.0),
            ..Default::default()
        },
    );
    let lookup = measurements.lookup();
    let profile = resolve_profile(
        &class_b_rtx_2050(),
        Preset::Balanced,
        &ProfileOverrides::default(),
        &lookup,
    );
    assert_ne!(
        profile.asr_precision,
        Precision::Bf16,
        "a measured 3.9 GB peak cannot fit a 4 GB card's budget"
    );
    assert!(profile
        .asr_attempts
        .iter()
        .all(|a| a.precision != Precision::Bf16));
}

#[test]
fn measured_attempts_are_marked_as_measured() {
    let measurements = Measurements::default().with(
        "0.6b",
        Device::Cuda,
        Precision::Bf16,
        MeasuredPeaks {
            vram_mb: Some(2100.0),
            rtf: Some(0.28),
            ..Default::default()
        },
    );
    let lookup = measurements.lookup();
    let profile = resolve_profile(
        &class_b_rtx_2050(),
        Preset::Balanced,
        &ProfileOverrides::default(),
        &lookup,
    );
    let chosen = profile
        .asr_attempts
        .iter()
        .find(|a| a.precision == Precision::Bf16)
        .expect("bf16 rung");
    assert!(chosen.measured, "the rung came from a measurement");
    assert_eq!(chosen.expected_peak_mb, 2100.0);
}

/// Streaming is gated on a *measured* RTF, never an assumed one.
#[test]
fn streaming_turns_on_only_below_the_rtf_ceiling() {
    for (rtf, expected) in [(0.30_f32, true), (0.50, true), (0.51, false), (0.80, false)] {
        let measurements = Measurements::default().with(
            "0.6b",
            Device::Cuda,
            Precision::Bf16,
            MeasuredPeaks {
                vram_mb: Some(2100.0),
                rtf: Some(rtf),
                ..Default::default()
            },
        );
        let lookup = measurements.lookup();
        let profile = resolve_profile(
            &class_b_rtx_2050(),
            Preset::Balanced,
            &ProfileOverrides::default(),
            &lookup,
        );
        assert_eq!(
            profile.streaming_enabled, expected,
            "rtf {rtf} should give streaming_enabled={expected}"
        );
    }
}

#[test]
fn refinement_moves_to_the_gpu_only_once_measured_to_fit() {
    // 0.6B BF16 measured at 2.1 GB on a 4 GB card with 3.6 GB free and a
    // ~768 MB reserve leaves too little for the refiner.
    let tight = Measurements::default()
        .with(
            "0.6b",
            Device::Cuda,
            Precision::Bf16,
            MeasuredPeaks {
                vram_mb: Some(2100.0),
                rtf: Some(0.3),
                ..Default::default()
            },
        )
        .with(
            "qwen3.5-0.8b",
            Device::Cuda,
            Precision::Int4,
            MeasuredPeaks {
                vram_mb: Some(1050.0),
                ..Default::default()
            },
        );
    let lookup = tight.lookup();
    let profile = resolve_profile(
        &class_b_rtx_2050(),
        Preset::Balanced,
        &ProfileOverrides::default(),
        &lookup,
    );
    assert_eq!(
        profile.refinement_device,
        Device::Cpu,
        "measured 1050 MB does not fit what remains on a 4 GB card"
    );
    assert!(profile.has_reason("refinement_cpu_measured_no_fit"));

    // The same measurement on an 8 GB card does fit.
    let lookup = tight.lookup();
    let roomy = resolve_profile(
        &class_d_8gb(),
        Preset::Balanced,
        &ProfileOverrides::default(),
        &lookup,
    );
    assert_eq!(roomy.refinement_device, Device::Cuda);
    assert_eq!(roomy.refinement_gpu_layers, 99);
    assert!(roomy.has_reason("refinement_gpu_measured_fit"));
}

// ---------------------------------------------------------------------------
// Invariants that must hold for every class
// ---------------------------------------------------------------------------

#[test]
fn the_resolver_is_deterministic() {
    for caps in [
        class_a_cpu_4core(),
        cpu_only_6core(),
        class_b_rtx_2050(),
        class_c_6gb(),
        class_d_8gb(),
        class_e_24gb(),
        amd_vulkan(),
        hybrid_graphics(),
        gpu_without_cuda(),
    ] {
        for preset in [
            Preset::Auto,
            Preset::Fast,
            Preset::Balanced,
            Preset::Accurate,
        ] {
            let a = resolve(&caps, preset);
            let b = resolve(&caps, preset);
            assert_eq!(a, b, "resolve_profile is not deterministic");
        }
    }
}

#[test]
fn every_class_produces_a_usable_profile_with_a_stated_reason() {
    for (name, caps) in [
        ("cpu-4core", class_a_cpu_4core()),
        ("cpu-6core", cpu_only_6core()),
        ("4gb-cuda", class_b_rtx_2050()),
        ("6gb", class_c_6gb()),
        ("8gb", class_d_8gb()),
        ("24gb", class_e_24gb()),
        ("amd-vulkan", amd_vulkan()),
        ("hybrid", hybrid_graphics()),
        ("gpu-no-cuda", gpu_without_cuda()),
    ] {
        for preset in [
            Preset::Auto,
            Preset::Fast,
            Preset::Balanced,
            Preset::Accurate,
        ] {
            let profile = resolve(&caps, preset);
            assert!(
                asr_manifest(&profile.asr_model).is_some(),
                "{name}/{preset:?}: unknown ASR model {}",
                profile.asr_model
            );
            if let Some(id) = &profile.refinement_model {
                assert!(
                    refinement_manifest(id).is_some(),
                    "{name}/{preset:?}: unknown refinement model {id}"
                );
            }
            assert!(
                profile.inference_threads >= 1
                    && profile.inference_threads <= caps.cpu.logical_cores,
                "{name}/{preset:?}: {} threads for {} cores",
                profile.inference_threads,
                caps.cpu.logical_cores
            );
            assert!(
                !profile.reasons.is_empty(),
                "{name}/{preset:?}: no explanation for the chosen profile"
            );
            // A CPU profile must never claim GPU layers.
            if profile.refinement_device == Device::Cpu {
                assert_eq!(profile.refinement_gpu_layers, 0, "{name}/{preset:?}");
            }
            // ASR must never be assigned to a device the machine cannot use.
            if profile.asr_device == Device::Cuda {
                assert!(caps.can_use_cuda(), "{name}/{preset:?}: CUDA without CUDA");
            }
        }
    }
}

#[test]
fn the_vram_reserve_is_a_floor_on_measured_free_not_a_share_of_nominal_total() {
    // A busy 4 GB card: 800 MB free. The reserve must not scale with how little
    // is free, and must never exceed half the device.
    let busy = with_cuda(Capabilities {
        gpus: vec![nvidia("NVIDIA GeForce RTX 2050", 4096.0, 800.0)],
        cpu: cpu("AMD Ryzen 5 5600H", 6, 12),
        ram: ram(16.0, 9.0),
        ..Default::default()
    });
    let idle = class_b_rtx_2050();
    assert_eq!(
        vram_reserve_mb(&busy),
        vram_reserve_mb(&idle),
        "the reserve depends on the device, not on current free memory"
    );
    assert!(vram_reserve_mb(&busy) <= 4096.0 * 0.5);

    // No GPU means nothing to reserve.
    assert_eq!(vram_reserve_mb(&class_a_cpu_4core()), 0.0);

    // A large card reserves proportionally more than the flat floor.
    assert!(
        vram_reserve_mb(&class_e_24gb()) > 768.0,
        "a 24 GB card driving big monitors needs more than the floor"
    );
}

#[test]
fn a_busy_gpu_degrades_rather_than_over_admitting() {
    // Only 800 MB free: nothing fits on the GPU once the reserve is applied.
    let busy = with_cuda(Capabilities {
        gpus: vec![nvidia("NVIDIA GeForce RTX 2050", 4096.0, 800.0)],
        cpu: cpu("AMD Ryzen 5 5600H", 6, 12),
        ram: ram(16.0, 9.0),
        ..Default::default()
    });
    let profile = resolve(&busy, Preset::Balanced);
    assert!(
        profile.asr_attempts.is_empty() || profile.asr_attempts[0].device == Device::Cpu,
        "a nearly-full GPU must not be handed a model: {:?}",
        profile.asr_attempts
    );
    assert!(profile.has_reason("asr_no_viable_precision"));
}

#[test]
fn profiles_round_trip_through_json() {
    let profile = resolve(&class_b_rtx_2050(), Preset::Balanced);
    let json = serde_json::to_string(&profile).expect("serialize");
    let back: ResolvedProfile = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(profile, back);
}

#[test]
fn preset_parsing_is_tolerant_but_never_silently_becomes_custom() {
    assert_eq!(Preset::parse("fast"), Preset::Fast);
    assert_eq!(Preset::parse("BALANCED"), Preset::Balanced);
    assert_eq!(Preset::parse("Accurate"), Preset::Accurate);
    assert_eq!(Preset::parse("custom"), Preset::Custom);
    // An unknown value must default to Auto, not Custom: silently treating it as
    // Custom would freeze whatever configuration happened to be stored.
    assert_eq!(Preset::parse("nonsense"), Preset::Auto);
    assert_eq!(Preset::parse(""), Preset::Auto);
}
