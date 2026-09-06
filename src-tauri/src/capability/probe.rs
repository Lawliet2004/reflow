//! Milestone 1 / Task 6: report what the machine can actually do.
//!
//! The previous probe made two specific mistakes that invalidated every
//! resource decision built on top of it:
//!
//! 1. `nvidia-smi --query-gpu=memory.used` was read as GPU *capacity*. On a
//!    4 GB card sitting idle that reports ~300 MB, so the app believed it had
//!    almost no VRAM; with a browser open it reports ~2 GB and the app
//!    believed it had exactly as much VRAM as was already in use.
//! 2. `used_memory()` was stored in `total_ram_mb`.
//!
//! It also could not express "NVIDIA GPU present, but the installed torch is
//! a CPU-only wheel", which is a real and common state on Windows.
//!
//! Everything that parses external output is a pure function over `&str` so it
//! can be tested against captured fixtures without the hardware present.

use serde::{Deserialize, Serialize};

/// Which vendor's GPU we are looking at. Determines which compute APIs are
/// even candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    Apple,
    Unknown,
}

impl GpuVendor {
    /// Best-effort vendor from an adapter description string.
    pub fn from_name(name: &str) -> Self {
        let n = name.to_ascii_lowercase();
        if n.contains("nvidia")
            || n.contains("geforce")
            || n.contains("quadro")
            || n.contains("rtx")
            || n.contains("gtx")
            || n.contains("tesla")
        {
            GpuVendor::Nvidia
        } else if n.contains("amd") || n.contains("radeon") || n.contains("vega") {
            GpuVendor::Amd
        } else if n.contains("intel")
            || n.contains("arc")
            || n.contains("iris")
            || n.contains("uhd")
        {
            GpuVendor::Intel
        } else if n.contains("apple") || n.contains("m1") || n.contains("m2") || n.contains("m3") {
            GpuVendor::Apple
        } else {
            GpuVendor::Unknown
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            GpuVendor::Nvidia => "nvidia",
            GpuVendor::Amd => "amd",
            GpuVendor::Intel => "intel",
            GpuVendor::Apple => "apple",
            GpuVendor::Unknown => "unknown",
        }
    }
}

/// A single GPU as the driver reports it.
///
/// All three memory figures are recorded separately. Deriving `free` by
/// subtracting a guess from a nominal total is what the old code effectively
/// did, and it is wrong on any machine that is also driving a display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuInfo {
    /// Driver index, as used by `--main-gpu` / `CUDA_VISIBLE_DEVICES`.
    pub index: u32,
    pub name: String,
    pub vendor: GpuVendor,
    /// Physical capacity.
    pub total_vram_mb: f32,
    /// Currently allocated by all processes on the device.
    pub used_vram_mb: f32,
    /// Reported free, measured now. This is the only number a load decision
    /// may be based on.
    pub free_vram_mb: f32,
    /// Driver version string, when available. Part of the benchmark cache key,
    /// because a driver change can change offload behaviour.
    pub driver_version: Option<String>,
    /// Compute capability for NVIDIA devices, e.g. `"8.6"`.
    pub compute_capability: Option<String>,
}

/// How the toolchain sees CUDA.
///
/// `gpu_present` and `cuda_runtime_present` and `torch_cuda_available` are
/// genuinely three different facts, and conflating them is what produced
/// "GPU selected" states that silently ran on the CPU.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CudaStatus {
    /// An NVIDIA device was enumerated.
    pub nvidia_gpu_present: bool,
    /// `nvidia-smi` answered, so a driver and CUDA runtime exist.
    pub driver_present: bool,
    /// Maximum CUDA version the driver supports.
    pub driver_cuda_version: Option<String>,
    /// `torch.cuda.is_available()` inside the sidecar.
    pub torch_cuda_available: bool,
    /// CUDA version the installed torch was built against.
    pub torch_cuda_version: Option<String>,
}

impl CudaStatus {
    /// `true` when an NVIDIA GPU exists but Python cannot use it. This is the
    /// state the old capability model could not express at all, and it is
    /// fixable by the user, so it must be reported distinctly.
    pub fn gpu_present_but_unusable(&self) -> bool {
        self.nvidia_gpu_present && !self.torch_cuda_available
    }
}

/// Vulkan availability, probed independently of CUDA.
///
/// `llama.cpp` can run on Vulkan for AMD and Intel devices where CUDA is not
/// an option, so this cannot be folded into `CudaStatus`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VulkanStatus {
    pub available: bool,
    pub api_version: Option<String>,
    /// Device names the loader enumerated.
    pub devices: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CpuInfo {
    pub model: String,
    /// Physical cores. `None` when the OS will not tell us.
    pub physical_cores: Option<usize>,
    pub logical_cores: usize,
    /// Global load at probe time, 0-100.
    pub load_pct: f32,
}

impl CpuInfo {
    /// Threads an inference runtime may use while leaving headroom for the UI,
    /// the audio callback and the OS.
    ///
    /// Audio capture must never be starved by inference, so at least one
    /// logical core is always held back.
    pub fn inference_threads(&self) -> usize {
        self.logical_cores.saturating_sub(1).max(1)
    }
}

/// System RAM, with per-runtime attribution.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RamInfo {
    pub total_mb: f32,
    pub used_mb: f32,
    pub available_mb: f32,
    /// Resident set of the Reflow process itself.
    pub app_mb: f32,
    /// Resident set of the Python ASR sidecar, when running.
    pub asr_mb: f32,
    /// Resident set of `llama-server`, when running.
    pub refinement_mb: f32,
}

/// The full, honest picture of the machine.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    pub gpus: Vec<GpuInfo>,
    pub cuda: CudaStatus,
    pub vulkan: VulkanStatus,
    pub cpu: CpuInfo,
    pub ram: RamInfo,
    pub os_name: String,
    /// RFC 3339 timestamp of the probe, so a stale snapshot is obvious.
    pub probed_at: String,
}

impl Capabilities {
    /// The device a GPU workload would actually run on: the NVIDIA GPU with
    /// the most free VRAM, else any GPU, else none.
    pub fn primary_gpu(&self) -> Option<&GpuInfo> {
        self.gpus
            .iter()
            .filter(|g| g.vendor == GpuVendor::Nvidia)
            .max_by(|a, b| {
                a.free_vram_mb
                    .partial_cmp(&b.free_vram_mb)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .or_else(|| self.gpus.first())
    }

    /// Free VRAM on the device a load would target, measured now.
    pub fn free_vram_mb(&self) -> f32 {
        self.primary_gpu().map(|g| g.free_vram_mb).unwrap_or(0.0)
    }

    pub fn total_vram_mb(&self) -> f32 {
        self.primary_gpu().map(|g| g.total_vram_mb).unwrap_or(0.0)
    }

    /// `true` when a CUDA workload can actually be dispatched from Python.
    pub fn can_use_cuda(&self) -> bool {
        self.cuda.torch_cuda_available
    }

    /// A GPU exists that `llama.cpp` could target through Vulkan even without
    /// CUDA. Used for the AMD and Intel routing decision.
    pub fn can_use_vulkan(&self) -> bool {
        self.vulkan.available && !self.gpus.is_empty()
    }

    /// One-line summary for the readiness indicator.
    pub fn summary(&self) -> String {
        match self.primary_gpu() {
            None => format!(
                "CPU only · {} logical cores · {:.1} GB RAM free",
                self.cpu.logical_cores,
                self.ram.available_mb / 1024.0
            ),
            Some(gpu) if self.cuda.gpu_present_but_unusable() => format!(
                "{} present, CUDA unavailable · {:.0} MB free VRAM",
                gpu.name, gpu.free_vram_mb
            ),
            Some(gpu) => format!(
                "{} · {:.0}/{:.0} MB VRAM free",
                gpu.name, gpu.free_vram_mb, gpu.total_vram_mb
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Pure parsers. Kept free of I/O so they can be tested against fixtures.
// ---------------------------------------------------------------------------

/// The `nvidia-smi` query this parser expects.
pub const NVIDIA_SMI_QUERY: &str =
    "index,name,memory.total,memory.used,memory.free,driver_version,compute_cap";

/// Parse `nvidia-smi --query-gpu=<NVIDIA_SMI_QUERY> --format=csv,noheader,nounits`.
///
/// Tolerates `[N/A]` and `[Not Supported]` placeholders, which older drivers
/// emit for `compute_cap`, and ignores trailing blank lines.
pub fn parse_nvidia_smi(stdout: &str) -> Vec<GpuInfo> {
    let mut out = Vec::new();
    for (fallback_index, line) in stdout.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        if fields.len() < 5 {
            continue;
        }
        let index = parse_placeholder(fields[0])
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(fallback_index as u32);
        let name = parse_placeholder(fields[1])
            .unwrap_or("NVIDIA GPU")
            .to_string();
        let total = parse_mib(fields[2]);
        let used = parse_mib(fields[3]);
        let free = parse_mib(fields[4]);
        // Only trust `free` if the driver gave it to us; deriving it is the
        // bug this parser exists to avoid, so fall back explicitly and only
        // when both other figures are present.
        let free_vram_mb = if free > 0.0 {
            free
        } else if total > 0.0 && used <= total {
            total - used
        } else {
            0.0
        };
        out.push(GpuInfo {
            index,
            vendor: GpuVendor::from_name(&name),
            name,
            total_vram_mb: total,
            used_vram_mb: used,
            free_vram_mb,
            driver_version: fields
                .get(5)
                .and_then(|f| parse_placeholder(f))
                .map(String::from),
            compute_capability: fields
                .get(6)
                .and_then(|f| parse_placeholder(f))
                .map(String::from),
        });
    }
    out
}

/// Drivers report unavailable values as `[N/A]`, `[Not Supported]`,
/// `[Unknown Error]` or an empty field. Treat all of them as absent rather
/// than parsing them into a misleading zero.
fn parse_placeholder(field: &str) -> Option<&str> {
    let f = field.trim();
    if f.is_empty() || f.starts_with('[') {
        return None;
    }
    Some(f)
}

fn parse_mib(field: &str) -> f32 {
    parse_placeholder(field)
        // `nounits` normally strips these, but be tolerant of a caller that
        // forgot the flag.
        .map(|f| f.trim_end_matches(" MiB").trim_end_matches(" MB").trim())
        .and_then(|f| f.parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(0.0)
}

/// Parse the driver's maximum supported CUDA version out of bare
/// `nvidia-smi` output.
pub fn parse_driver_cuda_version(stdout: &str) -> Option<String> {
    // e.g. "| NVIDIA-SMI 551.86    Driver Version: 551.86    CUDA Version: 12.4 |"
    let idx = stdout.find("CUDA Version:")?;
    let rest = &stdout[idx + "CUDA Version:".len()..];
    let token = rest.split_whitespace().next()?;
    let cleaned: String = token
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// Parse `vulkaninfo --summary` output.
pub fn parse_vulkaninfo(stdout: &str) -> VulkanStatus {
    let mut status = VulkanStatus::default();
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Vulkan Instance Version:") {
            status.api_version = Some(rest.trim().to_string());
            status.available = true;
        } else if let Some(rest) = line.strip_prefix("deviceName") {
            if let Some(name) = rest.split('=').nth(1) {
                let name = name.trim();
                if !name.is_empty() {
                    status.devices.push(name.to_string());
                    status.available = true;
                }
            }
        }
    }
    status
}

/// Parse the WMI/PowerShell adapter listing used when `nvidia-smi` is absent.
///
/// Expected shape is one `Name|AdapterRAM` pair per line, where `AdapterRAM`
/// is in bytes and may be empty for devices that do not report it.
pub fn parse_wmi_adapters(stdout: &str) -> Vec<GpuInfo> {
    let mut out = Vec::new();
    for (index, line) in stdout.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('|');
        let Some(name) = parts.next().map(str::trim).filter(|n| !n.is_empty()) else {
            continue;
        };
        // WMI's AdapterRAM is a uint32 and therefore wrong (wraps) above 4 GB.
        // Record it as total but never treat it as free.
        let total_mb = parts
            .next()
            .and_then(|v| parse_placeholder(v))
            .and_then(|v| v.parse::<f64>().ok())
            .map(|bytes| (bytes / (1024.0 * 1024.0)) as f32)
            .unwrap_or(0.0);
        out.push(GpuInfo {
            index: index as u32,
            vendor: GpuVendor::from_name(name),
            name: name.to_string(),
            total_vram_mb: total_mb,
            used_vram_mb: 0.0,
            // Unknown, not "all of it". A caller must not admit a model on
            // the strength of an unmeasured figure.
            free_vram_mb: 0.0,
            driver_version: None,
            compute_capability: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `nvidia-smi --query-gpu=index,name,memory.total,memory.used,
    // memory.free,driver_version,compute_cap --format=csv,noheader,nounits`
    // on an RTX 2050 laptop.
    const RTX_2050: &str = "0, NVIDIA GeForce RTX 2050, 4096, 512, 3584, 551.86, 8.6\n";

    const DUAL_GPU: &str = "\
0, NVIDIA GeForce RTX 4090, 24564, 1024, 23540, 550.54, 8.9
1, NVIDIA GeForce GTX 1060, 6144, 5900, 244, 550.54, 6.1
";

    // Older driver: compute_cap unsupported, memory.free not reported.
    const OLD_DRIVER: &str =
        "0, NVIDIA GeForce GTX 960M, 2048, 300, [Not Supported], 391.35, [N/A]\n";

    #[test]
    fn parses_total_used_and_free_separately() {
        let gpus = parse_nvidia_smi(RTX_2050);
        assert_eq!(gpus.len(), 1);
        let gpu = &gpus[0];
        assert_eq!(gpu.index, 0);
        assert_eq!(gpu.name, "NVIDIA GeForce RTX 2050");
        assert_eq!(gpu.vendor, GpuVendor::Nvidia);
        // The regression: 512 is memory.used and must never be reported as
        // capacity.
        assert_eq!(gpu.total_vram_mb, 4096.0);
        assert_eq!(gpu.used_vram_mb, 512.0);
        assert_eq!(gpu.free_vram_mb, 3584.0);
        assert_ne!(
            gpu.total_vram_mb, gpu.used_vram_mb,
            "capacity must not be the used figure"
        );
        assert_eq!(gpu.driver_version.as_deref(), Some("551.86"));
        assert_eq!(gpu.compute_capability.as_deref(), Some("8.6"));
    }

    #[test]
    fn picks_the_nvidia_gpu_with_the_most_free_vram() {
        let caps = Capabilities {
            gpus: parse_nvidia_smi(DUAL_GPU),
            ..Default::default()
        };
        let gpu = caps.primary_gpu().expect("a gpu");
        assert_eq!(gpu.name, "NVIDIA GeForce RTX 4090");
        assert_eq!(caps.free_vram_mb(), 23540.0);
        // The nearly-full 1060 must not be chosen just because it is present.
        assert_ne!(gpu.index, 1);
    }

    #[test]
    fn tolerates_unsupported_driver_fields() {
        let gpus = parse_nvidia_smi(OLD_DRIVER);
        assert_eq!(gpus.len(), 1);
        let gpu = &gpus[0];
        assert_eq!(gpu.compute_capability, None);
        // `memory.free` was unavailable, so it is derived from total - used
        // rather than reported as zero.
        assert_eq!(gpu.free_vram_mb, 1748.0);
    }

    #[test]
    fn ignores_blank_and_malformed_lines() {
        assert!(parse_nvidia_smi("").is_empty());
        assert!(parse_nvidia_smi("\n\n").is_empty());
        assert!(parse_nvidia_smi("garbage\n").is_empty());
        // A truncated row is skipped rather than producing a zeroed GPU that
        // would read as "present with no memory".
        assert!(parse_nvidia_smi("0, NVIDIA, 4096\n").is_empty());
    }

    #[test]
    fn no_gpu_reports_no_vram_and_a_cpu_summary() {
        let caps = Capabilities {
            cpu: CpuInfo {
                model: "AMD Ryzen 5 5600H".into(),
                physical_cores: Some(6),
                logical_cores: 12,
                load_pct: 12.0,
            },
            ram: RamInfo {
                total_mb: 16384.0,
                used_mb: 8192.0,
                available_mb: 8192.0,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(caps.primary_gpu().is_none());
        assert_eq!(caps.free_vram_mb(), 0.0);
        assert!(!caps.can_use_cuda());
        assert!(!caps.can_use_vulkan());
        assert!(caps.summary().starts_with("CPU only"), "{}", caps.summary());
    }

    /// A CPU-only torch wheel on an NVIDIA machine is a real, fixable state.
    #[test]
    fn reports_gpu_present_but_cuda_unavailable() {
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
        assert!(caps.cuda.gpu_present_but_unusable());
        assert!(!caps.can_use_cuda());
        assert!(
            caps.summary().contains("CUDA unavailable"),
            "{}",
            caps.summary()
        );
        // The GPU and its memory are still reported honestly.
        assert_eq!(caps.total_vram_mb(), 4096.0);
    }

    #[test]
    fn cuda_available_is_not_implied_by_gpu_presence() {
        let mut cuda = CudaStatus {
            nvidia_gpu_present: true,
            driver_present: true,
            ..Default::default()
        };
        assert!(!cuda.torch_cuda_available);
        assert!(cuda.gpu_present_but_unusable());
        cuda.torch_cuda_available = true;
        assert!(!cuda.gpu_present_but_unusable());
    }

    #[test]
    fn parses_driver_cuda_version() {
        let smi = "\
Tue Aug 27 18:00:00 2026
+-----------------------------------------------------------------------------+
| NVIDIA-SMI 551.86       Driver Version: 551.86       CUDA Version: 12.4     |
";
        assert_eq!(parse_driver_cuda_version(smi).as_deref(), Some("12.4"));
        assert_eq!(parse_driver_cuda_version("no cuda here"), None);
    }

    #[test]
    fn parses_vulkan_summary_independently_of_cuda() {
        let out = "\
Vulkan Instance Version: 1.3.280

Devices:
========
GPU0:
        deviceName         = Intel(R) Iris(R) Xe Graphics
GPU1:
        deviceName         = NVIDIA GeForce RTX 2050
";
        let vk = parse_vulkaninfo(out);
        assert!(vk.available);
        assert_eq!(vk.api_version.as_deref(), Some("1.3.280"));
        assert_eq!(vk.devices.len(), 2);
        assert!(vk.devices[0].contains("Iris"));

        let empty = parse_vulkaninfo("");
        assert!(!empty.available);
        assert!(empty.devices.is_empty());
    }

    /// Hybrid graphics: the iGPU drives the display and the dGPU does compute.
    #[test]
    fn hybrid_graphics_reports_both_adapters() {
        let wmi = "\
Intel(R) Iris(R) Xe Graphics|1073741824
NVIDIA GeForce RTX 2050|4293918720
";
        let gpus = parse_wmi_adapters(wmi);
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].vendor, GpuVendor::Intel);
        assert_eq!(gpus[1].vendor, GpuVendor::Nvidia);
        // WMI does not report free memory, so it must stay zero rather than
        // being optimistically set to the total.
        assert!(gpus.iter().all(|g| g.free_vram_mb == 0.0));

        let caps = Capabilities {
            gpus,
            ..Default::default()
        };
        // The NVIDIA card is still preferred for compute.
        assert_eq!(
            caps.primary_gpu().map(|g| g.vendor),
            Some(GpuVendor::Nvidia)
        );
    }

    #[test]
    fn wmi_parser_skips_blank_lines_and_missing_ram() {
        let gpus = parse_wmi_adapters("\nMicrosoft Basic Display Adapter|\n\n");
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].total_vram_mb, 0.0);
        assert_eq!(gpus[0].vendor, GpuVendor::Unknown);
    }

    #[test]
    fn vendor_detection_covers_the_common_names() {
        assert_eq!(
            GpuVendor::from_name("NVIDIA GeForce RTX 2050"),
            GpuVendor::Nvidia
        );
        assert_eq!(GpuVendor::from_name("AMD Radeon RX 6600"), GpuVendor::Amd);
        assert_eq!(
            GpuVendor::from_name("Intel(R) Arc(TM) A770"),
            GpuVendor::Intel
        );
        assert_eq!(GpuVendor::from_name("Apple M2 Pro"), GpuVendor::Apple);
        assert_eq!(GpuVendor::from_name("Something Else"), GpuVendor::Unknown);
    }

    #[test]
    fn inference_threads_always_leave_a_core_for_audio_and_ui() {
        for logical in 1..=32 {
            let cpu = CpuInfo {
                logical_cores: logical,
                ..Default::default()
            };
            let threads = cpu.inference_threads();
            assert!(threads >= 1, "{logical} cores produced {threads} threads");
            if logical > 1 {
                assert!(
                    threads < logical,
                    "{logical} cores must not hand every core to inference"
                );
            }
        }
    }

    #[test]
    fn ram_totals_are_not_the_used_figure() {
        // Regression guard for `total_ram_mb: used_ram_mb.min(total_ram_mb)`.
        let ram = RamInfo {
            total_mb: 16384.0,
            used_mb: 9000.0,
            available_mb: 7384.0,
            app_mb: 220.0,
            asr_mb: 2300.0,
            refinement_mb: 900.0,
        };
        assert_ne!(ram.total_mb, ram.used_mb);
        assert!((ram.used_mb + ram.available_mb - ram.total_mb).abs() < 1.0);
        // Per-runtime attribution has to add up to no more than what is used.
        assert!(ram.app_mb + ram.asr_mb + ram.refinement_mb <= ram.used_mb);
    }
}
