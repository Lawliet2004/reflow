use std::time::Instant;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AppStateEnum {
    Uninitialized,
    Initializing,
    Idle,
    LoadingModel,
    Ready,
    Recording,
    Processing,
    Injecting,
    Error,
    Updating,
}

impl Default for AppStateEnum {
    fn default() -> Self {
        Self::Ready
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyMetrics {
    pub hotkey_to_recording_ms: u64,
    pub recording_to_first_audio_ms: u64,
    pub audio_to_first_partial_ms: u64,
    pub speech_end_to_final_ms: u64,
    pub final_to_injection_ms: u64,
    #[serde(default)]
    pub rewrite_ms: u64,
    pub total_duration_ms: u64,
    pub last_updated: String,
}

impl Default for LatencyMetrics {
    fn default() -> Self {
        Self {
            hotkey_to_recording_ms: 0,
            recording_to_first_audio_ms: 0,
            audio_to_first_partial_ms: 0,
            speech_end_to_final_ms: 0,
            final_to_injection_ms: 0,
            rewrite_ms: 0,
            total_duration_ms: 0,
            last_updated: chrono::Utc::now().to_rfc3339(),
        }
    }
}

#[derive(Debug, Default)]
pub struct LatencyTimer {
    pub hotkey_pressed_at: Option<Instant>,
    pub recording_started_at: Option<Instant>,
    pub first_audio_at: Option<Instant>,
    pub first_partial_at: Option<Instant>,
    pub speech_ended_at: Option<Instant>,
    pub final_asr_at: Option<Instant>,
    pub rewrite_finished_at: Option<Instant>,
    pub injection_finished_at: Option<Instant>,
}

impl LatencyTimer {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn to_metrics(&self, total_audio_ms: u64) -> LatencyMetrics {
        let hotkey_to_rec = match (self.hotkey_pressed_at, self.recording_started_at) {
            (Some(h), Some(r)) => r.duration_since(h).as_millis() as u64,
            _ => 0,
        };

        let rec_to_audio = match (self.recording_started_at, self.first_audio_at) {
            (Some(r), Some(a)) => a.duration_since(r).as_millis() as u64,
            _ => 0,
        };

        let audio_to_partial = match (self.first_audio_at, self.first_partial_at) {
            (Some(a), Some(p)) => p.duration_since(a).as_millis() as u64,
            _ => 0,
        };

        let speech_to_final = match (self.speech_ended_at, self.final_asr_at) {
            (Some(s), Some(f)) => f.duration_since(s).as_millis() as u64,
            _ => 0,
        };

        let final_to_inj = match (self.final_asr_at, self.injection_finished_at) {
            (Some(f), Some(i)) => i.duration_since(f).as_millis() as u64,
            _ => 0,
        };
        let rewrite_ms = match (self.final_asr_at, self.rewrite_finished_at) {
            (Some(s), Some(r)) => r.duration_since(s).as_millis() as u64,
            _ => 0,
        };

        LatencyMetrics {
            hotkey_to_recording_ms: hotkey_to_rec,
            recording_to_first_audio_ms: rec_to_audio,
            audio_to_first_partial_ms: audio_to_partial,
            speech_end_to_final_ms: speech_to_final,
            final_to_injection_ms: final_to_inj,
            rewrite_ms,
            total_duration_ms: total_audio_ms,
            last_updated: chrono::Utc::now().to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetrics {
    pub cpu_usage_pct: f32,
    pub app_ram_mb: f32,
    pub model_ram_mb: f32,
    pub total_ram_mb: f32,
    pub vram_mb: f32,
    pub gpu_name: String,
    pub model_loaded: bool,
    pub backend_name: String,
    pub os_name: String,
    pub session: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelStatus {
    pub installed: bool,
    pub loaded: bool,
    pub version: String,
    pub name: String,
    pub size_bytes: u64,
    pub download_progress_pct: u8,
    pub download_speed_mbps: f32,
    pub backend: String,
    pub is_downloading: bool,
    #[serde(default)]
    pub is_loading: bool,
    pub error: Option<String>,
    /// `true` when `nvidia-smi` reports a non-CPU device on this machine.
    /// Independent of whether the ASR runtime can use it.
    #[serde(default)]
    pub gpu_available: bool,
    /// Display name of the GPU (or "CPU" if none).
    #[serde(default)]
    pub gpu_name: String,
    /// Whether the sidecar's torch is CUDA-enabled. `false` when the
    /// sidecar is still starting or torch is the CPU-only build.
    #[serde(default)]
    pub cuda_available: bool,
    /// The CUDA version the loaded torch was built against.
    #[serde(default)]
    pub torch_cuda_version: Option<String>,
    /// Pre-formatted `pip install` command for the user to enable GPU.
    #[serde(default)]
    pub asr_gpu_hint: Option<String>,
}

impl Default for ModelStatus {
    fn default() -> Self {
        Self {
            installed: true,
            loaded: true,
            version: "0.6B-v1".into(),
            name: "Qwen/Qwen3-ASR-0.6B".into(),
            size_bytes: 1_880_000_000,
            download_progress_pct: 100,
            download_speed_mbps: 0.0,
            backend: "auto".into(),
            is_downloading: false,
            is_loading: false,
            error: None,
            gpu_available: false,
            gpu_name: String::new(),
            cuda_available: false,
            torch_cuda_version: None,
            asr_gpu_hint: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectionFeedback {
    pub pasted: bool,
    pub fallback_copy: bool,
    pub paste_chord: String,
    pub process_name: String,
    pub message: String,
    /// Optional intelligence-tier status. `"rewriter_error"` carries a
    /// short, user-facing explanation when the Stage 2 LLM was attempted
    /// but failed. The frontend can surface this next to the model badge
    /// or as a toast.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingTranscriptPayload {
    pub committed_prefix: String,
    pub mutable_suffix: String,
    pub full_text: String,
    pub language: String,
    pub audio_level: f32,
    #[serde(default)]
    pub stage: String,
}
