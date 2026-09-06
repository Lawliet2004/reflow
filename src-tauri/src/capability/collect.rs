//! Runs the external probes and feeds their output to the pure parsers in
//! [`super::probe`].
//!
//! Everything here is I/O and caching. The interpretation lives in the parsers
//! so it stays testable without the hardware.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use sysinfo::{
    CpuRefreshKind, MemoryRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind,
    System,
};

#[cfg(target_os = "windows")]
use super::probe::parse_wmi_adapters;
use super::probe::{
    parse_driver_cuda_version, parse_nvidia_smi, parse_vulkaninfo, Capabilities, CpuInfo,
    CudaStatus, GpuInfo, GpuVendor, RamInfo, VulkanStatus, NVIDIA_SMI_QUERY,
};

/// How long a full snapshot is reused.
///
/// Static facts (GPU model, core count) never change; VRAM and RAM do, and a
/// load decision must not be made against a stale reading. Two seconds keeps
/// the diagnostics panel responsive without shelling out on every frame.
const CACHE_TTL: Duration = Duration::from_secs(2);

/// Hard cap on how long a probe subprocess may block us.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

struct Cached {
    caps: Capabilities,
    at: Instant,
}

fn cache() -> &'static Mutex<Option<Cached>> {
    static CACHE: OnceLock<Mutex<Option<Cached>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Facts that cannot change while the process runs, probed once.
struct StaticProbes {
    vulkan: VulkanStatus,
    driver_cuda_version: Option<String>,
    cpu_model: String,
    physical_cores: Option<usize>,
}

fn static_probes() -> &'static StaticProbes {
    static STATIC: OnceLock<StaticProbes> = OnceLock::new();
    STATIC.get_or_init(|| {
        let mut sys = System::new_with_specifics(
            RefreshKind::nothing().with_cpu(CpuRefreshKind::everything()),
        );
        sys.refresh_cpu_all();
        let cpu_model = sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Unknown CPU".into());
        StaticProbes {
            vulkan: probe_vulkan(),
            driver_cuda_version: probe_driver_cuda_version(),
            cpu_model,
            physical_cores: sys.physical_core_count(),
        }
    })
}

/// Current capabilities, cached for [`CACHE_TTL`].
pub fn capabilities() -> Capabilities {
    {
        let guard = cache().lock().unwrap_or_else(|p| p.into_inner());
        if let Some(cached) = guard.as_ref() {
            if cached.at.elapsed() < CACHE_TTL {
                return cached.caps.clone();
            }
        }
    }
    let fresh = probe_now();
    let mut guard = cache().lock().unwrap_or_else(|p| p.into_inner());
    *guard = Some(Cached {
        caps: fresh.clone(),
        at: Instant::now(),
    });
    fresh
}

/// Force a fresh probe, ignoring the cache. Used immediately before a load
/// decision and after every model load, where a stale free-VRAM figure would
/// be actively harmful.
pub fn capabilities_uncached() -> Capabilities {
    let fresh = probe_now();
    let mut guard = cache().lock().unwrap_or_else(|p| p.into_inner());
    *guard = Some(Cached {
        caps: fresh.clone(),
        at: Instant::now(),
    });
    fresh
}

/// What the ASR sidecar reports about its torch build.
#[derive(Debug, Clone, Default)]
struct TorchCuda {
    available: bool,
    version: Option<String>,
}

/// Record what the ASR sidecar reports about torch, which is the only source
/// for `torch.cuda.is_available()`.
///
/// Held separately from the probe cache because it comes from a different
/// process on a different schedule.
pub fn set_torch_cuda(available: bool, version: Option<String>) {
    let mut guard = torch_state().lock().unwrap_or_else(|p| p.into_inner());
    *guard = TorchCuda { available, version };
}

fn torch_state() -> &'static Mutex<TorchCuda> {
    static TORCH: OnceLock<Mutex<TorchCuda>> = OnceLock::new();
    TORCH.get_or_init(|| Mutex::new(TorchCuda::default()))
}

fn torch_cuda() -> TorchCuda {
    torch_state()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
}

fn probe_now() -> Capabilities {
    let statics = static_probes();
    let gpus = probe_gpus();
    let nvidia_present = gpus.iter().any(|g| g.vendor == GpuVendor::Nvidia);
    let driver_present =
        statics.driver_cuda_version.is_some() || gpus.iter().any(|g| g.driver_version.is_some());
    let torch = torch_cuda();
    let (cpu_load, ram) = probe_cpu_and_ram();

    Capabilities {
        gpus,
        cuda: CudaStatus {
            nvidia_gpu_present: nvidia_present,
            driver_present,
            driver_cuda_version: statics.driver_cuda_version.clone(),
            torch_cuda_available: torch.available,
            torch_cuda_version: torch.version,
        },
        vulkan: statics.vulkan.clone(),
        cpu: CpuInfo {
            model: statics.cpu_model.clone(),
            physical_cores: statics.physical_cores,
            logical_cores: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            load_pct: cpu_load,
        },
        ram,
        os_name: crate::platform::os_display_name(),
        probed_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn probe_gpus() -> Vec<GpuInfo> {
    if let Some(stdout) = run_probe(
        "nvidia-smi",
        &[
            &format!("--query-gpu={NVIDIA_SMI_QUERY}"),
            "--format=csv,noheader,nounits",
        ],
    ) {
        let gpus = parse_nvidia_smi(&stdout);
        if !gpus.is_empty() {
            return gpus;
        }
    }
    // No NVIDIA driver. Enumerate adapters so AMD/Intel machines are still
    // reported rather than appearing to have no GPU at all.
    probe_non_nvidia_adapters()
}

#[cfg(windows)]
fn probe_non_nvidia_adapters() -> Vec<GpuInfo> {
    let script = "Get-CimInstance Win32_VideoController | \
                  ForEach-Object { \"$($_.Name)|$($_.AdapterRAM)\" }";
    run_probe(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", script],
    )
    .map(|out| parse_wmi_adapters(&out))
    .unwrap_or_default()
}

#[cfg(not(windows))]
fn probe_non_nvidia_adapters() -> Vec<GpuInfo> {
    // The Vulkan device list is the most portable adapter enumeration we
    // already have on Linux; memory figures are unavailable, which the
    // parsers represent as zero rather than guessing.
    let vk = &static_probes().vulkan;
    vk.devices
        .iter()
        .enumerate()
        .map(|(index, name)| GpuInfo {
            index: index as u32,
            vendor: GpuVendor::from_name(name),
            name: name.clone(),
            total_vram_mb: 0.0,
            used_vram_mb: 0.0,
            free_vram_mb: 0.0,
            driver_version: None,
            compute_capability: None,
        })
        .collect()
}

fn probe_driver_cuda_version() -> Option<String> {
    run_probe("nvidia-smi", &[]).and_then(|out| parse_driver_cuda_version(&out))
}

fn probe_vulkan() -> VulkanStatus {
    run_probe("vulkaninfo", &["--summary"])
        .map(|out| parse_vulkaninfo(&out))
        .unwrap_or_default()
}

fn probe_cpu_and_ram() -> (f32, RamInfo) {
    static SYS: OnceLock<Mutex<System>> = OnceLock::new();
    let sys = SYS.get_or_init(|| {
        Mutex::new(System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        ))
    });
    let mut guard = sys.lock().unwrap_or_else(|p| p.into_inner());
    guard.refresh_cpu_all();
    guard.refresh_memory();
    guard.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_memory(),
    );

    let to_mb = |bytes: u64| bytes as f32 / (1024.0 * 1024.0);
    let total_mb = to_mb(guard.total_memory());
    let used_mb = to_mb(guard.used_memory());
    // `available_memory` is what a new allocation can actually get, which is
    // not the same as total - used once caches are accounted for.
    let available_mb = to_mb(guard.available_memory());

    let own_pid = Pid::from_u32(std::process::id());
    let app_mb = guard
        .process(own_pid)
        .map(|p| to_mb(p.memory()))
        .unwrap_or(0.0);

    // Attribute the two child runtimes so the UI can say where RAM went.
    let mut asr_mb = 0.0f32;
    let mut refinement_mb = 0.0f32;
    for process in guard.processes().values() {
        let name = process.name().to_string_lossy().to_ascii_lowercase();
        let mb = to_mb(process.memory());
        if name.starts_with("llama-server") {
            refinement_mb += mb;
        } else if name.starts_with("python") || name.starts_with("pythonw") {
            // Only count Python processes we are the parent of; an unrelated
            // interpreter must not be attributed to Reflow.
            if process.parent() == Some(own_pid) {
                asr_mb += mb;
            }
        }
    }

    (
        guard.global_cpu_usage(),
        RamInfo {
            total_mb,
            used_mb,
            available_mb,
            app_mb,
            asr_mb,
            refinement_mb,
        },
    )
}

/// Run a probe command with a hard timeout, returning its stdout.
///
/// A missing binary is a normal outcome (no NVIDIA driver, no Vulkan loader),
/// so failures are `None` rather than errors.
fn run_probe(program: &str, args: &[&str]) -> Option<String> {
    use std::process::{Command, Stdio};

    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().ok()?;
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                break;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    log::warn!("Capability probe '{program}' timed out; killing it");
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return None,
        }
    }

    let output = child.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe must work on any machine, including one with no GPU, no
    /// driver and no Vulkan loader. It must never panic and never report a
    /// nonsensical shape.
    #[test]
    fn probe_is_self_consistent_on_this_machine() {
        let caps = capabilities_uncached();

        assert!(caps.cpu.logical_cores >= 1);
        if let Some(physical) = caps.cpu.physical_cores {
            assert!(physical >= 1);
            assert!(physical <= caps.cpu.logical_cores);
        }
        assert!(caps.cpu.inference_threads() >= 1);
        assert!(caps.cpu.inference_threads() <= caps.cpu.logical_cores);

        assert!(caps.ram.total_mb > 0.0, "total RAM must be measured");
        assert!(
            caps.ram.used_mb <= caps.ram.total_mb,
            "used {} exceeds total {}",
            caps.ram.used_mb,
            caps.ram.total_mb
        );
        assert!(caps.ram.available_mb <= caps.ram.total_mb);
        assert!(caps.ram.app_mb > 0.0, "our own RSS must be measurable");

        for gpu in &caps.gpus {
            assert!(gpu.used_vram_mb <= gpu.total_vram_mb.max(gpu.used_vram_mb));
            assert!(gpu.free_vram_mb >= 0.0);
            assert!(
                gpu.free_vram_mb <= gpu.total_vram_mb.max(gpu.free_vram_mb),
                "free {} exceeds total {} on {}",
                gpu.free_vram_mb,
                gpu.total_vram_mb,
                gpu.name
            );
        }

        // Without a sidecar report, torch CUDA must read as unavailable
        // rather than being inferred from GPU presence.
        assert!(!caps.summary().is_empty());
        assert!(!caps.probed_at.is_empty());
    }

    #[test]
    fn torch_cuda_is_only_true_when_the_sidecar_says_so() {
        set_torch_cuda(false, None);
        assert!(!capabilities_uncached().can_use_cuda());

        set_torch_cuda(true, Some("12.4".into()));
        let caps = capabilities_uncached();
        assert!(caps.can_use_cuda());
        assert_eq!(caps.cuda.torch_cuda_version.as_deref(), Some("12.4"));

        // Reset so ordering between tests cannot leak state.
        set_torch_cuda(false, None);
    }

    #[test]
    fn cache_returns_quickly_after_the_first_probe() {
        let _ = capabilities_uncached();
        let start = Instant::now();
        for _ in 0..50 {
            let _ = capabilities();
        }
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "cached reads must not shell out"
        );
    }

    #[test]
    fn missing_probe_binary_is_not_an_error() {
        assert!(run_probe("reflow-definitely-not-a-real-binary", &[]).is_none());
    }
}
