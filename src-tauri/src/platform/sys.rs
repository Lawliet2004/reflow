use std::path::PathBuf;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

use crate::state::SystemMetrics;

use super::{os_display_name, session};

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

    pub fn detect_gpu() -> (String, f32) {
        static GPU: std::sync::OnceLock<(String, f32)> = std::sync::OnceLock::new();
        GPU.get_or_init(|| {
            if let Some(info) = nvidia_smi_gpu() {
                return info;
            }
            ("CPU".into(), 0.0)
        })
        .clone()
    }

    pub fn get_system_metrics() -> SystemMetrics {
        use std::sync::{Mutex, OnceLock};

        // Reuse one sysinfo handle and refresh only what the UI shows.
        // A full process-list scan on every poll wastes noticeable CPU.
        static SYS: OnceLock<Mutex<System>> = OnceLock::new();
        let sys = SYS.get_or_init(|| {
            Mutex::new(System::new_with_specifics(
                RefreshKind::nothing()
                    .with_cpu(CpuRefreshKind::everything())
                    .with_memory(MemoryRefreshKind::everything()),
            ))
        });

        let cpu_usage_pct;
        let total_ram_mb;
        let used_ram_mb;
        let app_ram_mb;
        {
            let mut guard = sys
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.refresh_cpu_all();
            guard.refresh_memory();
            guard.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[Pid::from_u32(std::process::id())]),
                true,
                ProcessRefreshKind::everything(),
            );
            cpu_usage_pct = guard.global_cpu_usage();
            total_ram_mb = (guard.total_memory() as f32) / (1024.0 * 1024.0);
            used_ram_mb = (guard.used_memory() as f32) / (1024.0 * 1024.0);
            app_ram_mb = guard
                .process(Pid::from_u32(std::process::id()))
                .map(|proc| proc.memory() as f32 / (1024.0 * 1024.0))
                .unwrap_or(0.0);
        }

        let (gpu_name, vram_mb) = Self::detect_gpu();
        let session = session();

        SystemMetrics {
            cpu_usage_pct,
            app_ram_mb,
            model_ram_mb: 0.0,
            total_ram_mb: used_ram_mb.min(total_ram_mb),
            vram_mb,
            gpu_name: gpu_name.clone(),
            model_loaded: false,
            backend_name: if gpu_name == "CPU" {
                "CPU".into()
            } else {
                format!("GPU ({gpu_name})")
            },
            os_name: os_display_name(),
            session: session.as_str().to_string(),
        }
    }

    pub fn generate_diagnostics_report() -> String {
        let metrics = Self::get_system_metrics();
        let audio_backend = if cfg!(windows) {
            "WASAPI (cpal)"
        } else if cfg!(target_os = "linux") {
            "ALSA / PipeWire (cpal)"
        } else {
            "cpal"
        };

        format!(
            "# Reflow Local Dictation System Diagnostics\n\n\
            - App Version: 0.1.0 (Tauri 2)\n\
            - OS: {}\n\
            - Display session: {}\n\
            - Primary ASR Model: Qwen/Qwen3-ASR-0.6B\n\
            - Active Backend: {}\n\
            - GPU Device: {}\n\
            - GPU memory (used): {:.1} MB\n\
            - App RAM: {:.1} MB\n\
            - System RAM (used): {:.1} MB\n\
            - CPU Load: {:.1}%\n\
            - Audio Subsystem: {}\n\
            - VAD: Low-Latency RMS Energy + Hangover Pre/Post Buffer\n\
            - History: Local SQLite @ {}\n\
            - Data directory: {}\n\
            - Privacy: 100% Offline (No Cloud API / Zero Audio Telemetry)\n",
            metrics.os_name,
            metrics.session,
            metrics.backend_name,
            metrics.gpu_name,
            metrics.vram_mb,
            metrics.app_ram_mb,
            metrics.total_ram_mb,
            metrics.cpu_usage_pct,
            audio_backend,
            Self::get_db_path().display(),
            Self::get_app_dir().display()
        )
    }
}

fn nvidia_smi_gpu() -> Option<(String, f32)> {
    let output = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.used",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&output.stdout);
    let line = line.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    let mut parts = line.split(',').map(|part| part.trim());
    let name = parts.next()?.to_string();
    let vram = parts
        .next()
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(0.0);
    Some((name, vram))
}
