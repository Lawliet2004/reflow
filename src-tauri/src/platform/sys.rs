use std::path::PathBuf;

use crate::state::SystemMetrics;

use super::session;

pub struct PlatformSys;

impl PlatformSys {
    pub fn get_app_dir() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("reflow")
    }

    pub fn get_db_path() -> PathBuf {
        Self::get_app_dir().join("database").join("history.db")
    }

    pub fn get_config_path() -> PathBuf {
        Self::get_app_dir().join("config").join("settings.json")
    }

    pub fn get_models_dir() -> PathBuf {
        Self::get_app_dir().join("models")
    }

    pub fn get_logs_dir() -> PathBuf {
        Self::get_app_dir().join("logs")
    }

    /// Legacy shim: GPU name plus **total** VRAM.
    ///
    /// The second element used to be `memory.used`, which callers then treated
    /// as capacity. Prefer [`crate::capability::capabilities`] for anything
    /// that makes a decision; this exists for display-only call sites.
    pub fn detect_gpu() -> (String, f32) {
        let caps = crate::capability::capabilities();
        match caps.primary_gpu() {
            Some(gpu) => (gpu.name.clone(), gpu.total_vram_mb),
            None => ("CPU".into(), 0.0),
        }
    }

    pub fn get_system_metrics() -> SystemMetrics {
        let caps = crate::capability::capabilities();
        let gpu = caps.primary_gpu();
        let gpu_name = gpu.map(|g| g.name.clone()).unwrap_or_else(|| "CPU".into());
        let session = session();

        // Name the backend by what can actually be dispatched, not by what is
        // physically present. "GPU (RTX 2050)" while torch is a CPU-only build
        // is the exact misreport this task removes.
        let backend_name = match gpu {
            None => "CPU".to_string(),
            Some(g) if caps.cuda.torch_cuda_available => format!("CUDA ({})", g.name),
            Some(g) if caps.vulkan.available => format!("Vulkan ({})", g.name),
            Some(g) => format!("CPU ({} present, CUDA unavailable)", g.name),
        };

        SystemMetrics {
            cpu_usage_pct: caps.cpu.load_pct,
            app_ram_mb: caps.ram.app_mb,
            model_ram_mb: caps.ram.asr_mb,
            total_ram_mb: caps.ram.total_mb,
            used_ram_mb: caps.ram.used_mb,
            available_ram_mb: caps.ram.available_mb,
            vram_mb: gpu.map(|g| g.used_vram_mb).unwrap_or(0.0),
            total_vram_mb: gpu.map(|g| g.total_vram_mb).unwrap_or(0.0),
            used_vram_mb: gpu.map(|g| g.used_vram_mb).unwrap_or(0.0),
            free_vram_mb: gpu.map(|g| g.free_vram_mb).unwrap_or(0.0),
            gpu_name,
            gpu_vendor: gpu
                .map(|g| g.vendor.as_str().to_string())
                .unwrap_or_default(),
            gpu_present: gpu.is_some(),
            cuda_available: caps.cuda.torch_cuda_available,
            vulkan_available: caps.vulkan.available,
            cpu_model: caps.cpu.model.clone(),
            physical_cores: caps.cpu.physical_cores.unwrap_or(0),
            logical_cores: caps.cpu.logical_cores,
            asr_ram_mb: caps.ram.asr_mb,
            refinement_ram_mb: caps.ram.refinement_mb,
            model_loaded: false,
            backend_name,
            os_name: caps.os_name.clone(),
            session: session.as_str().to_string(),
        }
    }

    /// Human-readable diagnostics.
    ///
    /// Deliberately contains no transcript text: dictation content must never
    /// leak into a report a user might paste into an issue.
    pub fn generate_diagnostics_report() -> String {
        let caps = crate::capability::capabilities_uncached();
        let audio_backend = if cfg!(windows) {
            "WASAPI (cpal)"
        } else if cfg!(target_os = "linux") {
            "ALSA / PipeWire (cpal)"
        } else {
            "cpal"
        };

        let mut report = String::new();
        report.push_str("# Reflow Local Dictation System Diagnostics\n\n");
        report.push_str(&format!(
            "- App version: {} (Tauri 2)\n",
            env!("CARGO_PKG_VERSION")
        ));
        report.push_str(&format!("- OS: {}\n", caps.os_name));
        report.push_str(&format!("- Display session: {}\n", session().as_str()));
        report.push_str(&format!("- Probed at: {}\n", caps.probed_at));

        report.push_str("\n## CPU\n");
        report.push_str(&format!("- Model: {}\n", caps.cpu.model));
        report.push_str(&format!(
            "- Cores: {} physical / {} logical ({} available to inference)\n",
            caps.cpu
                .physical_cores
                .map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".into()),
            caps.cpu.logical_cores,
            caps.cpu.inference_threads()
        ));
        report.push_str(&format!("- Load: {:.1}%\n", caps.cpu.load_pct));

        report.push_str("\n## Memory\n");
        report.push_str(&format!(
            "- System RAM: {:.0} MB total / {:.0} MB used / {:.0} MB available\n",
            caps.ram.total_mb, caps.ram.used_mb, caps.ram.available_mb
        ));
        report.push_str(&format!("- Reflow: {:.0} MB\n", caps.ram.app_mb));
        report.push_str(&format!("- ASR sidecar: {:.0} MB\n", caps.ram.asr_mb));
        report.push_str(&format!(
            "- Refinement runtime: {:.0} MB\n",
            caps.ram.refinement_mb
        ));

        report.push_str("\n## GPU\n");
        if caps.gpus.is_empty() {
            report.push_str("- No GPU detected\n");
        } else {
            for gpu in &caps.gpus {
                report.push_str(&format!(
                    "- [{}] {} ({}): {:.0} MB total / {:.0} MB used / {:.0} MB free\n",
                    gpu.index,
                    gpu.name,
                    gpu.vendor.as_str(),
                    gpu.total_vram_mb,
                    gpu.used_vram_mb,
                    gpu.free_vram_mb
                ));
                if let Some(driver) = &gpu.driver_version {
                    report.push_str(&format!("  - Driver: {driver}\n"));
                }
                if let Some(cc) = &gpu.compute_capability {
                    report.push_str(&format!("  - Compute capability: {cc}\n"));
                }
            }
        }
        report.push_str(&format!(
            "- NVIDIA device present: {}\n",
            caps.cuda.nvidia_gpu_present
        ));
        report.push_str(&format!(
            "- NVIDIA driver present: {}{}\n",
            caps.cuda.driver_present,
            caps.cuda
                .driver_cuda_version
                .as_ref()
                .map(|v| format!(" (supports CUDA {v})"))
                .unwrap_or_default()
        ));
        report.push_str(&format!(
            "- torch.cuda.is_available(): {}{}\n",
            caps.cuda.torch_cuda_available,
            caps.cuda
                .torch_cuda_version
                .as_ref()
                .map(|v| format!(" (built for CUDA {v})"))
                .unwrap_or_default()
        ));
        if caps.cuda.gpu_present_but_unusable() {
            report.push_str(
                "  - GPU present but CUDA is unavailable to Python: the installed torch is \
                 most likely a CPU-only wheel.\n",
            );
        }
        report.push_str(&format!(
            "- Vulkan: {}{}\n",
            caps.vulkan.available,
            caps.vulkan
                .api_version
                .as_ref()
                .map(|v| format!(" (instance {v})"))
                .unwrap_or_default()
        ));
        for device in &caps.vulkan.devices {
            report.push_str(&format!("  - {device}\n"));
        }

        report.push_str("\n## Pipeline\n");
        report.push_str(&format!("- Audio subsystem: {audio_backend}\n"));
        report.push_str("- VAD: RMS energy with hangover and silence-boundary detection\n");
        report.push_str(&format!(
            "- History: local SQLite @ {}\n",
            Self::get_db_path().display()
        ));
        report.push_str(&format!(
            "- Data directory: {}\n",
            Self::get_app_dir().display()
        ));
        report.push_str("- Privacy: fully local. No cloud API, no audio telemetry.\n");
        report.push_str("- Transcript contents are excluded from this report by design.\n");

        report
    }
}
