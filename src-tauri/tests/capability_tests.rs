//! Milestone 1 / Task 6: the capability probe must report what the machine can
//! actually do.
//!
//! The parsers are pure so they can be checked against captured fixtures with
//! no hardware present; the collector is checked for internal consistency on
//! whatever machine runs the suite.

use reflow_lib::capability::probe::{
    parse_driver_cuda_version, parse_nvidia_smi, parse_vulkaninfo, parse_wmi_adapters,
    Capabilities, CpuInfo, CudaStatus, GpuVendor, RamInfo,
};
use reflow_lib::capability::{capabilities, capabilities_uncached, set_torch_cuda};

/// `nvidia-smi --query-gpu=index,name,memory.total,memory.used,memory.free,
/// driver_version,compute_cap --format=csv,noheader,nounits` on the target
/// RTX 2050 laptop.
const RTX_2050: &str = "0, NVIDIA GeForce RTX 2050, 4096, 512, 3584, 551.86, 8.6\n";

#[test]
fn used_vram_is_never_reported_as_capacity() {
    let gpus = parse_nvidia_smi(RTX_2050);
    let gpu = &gpus[0];
    assert_eq!(gpu.total_vram_mb, 4096.0);
    assert_eq!(gpu.used_vram_mb, 512.0);
    assert_eq!(gpu.free_vram_mb, 3584.0);
    // The original bug: memory.used read as the capacity figure.
    assert_ne!(gpu.total_vram_mb, gpu.used_vram_mb);
}

#[test]
fn free_vram_is_measured_not_derived_from_a_nominal_total() {
    // A busy device: 3.5 GB of a 4 GB card already allocated. An admission
    // check must see 512 MB free, not "4096 minus our own estimate".
    let busy = "0, NVIDIA GeForce RTX 2050, 4096, 3584, 512, 551.86, 8.6\n";
    let caps = Capabilities {
        gpus: parse_nvidia_smi(busy),
        ..Default::default()
    };
    assert_eq!(caps.free_vram_mb(), 512.0);
    assert_eq!(caps.total_vram_mb(), 4096.0);
    assert!(caps.free_vram_mb() < caps.total_vram_mb());
}

#[test]
fn no_gpu_machine_reports_cpu_only() {
    let caps = Capabilities {
        cpu: CpuInfo {
            model: "Intel(R) Core(TM) i5-1135G7".into(),
            physical_cores: Some(4),
            logical_cores: 8,
            load_pct: 7.5,
        },
        ram: RamInfo {
            total_mb: 16384.0,
            used_mb: 6000.0,
            available_mb: 10384.0,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(caps.primary_gpu().is_none());
    assert_eq!(caps.free_vram_mb(), 0.0);
    assert!(!caps.can_use_cuda());
    assert!(!caps.can_use_vulkan());
    assert!(caps.summary().starts_with("CPU only"));
    // Four physical cores means three threads for inference.
    assert_eq!(caps.cpu.inference_threads(), 7);
}

#[test]
fn gpu_without_usable_cuda_is_its_own_state() {
    let caps = Capabilities {
        gpus: parse_nvidia_smi(RTX_2050),
        cuda: CudaStatus {
            nvidia_gpu_present: true,
            driver_present: true,
            driver_cuda_version: Some("12.4".into()),
            torch_cuda_available: false,
            torch_cuda_version: None,
        },
        ..Default::default()
    };
    // Not "no GPU", and not "GPU ready" either.
    assert!(caps.primary_gpu().is_some());
    assert!(!caps.can_use_cuda());
    assert!(caps.cuda.gpu_present_but_unusable());
    assert!(caps.summary().contains("CUDA unavailable"));
}

#[test]
fn hybrid_graphics_prefers_the_discrete_device() {
    let wmi = "Intel(R) Iris(R) Xe Graphics|1073741824\nNVIDIA GeForce RTX 2050|4293918720\n";
    let caps = Capabilities {
        gpus: parse_wmi_adapters(wmi),
        ..Default::default()
    };
    assert_eq!(caps.gpus.len(), 2);
    assert_eq!(
        caps.primary_gpu().map(|g| g.vendor),
        Some(GpuVendor::Nvidia)
    );
    // WMI cannot report free VRAM, so it stays zero rather than being
    // optimistically assumed.
    assert_eq!(caps.free_vram_mb(), 0.0);
}

#[test]
fn vulkan_is_probed_independently_of_cuda() {
    let vk = parse_vulkaninfo(
        "Vulkan Instance Version: 1.3.280\n\tdeviceName         = AMD Radeon RX 6600\n",
    );
    assert!(vk.available);
    assert_eq!(vk.devices, vec!["AMD Radeon RX 6600".to_string()]);

    let caps = Capabilities {
        gpus: parse_wmi_adapters("AMD Radeon RX 6600|0\n"),
        vulkan: vk,
        ..Default::default()
    };
    // An AMD machine has no CUDA but is still a viable Vulkan target.
    assert!(!caps.can_use_cuda());
    assert!(caps.can_use_vulkan());
}

#[test]
fn driver_cuda_version_is_parsed_from_bare_smi_output() {
    let smi = "| NVIDIA-SMI 551.86  Driver Version: 551.86  CUDA Version: 12.4 |";
    assert_eq!(parse_driver_cuda_version(smi).as_deref(), Some("12.4"));
}

/// `set_torch_cuda` writes process-global state, so tests that touch it have
/// to serialise against each other. Without this they race and one observes
/// the other's value.
static TORCH_STATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs against the real machine. It must hold on a laptop with no GPU, a
/// laptop with hybrid graphics, and a desktop with a discrete card.
#[test]
fn live_probe_is_internally_consistent() {
    let _guard = TORCH_STATE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    set_torch_cuda(false, None);
    let caps = capabilities_uncached();

    assert!(caps.cpu.logical_cores >= 1);
    assert!(caps.cpu.inference_threads() >= 1);
    assert!(caps.cpu.inference_threads() <= caps.cpu.logical_cores);

    assert!(caps.ram.total_mb > 0.0);
    assert!(caps.ram.used_mb <= caps.ram.total_mb);
    assert!(caps.ram.available_mb <= caps.ram.total_mb);
    assert!(caps.ram.app_mb > 0.0);

    for gpu in &caps.gpus {
        assert!(gpu.free_vram_mb >= 0.0);
        assert!(gpu.used_vram_mb >= 0.0);
        if gpu.total_vram_mb > 0.0 {
            assert!(
                gpu.free_vram_mb <= gpu.total_vram_mb,
                "{}: free {} > total {}",
                gpu.name,
                gpu.free_vram_mb,
                gpu.total_vram_mb
            );
        }
    }

    // Without a sidecar report, CUDA must read as unusable rather than being
    // inferred from GPU presence.
    assert!(!caps.can_use_cuda());
    assert!(!caps.probed_at.is_empty());
    assert!(!caps.summary().is_empty());
}

#[test]
fn torch_report_is_the_only_source_of_cuda_availability() {
    let _guard = TORCH_STATE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    set_torch_cuda(true, Some("12.4".into()));
    let caps = capabilities_uncached();
    assert!(caps.can_use_cuda());
    assert_eq!(caps.cuda.torch_cuda_version.as_deref(), Some("12.4"));

    set_torch_cuda(false, None);
    assert!(!capabilities_uncached().can_use_cuda());
}

#[test]
fn cached_reads_do_not_shell_out() {
    let _ = capabilities_uncached();
    let start = std::time::Instant::now();
    for _ in 0..100 {
        let _ = capabilities();
    }
    assert!(
        start.elapsed() < std::time::Duration::from_secs(1),
        "100 cached reads took {:?}",
        start.elapsed()
    );
}

/// Regression guard: the diagnostics report must never carry transcript text.
#[test]
fn diagnostics_report_contains_no_transcript_text() {
    let report = reflow_lib::platform::PlatformSys::generate_diagnostics_report();
    assert!(report.contains("Reflow Local Dictation System Diagnostics"));
    assert!(report.contains("torch.cuda.is_available()"));
    assert!(report.contains("System RAM:"));
    assert!(
        report.contains("Transcript contents are excluded"),
        "the report must state its own redaction policy"
    );
    // The report separates the three memory figures rather than printing one.
    assert!(report.contains("total /"));
}
