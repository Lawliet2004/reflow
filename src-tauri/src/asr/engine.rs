use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialTranscript {
    pub text: String,
    pub language: String,
    pub is_final: bool,
    pub confidence: f32,
}

/// Live snapshot of the ASR engine (sidecar) state for the UI.
///
/// Every field is `#[serde(default)]`: a reply from an older sidecar that
/// predates a field must deserialize rather than fail, because this struct is
/// the wire format between two independently-updated halves of the app.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EngineStatus {
    pub loaded: bool,
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub vram_mb: f32,
    #[serde(default)]
    pub is_downloading: bool,
    #[serde(default)]
    pub is_loading: bool,
    #[serde(default)]
    pub download_progress_pct: u8,
    #[serde(default)]
    pub error: Option<String>,
    /// `true` iff `torch.cuda.is_available()` in the sidecar's Python
    /// process. Lets the UI distinguish "no GPU" from "GPU present but
    /// torch is the CPU-only build" so it can show a fixable hint.
    #[serde(default)]
    pub cuda_available: bool,
    /// `true` while the sidecar has not finished its one-time CUDA probe.
    ///
    /// `cuda_available == false` means different things before and after the
    /// probe: "not looked yet" versus "looked, and torch cannot use CUDA".
    /// A load decision made during warm-up must wait rather than act, or it
    /// will send a GPU-capable machine to the CPU every single launch.
    #[serde(default)]
    pub cuda_probe_pending: bool,
    /// The CUDA version the loaded torch was built against, e.g. `"12.1"`.
    /// `None` when the sidecar hasn't been able to import torch.
    #[serde(default)]
    pub torch_cuda_version: Option<String>,
    /// Pre-formatted `pip install` command shown to the user when a GPU
    /// is present but the sidecar's torch is CPU-only. `None` otherwise.
    #[serde(default)]
    pub gpu_hint: Option<String>,
    /// `true` when the loaded model is partly backed by shared system memory
    /// rather than VRAM.
    ///
    /// On Windows WDDM this is not an error condition from the driver's point
    /// of view — the allocation succeeds and inference runs about ten times
    /// slower. Treating it as success is how a benchmark-driven architecture
    /// ends up validating its own worst configuration, so it is reported as a
    /// first-class status alongside OOM and timeout.
    #[serde(default)]
    pub spill_detected: bool,
    /// Human-readable evidence for `spill_detected`.
    #[serde(default)]
    pub spill_reasons: Vec<String>,
    /// `false` when spill could not be assessed (CPU load, or no `nvidia-smi`).
    /// Distinct from "checked and clean".
    #[serde(default)]
    pub spill_checked: bool,
    /// Machine-readable failure classification. See [`LoadFailureKind`].
    #[serde(default)]
    pub failure_kind: Option<String>,
    /// Precision the model actually loaded with, not the one requested.
    #[serde(default)]
    pub precision: String,
    /// Seconds the last successful load took.
    #[serde(default)]
    pub load_seconds: f32,
    /// Real-time factor measured during warmup.
    #[serde(default)]
    pub warmup_rtf: Option<f32>,
}

/// Why a model load failed. Each variant implies different remediation, so
/// collapsing them into one string throws away the only actionable detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadFailureKind {
    /// Loaded, but partly in shared system memory. Benchmark-invalidating.
    Spill,
    /// The device refused the allocation outright.
    Oom,
    /// The requested quantization is not available in this build.
    UnsupportedPrecision,
    /// Weights are not on disk.
    WeightsMissing,
    Timeout,
    Crash,
    Unknown,
}

impl LoadFailureKind {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "spill" => Self::Spill,
            "oom" => Self::Oom,
            "unsupported_precision" => Self::UnsupportedPrecision,
            "weights_missing" => Self::WeightsMissing,
            "timeout" => Self::Timeout,
            "crash" => Self::Crash,
            _ => Self::Unknown,
        }
    }

    /// `true` when a benchmark result recorded under this failure must be
    /// discarded rather than cached.
    pub fn invalidates_benchmark(&self) -> bool {
        matches!(self, Self::Spill | Self::Oom | Self::Timeout | Self::Crash)
    }

    /// `true` when retrying the same configuration cannot help.
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            Self::Spill | Self::UnsupportedPrecision | Self::WeightsMissing
        )
    }

    /// One-line remediation shown next to the error.
    pub fn remediation(&self) -> &'static str {
        match self {
            Self::Spill => {
                "This model does not fit in VRAM on this GPU. Pick a smaller \
                 model or a lower precision, or run it on the CPU."
            }
            Self::Oom => {
                "Out of GPU memory. Close other GPU applications, or pick a \
                 smaller model or lower precision."
            }
            Self::UnsupportedPrecision => {
                "This precision is not available in the installed runtime. \
                 Pick a different precision."
            }
            Self::WeightsMissing => "Model weights are not installed. Download them first.",
            Self::Timeout => "The runtime did not respond in time. Try reloading the model.",
            Self::Crash => "The runtime crashed. Check the logs, then reload the model.",
            Self::Unknown => "Check the logs for the underlying error.",
        }
    }
}

impl EngineStatus {
    /// `true` when the engine is loaded and known to be running entirely in
    /// device memory.
    ///
    /// Deliberately requires `spill_checked`: an unverified load is not the
    /// same as a verified-clean one, and a resource decision must not treat it
    /// as such.
    pub fn is_healthy_on_device(&self) -> bool {
        self.loaded && self.spill_checked && !self.spill_detected
    }

    pub fn failure_kind(&self) -> Option<LoadFailureKind> {
        self.failure_kind.as_deref().map(LoadFailureKind::parse)
    }
}

pub trait ASREngine: Send + Sync {
    fn initialize(&mut self) -> Result<(), String>;
    /// Ask the runtime to compute its CUDA capability snapshot now.
    ///
    /// For the Python sidecar this runs the warm imports (10-60s cold) and
    /// publishes `cuda_available` into `engine_status`. The default is a
    /// no-op for engines whose CUDA status is known without probing. Senders
    /// must treat the ack as "started", not "done" — completion is observed
    /// through `engine_status` flipping `cuda_probe_pending` to `false`.
    fn probe_cuda(&mut self) -> Result<(), String> {
        Ok(())
    }
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
    fn cancellation_signal(&self) -> Option<std::sync::Arc<std::sync::atomic::AtomicBool>> {
        None
    }

    fn get_detected_language(&self) -> String;
    fn get_backend_name(&self) -> String;

    /// Non-fatal notice from the most recent transcription, cleared on read.
    ///
    /// Exists so that "only the first 120 s was transcribed" reaches the user
    /// instead of only the log file.
    fn take_last_warning(&mut self) -> Option<String> {
        None
    }

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
