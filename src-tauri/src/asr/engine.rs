use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialTranscript {
    pub text: String,
    pub language: String,
    pub is_final: bool,
    pub confidence: f32,
}

/// Live snapshot of the ASR engine (sidecar) state for the UI.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EngineStatus {
    pub loaded: bool,
    pub device: String,
    pub backend: String,
    pub vram_mb: f32,
    pub is_downloading: bool,
    #[serde(default)]
    pub is_loading: bool,
    pub download_progress_pct: u8,
    pub error: Option<String>,
    /// `true` iff `torch.cuda.is_available()` in the sidecar's Python
    /// process. Lets the UI distinguish "no GPU" from "GPU present but
    /// torch is the CPU-only build" so it can show a fixable hint.
    #[serde(default)]
    pub cuda_available: bool,
    /// The CUDA version the loaded torch was built against, e.g. `"12.1"`.
    /// `None` when the sidecar hasn't been able to import torch.
    #[serde(default)]
    pub torch_cuda_version: Option<String>,
    /// Pre-formatted `pip install` command shown to the user when a GPU
    /// is present but the sidecar's torch is CPU-only. `None` otherwise.
    #[serde(default)]
    pub gpu_hint: Option<String>,
}

pub trait ASREngine: Send + Sync {
    fn initialize(&mut self) -> Result<(), String>;
    fn load_model(&mut self, model_dir: &str, backend: &str) -> Result<(), String> {
        // Default shim: existing call sites continue to work; they get the
        // auto-ladder (today's behavior).
        self.load_model_with_precision(model_dir, backend, "auto")
    }
    fn load_model_with_precision(
        &mut self,
        model_dir: &str,
        backend: &str,
        precision: &str,
    ) -> Result<(), String>;
    fn unload_model(&mut self) -> Result<(), String>;
    fn is_model_loaded(&self) -> bool;

    /// Begin a stream; `vocabulary` are custom dictionary terms passed as
    /// recognition hotwords when the engine supports them.
    fn start_stream(&mut self, language: &str, vocabulary: &[String]) -> Result<(), String>;
    fn push_audio(&mut self, samples_16k_mono: &[f32]) -> Result<Option<String>, String>;
    fn get_partial_transcript(&mut self) -> Result<String, String>;
    fn stop_stream(&mut self) -> Result<String, String>;
    fn cancel_stream(&mut self) -> Result<(), String>;

    fn get_detected_language(&self) -> String;
    fn get_backend_name(&self) -> String;

    /// Download model `repo` weights into `model_dir` (async; poll engine_status).
    fn install_model_dir(&mut self, _model_dir: &str, _repo: &str) -> Result<(), String> {
        Err("Model install is not supported by this engine".into())
    }

    fn engine_status(&mut self) -> EngineStatus {
        EngineStatus {
            loaded: self.is_model_loaded(),
            backend: self.get_backend_name(),
            ..Default::default()
        }
    }

    fn set_resource_dir(&mut self, _dir: std::path::PathBuf) {}
}
