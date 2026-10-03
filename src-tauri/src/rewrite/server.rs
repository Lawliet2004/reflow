use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};

use super::client::FlowClient;
use crate::capability::{capabilities, capabilities_uncached, Capabilities};
use crate::platform::PlatformSys;
use crate::profile::vram_reserve_mb;

pub struct FlowModelSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub filename: &'static str,
    pub repo: &'static str,
    pub approx_bytes: u64,
    /// Repeating model blocks accepted by `--n-gpu-layers`. A value of 99
    /// remains the llama.cpp convention for offloading every eligible tensor.
    pub gpu_layer_count: u32,
}

/// Pick a `llama-server` execution mode from the user's Stage 2 preference and
/// the hardware detected on this machine. The actual accelerated backend is
/// still validated by launch; a failed GPU launch transparently falls back to
/// CPU and is surfaced in runtime status.
pub fn pick_llama_mode(requested: &str) -> LlamaMode {
    pick_llama_mode_for_capabilities(requested, &capabilities())
}

fn pick_llama_mode_for_capabilities(requested: &str, caps: &Capabilities) -> LlamaMode {
    let requested = requested.trim().to_ascii_lowercase();
    let gpu_available = caps.primary_gpu().is_some();
    let gpu_name = caps
        .primary_gpu()
        .map(|gpu| gpu.name.clone())
        .unwrap_or_default();

    match requested.as_str() {
        "cpu" => LlamaMode::Cpu,
        "gpu" | "cuda" | "vulkan" | "auto" if gpu_available => LlamaMode::Gpu(gpu_name),
        _ => LlamaMode::Cpu,
    }
}

const FULL_GPU_OFFLOAD: u32 = 99;
const GPU_TRANSIENT_HEADROOM_MB: f32 = 256.0;
const GPU_RUNTIME_OVERHEAD_MB: f32 = 192.0;
const GPU_WEIGHT_SAFETY_FACTOR: f32 = 1.10;

/// Resolve automatic GPU intent to the largest safe offload. More resident
/// layers avoid CPU/GPU transfers and therefore minimize expected steady-state
/// latency. When all weights fit, 99 asks llama.cpp for full offload; otherwise
/// a positive model-specific layer count preserves partial offload.
pub(crate) fn latency_optimized_gpu_layers(
    flow_model: &str,
    caps: &Capabilities,
    configured_reserve_mb: u32,
    context_size: u32,
) -> u32 {
    if caps.primary_gpu().is_none() {
        return 0;
    }

    let spec = flow_model_spec(flow_model);
    if spec.approx_bytes == 0 || spec.gpu_layer_count == 0 {
        return 0;
    }

    // Some non-NVIDIA probes can enumerate Vulkan without exposing live VRAM.
    // One layer is a conservative useful attempt; launch fallback still keeps
    // the CPU path safe if the driver cannot admit it.
    let free_vram_mb = caps.free_vram_mb();
    if free_vram_mb <= 0.0 {
        return 1;
    }

    let reserve_mb = if configured_reserve_mb > 0 {
        configured_reserve_mb as f32
    } else {
        vram_reserve_mb(caps)
    };
    // A conservative cache/workspace reserve until a measured co-resident
    // calibration exists. Increasing context must consume admission headroom.
    let context_workspace_mb = context_size.clamp(256, 32_768) as f32 * 0.125;
    let layer_budget_mb = free_vram_mb
        - reserve_mb
        - GPU_TRANSIENT_HEADROOM_MB
        - GPU_RUNTIME_OVERHEAD_MB
        - context_workspace_mb;
    if layer_budget_mb <= 0.0 {
        return 0;
    }

    let estimated_weight_mb =
        (spec.approx_bytes as f32 / (1024.0 * 1024.0)) * GPU_WEIGHT_SAFETY_FACTOR;
    if layer_budget_mb >= estimated_weight_mb {
        return FULL_GPU_OFFLOAD;
    }

    let fraction = (layer_budget_mb / estimated_weight_mb).clamp(0.0, 1.0);
    let layers = (spec.gpu_layer_count as f32 * fraction).floor() as u32;
    layers.min(spec.gpu_layer_count.saturating_sub(1))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlamaMode {
    Cpu,
    Gpu(String),
}

impl LlamaMode {
    pub fn backend_label(&self) -> String {
        match self {
            LlamaMode::Cpu => "CPU".into(),
            LlamaMode::Gpu(name) if name.is_empty() || name == "CPU" => "GPU".into(),
            LlamaMode::Gpu(name) => format!("GPU ({name})"),
        }
    }

    /// Number of transformer layers to offload to the GPU. `99` covers the
    /// entire model for any GGUF we ship; `0` keeps everything on the CPU.
    /// An explicit `override_layers` (when the user picked a value in
    /// Settings) wins; otherwise we return the binary 0/99 default.
    pub fn n_gpu_layers(&self, override_layers: Option<u32>) -> u32 {
        if let Some(n) = override_layers {
            return n;
        }
        match self {
            LlamaMode::Cpu => 0,
            LlamaMode::Gpu(_) => 99,
        }
    }

    pub fn is_gpu(&self) -> bool {
        matches!(self, LlamaMode::Gpu(_))
    }
}

/// Threads to hand `llama-server` via `--threads`, or `None` to keep the
/// server's own default.
///
/// `-t` sizes llama.cpp's CPU worker pool, so the right pool is the physical
/// core count whenever the CPU runs real layers — CPU mode, or a partial GPU
/// offload. SMT siblings buy almost nothing on memory-bound matmul, and on a
/// 6-core/12-thread part the logical count would feed the pool twice the
/// useful width. When every layer is offloaded the pool idles, so the flag is
/// left off entirely and the server keeps its own default.
pub(crate) fn llama_server_threads(
    mode: &LlamaMode,
    n_gpu_layers: u32,
    model_gpu_layers: u32,
    caps: &Capabilities,
) -> Option<usize> {
    let cpu_runs_layers = match mode {
        LlamaMode::Cpu => true,
        LlamaMode::Gpu(_) => {
            n_gpu_layers < FULL_GPU_OFFLOAD
                && (model_gpu_layers == 0 || n_gpu_layers < model_gpu_layers)
        }
    };
    if !cpu_runs_layers {
        return None;
    }
    // `inference_threads` (logical - 1) is the codebase's conservative answer
    // when the OS will not report a physical count.
    Some(
        caps.cpu
            .physical_cores
            .unwrap_or_else(|| caps.cpu.inference_threads())
            .max(1),
    )
}

pub const FLOW_MODELS: [FlowModelSpec; 3] = [
    FlowModelSpec {
        id: "none",
        label: "None",
        filename: "",
        repo: "",
        approx_bytes: 0,
        gpu_layer_count: 0,
    },
    // Measured against LFM2.5-1.2B, which this replaced: 508 MB on disk vs
    // 697 MB, 2.0-2.5s to serve /health vs 5.1s, comparable completion time,
    // and it resolves spoken self-repairs correctly ("friday i mean thursday"
    // -> "Thursday"). 24 blocks, per `num_hidden_layers` in the upstream
    // config; the architecture is hybrid (18 linear + 6 full attention).
    FlowModelSpec {
        id: "qwen3.5-0.8b",
        label: "Qwen3.5 0.8B",
        filename: "Qwen3.5-0.8B-Q4_K_M.gguf",
        repo: "unsloth/Qwen3.5-0.8B-GGUF",
        approx_bytes: 532_517_120,
        gpu_layer_count: 24,
    },
    // `Qwen/Qwen3.5-2B-GGUF` does not exist, so the deep_context tier could
    // never install. `unsloth/Qwen3.5-2B-GGUF` is a real repo and its files
    // are named without the `-Instruct` infix.
    FlowModelSpec {
        id: "qwen3.5-2b",
        label: "Qwen3.5 2B",
        filename: "Qwen3.5-2B-Q4_K_M.gguf",
        repo: "unsloth/Qwen3.5-2B-GGUF",
        approx_bytes: 1_290_000_000,
        gpu_layer_count: 24,
    },
];

pub fn flow_model_spec(flow_model: &str) -> &'static FlowModelSpec {
    FLOW_MODELS
        .iter()
        .find(|spec| spec.id == flow_model)
        .unwrap_or(&FLOW_MODELS[0])
}

/// HTTP timeout for a keep-warm CPU rewrite. CPU llama-server needs
/// several seconds for prompt eval + decode on a laptop.
pub fn flow_http_timeout() -> Duration {
    Duration::from_secs(12)
}

/// How a launch attempt failed. Kept distinct because the remediation
/// differs: a missing binary needs Reinstall, an early exit needs the CPU
/// fallback, and a health timeout usually means the model is simply too large
/// for the selected device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowLaunchFailure {
    BinaryMissing,
    RuntimeFlavorMismatch,
    ModelMissing,
    /// The OS refused to start the process at all.
    SpawnFailed(String),
    /// The child started and then exited before serving `/health`.
    ExitedEarly {
        status: String,
        stderr: String,
    },
    /// The child stayed alive but never became healthy in time.
    HealthTimeout {
        stderr: String,
    },
    /// The chosen port was taken between our probe bind and the child's bind.
    PortInUse {
        port: u16,
    },
}

impl FlowLaunchFailure {
    /// Stable machine-readable kind, for UI branching that must not depend on
    /// matching English prose.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::BinaryMissing => "binary_missing",
            Self::RuntimeFlavorMismatch => "runtime_flavor_mismatch",
            Self::ModelMissing => "model_missing",
            Self::SpawnFailed(_) => "spawn_failed",
            Self::ExitedEarly { .. } => "exited_early",
            Self::HealthTimeout { .. } => "health_timeout",
            Self::PortInUse { .. } => "port_in_use",
        }
    }

    /// `true` when retrying on a different port is the right response.
    pub fn is_port_conflict(&self) -> bool {
        match self {
            Self::PortInUse { .. } => true,
            Self::ExitedEarly { stderr, .. } => stderr_indicates_port_conflict(stderr),
            _ => false,
        }
    }
}

impl std::fmt::Display for FlowLaunchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BinaryMissing => write!(f, "llama-server is not installed"),
            Self::RuntimeFlavorMismatch => {
                write!(f, "llama-server GPU runtime is not installed")
            }
            Self::ModelMissing => write!(f, "Flow model is not installed"),
            Self::SpawnFailed(e) => write!(f, "failed to start llama-server: {e}"),
            Self::ExitedEarly { status, stderr } => {
                write!(f, "llama-server exited before becoming ready ({status})")?;
                if !stderr.is_empty() {
                    write!(f, ": {stderr}")?;
                }
                Ok(())
            }
            Self::HealthTimeout { stderr } => {
                write!(f, "llama-server did not become ready in time")?;
                if !stderr.is_empty() {
                    write!(f, ": {stderr}")?;
                }
                Ok(())
            }
            Self::PortInUse { port } => {
                write!(f, "port {port} was taken before llama-server could bind it")
            }
        }
    }
}

fn stderr_indicates_port_conflict(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    s.contains("address already in use")
        || s.contains("bind: ")
        || s.contains("failed to bind")
        || s.contains("eaddrinuse")
        || s.contains("only one usage of each socket address")
}

/// The request that produced the currently running child.
///
/// The cache key has to be the *request*, not the resolved outcome. When a GPU
/// request falls back to CPU the runtime ends up with `active_mode = Cpu` and
/// `active_n_gpu_layers = Some(0)`, while `pick_llama_mode` keeps returning
/// `Gpu`. Comparing outcomes therefore never matched and `llama-server` was
/// torn down and relaunched on every single dictation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRequest {
    pub flow_model: String,
    pub requested_mode: LlamaMode,
    pub override_layers: Option<u32>,
    pub vram_reserve_mb: u32,
    /// `--ctx-size`. Part of the request because changing it requires a
    /// relaunch — `llama-server` fixes its KV cache at startup.
    pub context_size: u32,
}

/// Lines of `llama-server` stderr retained for diagnostics.
const STDERR_RING_LINES: usize = 200;

#[derive(Debug, Default)]
pub struct StderrRing {
    lines: std::collections::VecDeque<String>,
}

impl StderrRing {
    pub fn push(&mut self, line: String) {
        if self.lines.len() == STDERR_RING_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Whole buffer, oldest first.
    pub fn text(&self) -> String {
        self.lines.iter().cloned().collect::<Vec<_>>().join("\n")
    }

    /// Last `n` lines, for a one-line-ish error surface.
    pub fn tail(&self, n: usize) -> String {
        let skip = self.lines.len().saturating_sub(n);
        self.lines
            .iter()
            .skip(skip)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub struct FlowRuntime {
    /// Serialize readiness changes from startup, settings, and dictation tasks.
    operation: Mutex<()>,
    child: Mutex<Option<Child>>,
    pub client: parking_lot::RwLock<FlowClient>,
    pub active_model: RwLock<Option<String>>,
    /// Cache key for `ensure()`. See [`RuntimeRequest`].
    pub active_request: RwLock<Option<RuntimeRequest>>,
    /// Rolling `llama-server` stderr. Previously discarded via `Stdio::null()`,
    /// which made every launch failure undiagnosable.
    pub stderr_ring: Arc<Mutex<StderrRing>>,
    /// The execution mode that the running `llama-server` child was started
    /// with. `None` when the runtime is shut down. The frontend reads this via
    /// `FlowStatus.backend` to display "loaded on GPU (RTX 4060)" or
    /// "loaded on CPU".
    pub active_mode: RwLock<Option<LlamaMode>>,
    /// The actual `--n-gpu-layers` value the running child was launched
    /// with. `None` when the runtime is shut down. Surfaces in
    /// `FlowStatus.n_gpu_layers` so the UI can show e.g. "30/99 layers".
    pub active_n_gpu_layers: RwLock<Option<u32>>,
    /// `true` while `ensure()` is in flight. Replaces the previous
    /// hard-coded `is_loading: false` in `build_flow_status` so the UI
    /// can show a transient "loading" state between user action and
    /// either a successful launch or a failure.
    is_starting: AtomicBool,
    /// Last `ensure()` failure or GPU fallback warning. Cleared on a full success and on
    /// `shutdown()`. Surfaces in `FlowStatus.last_error` so the UI can
    /// show a one-line "LLM runtime error" hint with a Reinstall button.
    pub last_error: RwLock<Option<String>>,
    model_paths: Option<(PathBuf, PathBuf)>,
}

impl Default for FlowRuntime {
    fn default() -> Self {
        Self {
            operation: Mutex::new(()),
            child: Mutex::new(None),
            client: parking_lot::RwLock::new(FlowClient::new_missing()),
            active_model: RwLock::new(None),
            active_request: RwLock::new(None),
            stderr_ring: Arc::new(Mutex::new(StderrRing::default())),
            active_mode: RwLock::new(None),
            active_n_gpu_layers: RwLock::new(None),
            is_starting: AtomicBool::new(false),
            last_error: RwLock::new(None),
            model_paths: None,
        }
    }
}

impl Drop for FlowRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl FlowRuntime {
    pub fn for_audio_model(model: PathBuf, mmproj: PathBuf) -> Self {
        let mut runtime = Self::default();
        runtime.model_paths = Some((model, mmproj));
        runtime
    }

    #[cfg(test)]
    pub(crate) fn attach_test_child(&self, child: Child) {
        *self.child.lock() = Some(child);
    }

    fn model_path(&self, model_id: &str) -> PathBuf {
        self.model_paths
            .as_ref()
            .map(|(model, _)| model.clone())
            .unwrap_or_else(|| flow_gguf_path(model_id))
    }

    pub fn status_ready(&self) -> bool {
        if self.client.read().base_url.is_none() {
            return false;
        }
        // A cached URL is not evidence that its process survived. Without
        // checking the child, ensure() kept cache-hitting after a server crash.
        self.child
            .lock()
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
    }

    pub fn active_model(&self) -> Option<String> {
        self.active_model.read().clone()
    }

    pub fn active_mode(&self) -> Option<LlamaMode> {
        self.active_mode.read().clone()
    }

    pub fn active_n_gpu_layers(&self) -> Option<u32> {
        *self.active_n_gpu_layers.read()
    }

    pub fn is_starting(&self) -> bool {
        self.is_starting.load(Ordering::Acquire)
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.read().clone()
    }

    /// Apply the user's request deadline to the live client. A timeout is a
    /// client-side setting, not server identity, so it is deliberately kept
    /// out of `RuntimeRequest` — changing it must not relaunch llama-server.
    /// `0` means unset: keep the shipped default.
    pub fn set_deadline_ms(&self, deadline_ms: u64) {
        if deadline_ms > 0 {
            self.client.write().timeout = Duration::from_millis(deadline_ms);
        }
    }

    pub fn shutdown(&self) {
        let _operation = self.operation.lock();
        self.shutdown_inner();
    }

    fn shutdown_inner(&self) {
        self.stop_child();
        *self.client.write() = FlowClient::new_missing();
        *self.active_model.write() = None;
        *self.active_request.write() = None;
        *self.active_mode.write() = None;
        *self.active_n_gpu_layers.write() = None;
        self.is_starting.store(false, Ordering::Release);
        *self.last_error.write() = None;
    }

    fn stop_child(&self) {
        if let Some(mut child) = self.child.lock().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Recent `llama-server` stderr, for the diagnostics panel.
    pub fn stderr_log(&self) -> String {
        self.stderr_ring.lock().text()
    }

    /// Resolve the user's `compute_backend` setting into a concrete execution
    /// mode for the next launch. Exposed so the UI can show "GPU is selected"
    /// before the first inference happens.
    pub fn resolve_mode(&self, compute_backend: &str) -> LlamaMode {
        pick_llama_mode(compute_backend)
    }

    /// Start (or re-start) the runtime for `flow_model`, choosing GPU vs CPU
    /// based on `compute_backend` ("auto" | "cpu" | "gpu"). If the GPU mode
    /// was requested but the binary fails to start (e.g. the shipped
    /// `llama-server` was built without CUDA support), this transparently
    /// falls back to CPU, mirroring the audio sidecar.
    ///
    /// `override_layers`, when `Some`, preserves an explicit legacy partial
    /// offload. `None` lets the backend choose the largest safe full or partial
    /// offload from live free VRAM. A change in mode, override, or reserve
    /// triggers a relaunch.
    pub fn ensure(
        &self,
        flow_model: &str,
        compute_backend: &str,
        override_layers: Option<u32>,
        vram_reserve_mb: u32,
        context_size: u32,
    ) -> Result<(), String> {
        let _runtime_launch = crate::rewrite::runtime_inventory::launch_guard()?;
        let _operation = self.operation.lock();
        if flow_model == "none" {
            self.shutdown_inner();
            return Ok(());
        }
        let mode = pick_llama_mode(compute_backend);
        // Only refuse when the installed flavor is *known* and wrong.
        //
        // A missing `llama-server.kind` marker means "we cannot tell", not
        // "wrong flavor". Treating the two the same permanently disabled
        // refinement for anyone whose install wrote the binary but not the
        // marker — the install does them in that order, so an interruption in
        // between leaves a perfectly good llama-server that this check refused
        // to touch, with a full re-download as the only recovery. Launch is the
        // real backend validation anyway: it already retries on CPU when the
        // GPU attempt fails.
        if mode.is_gpu()
            && crate::rewrite::runtime_install::runtime_flavor_conflicts(compute_backend)
        {
            return Err(self.fail_ensure(FlowLaunchFailure::RuntimeFlavorMismatch));
        }
        // A context size of zero means "unset"; fall back to the shipped default
        // rather than handing llama-server an invalid flag.
        let context_size = if context_size == 0 {
            1024
        } else {
            context_size
        };
        let request = RuntimeRequest {
            flow_model: flow_model.to_string(),
            requested_mode: mode.clone(),
            override_layers,
            vram_reserve_mb,
            context_size,
        };
        // Compare the request, not the outcome. A GPU request that fell back
        // to CPU must cache-hit here, or every dictation pays a full
        // llama-server restart.
        if self.status_ready() && self.active_request.read().as_ref() == Some(&request) {
            return Ok(());
        }
        if self.status_ready() || self.active_model.read().is_some() {
            self.shutdown_inner();
        }
        // Mark the runtime as starting so the UI can show a transient
        // "loading" state. Cleared by `shutdown()` on every exit path.
        self.is_starting.store(true, Ordering::Release);

        let bin = llama_server_bin();
        let gguf = self.model_path(flow_model);
        if !bin.is_file() {
            return Err(self.fail_ensure(FlowLaunchFailure::BinaryMissing));
        }
        if !gguf.is_file()
            || self
                .model_paths
                .as_ref()
                .is_some_and(|(_, mmproj)| !mmproj.is_file())
        {
            return Err(self.fail_ensure(FlowLaunchFailure::ModelMissing));
        }

        // Automatic GPU intent is resolved only after the request-cache check,
        // so a warm runtime never re-probes VRAM or relaunches on every
        // dictation. Live free VRAM already includes the resident ASR model.
        let resolved_gpu_layers = if mode.is_gpu() {
            match override_layers {
                Some(layers) => layers.min(FULL_GPU_OFFLOAD),
                None => latency_optimized_gpu_layers(
                    flow_model,
                    &capabilities_uncached(),
                    vram_reserve_mb,
                    context_size,
                ),
            }
        } else {
            0
        };

        // Try the requested GPU mode first when at least one layer can be
        // admitted, then retry once on CPU. If current VRAM cannot safely hold
        // even one layer, skip the guaranteed-failing GPU launch.
        let attempts: Vec<LlamaMode> = if mode.is_gpu() && resolved_gpu_layers > 0 {
            vec![mode.clone(), LlamaMode::Cpu]
        } else {
            vec![LlamaMode::Cpu]
        };
        let mut last_err: Option<FlowLaunchFailure> = None;
        for attempt in attempts {
            let layers_for_attempt = if attempt.is_gpu() {
                Some(resolved_gpu_layers)
            } else {
                Some(0)
            };
            match self.launch(flow_model, &attempt, layers_for_attempt, context_size) {
                Ok(()) => {
                    *self.active_mode.write() = Some(attempt);
                    *self.active_n_gpu_layers.write() = layers_for_attempt;
                    *self.active_request.write() = Some(request);
                    self.is_starting.store(false, Ordering::Release);
                    *self.last_error.write() = last_err.as_ref().map(|failure| {
                        let warning = format!(
                            "GPU launch failed [{}]: {}; using CPU",
                            failure.kind(),
                            failure
                        );
                        log::warn!("{warning}");
                        warning
                    });
                    return Ok(());
                }
                Err(failure) => {
                    log::warn!(
                        "llama-server launch attempt ({}) failed [{}]: {}",
                        attempt.backend_label(),
                        failure.kind(),
                        failure
                    );
                    last_err = Some(failure);
                }
            }
        }
        let failure = last_err.unwrap_or(FlowLaunchFailure::SpawnFailed(
            "no launch attempt was made".into(),
        ));
        Err(self.fail_ensure(failure))
    }

    /// Record a launch failure, reset the client, and return the user-facing
    /// message. Keeps every `ensure()` failure path consistent.
    fn fail_ensure(&self, failure: FlowLaunchFailure) -> String {
        *self.client.write() = FlowClient::new_missing();
        self.is_starting.store(false, Ordering::Release);
        let message = failure.to_string();
        *self.last_error.write() = Some(message.clone());
        message
    }

    /// Evaluate the shared prompt prefix once, so the first real dictation does
    /// not pay for it.
    ///
    /// Every rewrite sends an identical system + few-shot prefix and differs
    /// only in the final user turn, so llama.cpp can serve the prefix from its
    /// slot cache. The catch is who pays for the first evaluation: prompt
    /// processing measures ~28 tokens/second when the model is on CPU, which is
    /// several seconds for the prefix — and unwarmed, that lands on the user
    /// while they wait for their first transcript. Worse, it exceeded the
    /// refinement deadline outright, so the rewrite was abandoned and the
    /// dictation silently went unpolished.
    ///
    /// Failure here is not an error: the runtime is up and usable either way, so
    /// this only ever costs the warm-up itself.
    pub fn warm_prompt_cache(&self, flow_model: &str) {
        let client = self.client.read().clone();
        if client.base_url.is_none() {
            return;
        }
        let started = std::time::Instant::now();
        // Deliberately trivial input: the point is the prefix, not the answer.
        let request = crate::rewrite::RewriteRequest {
            text: "ok".into(),
            cleanup_level: "high".into(),
            style: "neutral".into(),
            dictation_mode: "normal".into(),
            vocabulary: Vec::new(),
            app_process: String::new(),
            model_id: flow_model.to_string(),
        };
        match client.rewrite(&request) {
            Ok(_) => log::info!(
                "Refinement prompt cache warmed in {} ms",
                started.elapsed().as_millis()
            ),
            Err(err) => log::warn!(
                "Refinement prompt cache warm-up failed after {} ms ({err}); \
                 the first dictation will pay the prefix evaluation",
                started.elapsed().as_millis()
            ),
        }
    }

    /// Launch `llama-server`, retrying on a fresh port if the port we probed
    /// was taken in the window between our probe bind and the child's bind.
    fn launch(
        &self,
        flow_model: &str,
        mode: &LlamaMode,
        override_layers: Option<u32>,
        context_size: u32,
    ) -> Result<(), FlowLaunchFailure> {
        const PORT_ATTEMPTS: usize = 4;
        let mut last = FlowLaunchFailure::SpawnFailed("no port could be reserved".into());
        for attempt in 0..PORT_ATTEMPTS {
            let port = match reserve_port() {
                Ok(port) => port,
                Err(err) => {
                    last = FlowLaunchFailure::SpawnFailed(err);
                    continue;
                }
            };
            let outcome =
                self.launch_on_port(flow_model, mode, override_layers, context_size, port);
            // All launch failures must reap the previous attempt before a
            // retry can overwrite its child handle and leave an orphan.
            if outcome.is_err() {
                self.stop_child();
            }
            match outcome {
                Ok(()) => return Ok(()),
                Err(failure) if failure.is_port_conflict() && attempt + 1 < PORT_ATTEMPTS => {
                    log::warn!("llama-server could not bind port {port}; retrying on a new port");
                    last = failure;
                }
                Err(failure) => return Err(failure),
            }
        }
        Err(last)
    }

    fn launch_on_port(
        &self,
        flow_model: &str,
        mode: &LlamaMode,
        override_layers: Option<u32>,
        context_size: u32,
        port: u16,
    ) -> Result<(), FlowLaunchFailure> {
        let bin = llama_server_bin();
        let gguf = self.model_path(flow_model);
        self.stderr_ring.lock().clear();

        let n_layers = mode.n_gpu_layers(override_layers);
        let mut cmd = Command::new(&bin);
        cmd.arg("-m")
            .arg(&gguf)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string())
            .arg("--n-gpu-layers")
            .arg(n_layers.to_string())
            // The user-configured window, not a constant. This was hard-coded to
            // 1024, which meant Settings offered a context-size control that
            // llama-server never saw — and a long dictation was silently
            // truncated to fit a window the user thought they had raised.
            .arg("--ctx-size")
            .arg(context_size.to_string())
            .arg("--parallel")
            .arg("1")
            .arg("--jinja")
            // Qwen3.5 is a hybrid-reasoning family: left to itself it spends the
            // entire completion budget inside a <think> block and returns an
            // empty `message.content`. Measured on Qwen3.5-0.8B Q4_K_M:
            // 15.06s and "" without this flag, 0.21s and a correct rewrite with
            // it. Dictation polishing has no use for a reasoning trace, so
            // thinking is disabled outright rather than merely budgeted.
            .arg("--reasoning")
            .arg("off")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            // Capture stderr instead of discarding it. Without this the only
            // signal on failure was "exited before becoming ready", with no
            // llama.cpp output at all.
            .stderr(Stdio::piped());
        // Pin `--threads` to the physical core count when the CPU runs real
        // layers; leave it off when the whole model is on the GPU. The probe
        // is cached — `ensure()` read it moments ago for the launch decision,
        // so port-conflict retries do not re-probe.
        if let Some(threads) = llama_server_threads(
            mode,
            n_layers,
            flow_model_spec(flow_model).gpu_layer_count,
            &capabilities(),
        ) {
            cmd.arg("--threads").arg(threads.to_string());
        }
        if let Some((_, mmproj)) = &self.model_paths {
            cmd.arg("--mmproj").arg(mmproj);
            if !mode.is_gpu() {
                cmd.arg("--no-mmproj-offload");
            }
        }
        // On a multi-GPU host, pin llama-server to GPU 0 by default. The user
        // can layer-offload around this with `--n-gpu-layers`; explicit
        // `--tensor-split` is intentionally out of scope for v1.
        if mode.is_gpu() {
            cmd.arg("--main-gpu")
                .arg("0")
                .arg("--split-mode")
                .arg("none");
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| FlowLaunchFailure::SpawnFailed(e.to_string()))?;
        if let Some(stderr) = child.stderr.take() {
            let ring = Arc::clone(&self.stderr_ring);
            std::thread::Builder::new()
                .name("llama-server-stderr".into())
                .spawn(move || {
                    for line in BufReader::new(stderr).lines() {
                        match line {
                            Ok(line) => {
                                log::debug!("llama-server: {line}");
                                ring.lock().push(line);
                            }
                            Err(_) => break,
                        }
                    }
                })
                .ok();
        }
        *self.child.lock() = Some(child);

        let url = format!("http://127.0.0.1:{port}");
        // The client's completion budget has to match the window this server was
        // actually launched with, or a long rewrite is refused or truncated.
        let client =
            FlowClient::new_url(url.clone(), flow_http_timeout()).with_context_size(context_size);
        let health = format!("{url}/health");
        // Cold GPU shader compilation and CPU weight loading both need time
        // before we declare the launch a failure and fall back.
        let timeout_secs: u64 = 30;
        let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
        let http = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(300))
            .build()
            .map_err(|e| FlowLaunchFailure::SpawnFailed(e.to_string()))?;
        while std::time::Instant::now() < deadline {
            // If the child has already exited, surface that as a launch
            // failure so the GPU→CPU fallback can take over.
            let exited = {
                let mut guard = self.child.lock();
                match guard.as_mut() {
                    Some(child) => child
                        .try_wait()
                        .map_err(|e| FlowLaunchFailure::SpawnFailed(e.to_string()))?,
                    None => None,
                }
            };
            if let Some(status) = exited {
                // Give the stderr reader a moment to flush the reason.
                std::thread::sleep(Duration::from_millis(50));
                return Err(FlowLaunchFailure::ExitedEarly {
                    status: format!("{status}"),
                    stderr: self.stderr_ring.lock().tail(20),
                });
            }
            if http
                .get(&health)
                .send()
                .map(|r| r.status().is_success())
                .unwrap_or(false)
            {
                *self.active_model.write() = Some(flow_model.to_string());
                *self.client.write() = client;
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        // Timed out waiting for health endpoint. Tear down the child and
        // surface an error so the caller can decide whether to retry on CPU.
        if let Some(mut child) = self.child.lock().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        Err(FlowLaunchFailure::HealthTimeout {
            stderr: self.stderr_ring.lock().tail(20),
        })
    }
}

/// Ask the OS for a free loopback port.
///
/// There is an unavoidable race here: we cannot hand a bound listener to
/// `llama-server`, so the port can be taken between this call and the child's
/// own bind. The caller retries on a fresh port when that happens instead of
/// treating it as a hard launch failure.
fn reserve_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    drop(listener);
    Ok(port)
}

pub fn llama_server_bin() -> PathBuf {
    // Delegate to the runtime-install module so the env-var override
    // (`REFLOW_LLAMA_BIN`) is honored everywhere in the codebase.
    crate::rewrite::runtime_install::llama_server_bin()
}

/// Kill `llama-server` processes left behind by a previous Reflow run.
///
/// A force-killed or crashed Reflow never runs `FlowRuntime::drop`, so its
/// `llama-server` child is orphaned — still holding hundreds of MB of VRAM,
/// still bound to its port. On a 4 GB card two orphans are the difference
/// between a model loading on the GPU and the whole load failing. Measured
/// on this machine: after two force-killed runs, two orphaned servers held
/// ~1.4 GB, free VRAM read 1440 MB, and the next launch's prompt-cache
/// warm-up timed out.
///
/// Only processes whose parent is gone are touched, so a server legitimately
/// spawned by the running instance (or by anything else alive) is safe.
pub fn kill_orphaned_llama_servers() -> usize {
    use sysinfo::{ProcessesToUpdate, System};
    let own_pid = std::process::id();
    let managed_bin = PlatformSys::get_app_dir().join("bin");

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing().with_exe(sysinfo::UpdateKind::Always),
    );

    let mut killed = 0usize;
    for (pid, process) in system.processes() {
        let name = process.name().to_string_lossy().to_ascii_lowercase();
        // Windows ships `llama-server.exe`; Linux/macOS ship `llama-server`.
        // Matching only the `.exe` name leaked VRAM and ports on Linux after
        // every crash.
        if name != "llama-server.exe" && name != "llama-server" {
            continue;
        }
        let Some(parent) = process.parent() else {
            continue;
        };
        if !is_managed_orphan(
            process.exe(),
            &managed_bin,
            parent.as_u32(),
            own_pid,
            system.process(parent).is_some(),
        ) {
            continue;
        }
        log::warn!(
            "Killing orphaned llama-server (pid {}, parent {} is gone)",
            pid.as_u32(),
            parent.as_u32()
        );
        if process.kill() {
            killed += 1;
        }
    }
    killed
}

fn is_managed_orphan(
    executable: Option<&std::path::Path>,
    managed_bin: &std::path::Path,
    parent: u32,
    own_pid: u32,
    parent_alive: bool,
) -> bool {
    if parent == own_pid || parent_alive {
        return false;
    }
    let Some(executable) = executable else {
        return false;
    };
    // Never terminate another application's server or an unverifiable process.
    executable.parent() == Some(managed_bin)
}

pub fn flow_gguf_path(flow_model: &str) -> PathBuf {
    let file = flow_model_spec(flow_model).filename;
    PlatformSys::get_models_dir().join("flow").join(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rewrite::runtime_install::runtime_flavors_conflict;

    /// An install writes the binary and *then* its flavor marker. Interrupted
    /// in between, the result is a working llama-server with no marker — which
    /// `runtime_matches` reports as "not a match" because it cannot prove
    /// otherwise. Gating the launch on that answer disabled refinement
    /// permanently, with a full re-download as the only recovery. Only a
    /// positively-known wrong flavor may block a launch.
    #[test]
    fn an_unlabelled_runtime_is_not_treated_as_a_conflict() {
        for expected in ["Vulkan", "CPU"] {
            assert!(!runtime_flavors_conflict(Some(expected), None));
            assert!(!runtime_flavors_conflict(Some(expected), Some(expected)));
        }
        assert!(runtime_flavors_conflict(Some("CPU"), Some("Vulkan")));
        assert!(!runtime_flavors_conflict(None, Some("Vulkan")));
    }

    #[test]
    fn flow_registry_resolves_tier_models() {
        assert_eq!(
            flow_model_spec("qwen3.5-0.8b").filename,
            "Qwen3.5-0.8B-Q4_K_M.gguf"
        );
        assert_eq!(flow_model_spec("unknown").id, "none");
    }

    /// `Qwen/Qwen3.5-2B-GGUF` does not exist on the Hub, so the deep_context
    /// tier could never install. Pin the real repo and its real filename.
    #[test]
    fn deep_context_model_points_at_a_real_repo() {
        let spec = flow_model_spec("qwen3.5-2b");
        assert_eq!(spec.repo, "unsloth/Qwen3.5-2B-GGUF");
        assert_eq!(spec.filename, "Qwen3.5-2B-Q4_K_M.gguf");
        assert!(
            !spec.filename.contains("-Instruct-"),
            "the unsloth GGUFs have no -Instruct infix"
        );
        // Every installable tier must declare a repo and a filename.
        for spec in FLOW_MODELS.iter().filter(|s| s.id != "none") {
            assert!(!spec.repo.is_empty(), "{} has no repo", spec.id);
            assert!(!spec.filename.is_empty(), "{} has no filename", spec.id);
            assert!(spec.approx_bytes > 0, "{} has no size estimate", spec.id);
        }
    }

    /// GPU intent is admitted only when an adapter is actually enumerated;
    /// launch remains the final backend validation and retains CPU fallback.
    #[test]
    fn gpu_request_requires_detected_hardware() {
        let mut caps = Capabilities::default();
        assert_eq!(
            pick_llama_mode_for_capabilities("gpu", &caps),
            LlamaMode::Cpu
        );

        caps.gpus.push(crate::capability::GpuInfo {
            index: 0,
            name: "Test GPU".into(),
            vendor: crate::capability::GpuVendor::Nvidia,
            total_vram_mb: 4096.0,
            used_vram_mb: 0.0,
            free_vram_mb: 4096.0,
            driver_version: None,
            compute_capability: None,
        });
        assert!(pick_llama_mode_for_capabilities("gpu", &caps).is_gpu());
        assert!(pick_llama_mode_for_capabilities("vulkan", &caps).is_gpu());
        assert_eq!(
            pick_llama_mode_for_capabilities("cpu", &caps),
            LlamaMode::Cpu
        );

        assert_eq!(
            latency_optimized_gpu_layers("qwen3.5-0.8b", &caps, 0, 1024),
            FULL_GPU_OFFLOAD,
            "roomy VRAM should use full offload"
        );
        // Derived from the registry rather than hard-coded: this assertion used
        // to spell out the previous model's 16 blocks, so swapping in a model
        // with a different layer count failed the test for no real reason.
        // 1300 MB free leaves 1300 - 512 (reserve) - 256 (transient) - 192
        // (runtime) = 340 MB against ~559 MB of weights, so a partial offload is
        // the only option. A larger figure now buys full offload outright, the
        // model being 508 MB rather than the 790 MB one this replaced.
        let layers = flow_model_spec("qwen3.5-0.8b").gpu_layer_count;
        caps.gpus[0].free_vram_mb = 1300.0;
        let partial = latency_optimized_gpu_layers("qwen3.5-0.8b", &caps, 0, 1024);
        assert!(
            (1..layers).contains(&partial),
            "constrained VRAM should retain a useful partial offload, \
            got {partial} of {layers}"
        );
        let large_context = latency_optimized_gpu_layers("qwen3.5-0.8b", &caps, 0, 8192);
        assert!(
            large_context < partial,
            "larger contexts must leave less memory for weights"
        );
    }

    /// The regression that relaunched llama-server on every dictation: a GPU
    /// request that fell back to CPU stored `Cpu` / `Some(0)` while
    /// `pick_llama_mode` kept returning `Gpu`, so an outcome-based cache key
    /// never matched.
    #[test]
    fn cache_key_is_the_request_not_the_fallback_outcome() {
        let requested = RuntimeRequest {
            flow_model: "qwen3.5-0.8b".into(),
            requested_mode: LlamaMode::Gpu("RTX 2050".into()),
            override_layers: None,
            vram_reserve_mb: 0,
            context_size: 1024,
        };

        let runtime = FlowRuntime::default();
        // Simulate the post-fallback state: the child runs on CPU with zero
        // offloaded layers, but the *request* was GPU/auto.
        *runtime.active_mode.write() = Some(LlamaMode::Cpu);
        *runtime.active_n_gpu_layers.write() = Some(0);
        *runtime.active_request.write() = Some(requested.clone());

        assert_eq!(
            runtime.active_request.read().as_ref(),
            Some(&requested),
            "the same request must compare equal on the next dictation"
        );
        assert_ne!(
            runtime.active_mode.read().clone(),
            Some(requested.requested_mode.clone()),
            "outcome and request genuinely differ after a fallback"
        );

        // A different request must not hit the cache.
        let other = RuntimeRequest {
            override_layers: Some(12),
            ..requested.clone()
        };
        assert_ne!(runtime.active_request.read().as_ref(), Some(&other));
    }

    #[test]
    fn shutdown_clears_the_cache_key() {
        let runtime = FlowRuntime::default();
        *runtime.active_request.write() = Some(RuntimeRequest {
            flow_model: "qwen3.5-0.8b".into(),
            requested_mode: LlamaMode::Cpu,
            override_layers: Some(0),
            vram_reserve_mb: 0,
            context_size: 1024,
        });
        runtime.shutdown();
        assert!(runtime.active_request.read().is_none());
    }

    #[test]
    fn a_cached_url_without_a_live_child_is_not_ready() {
        let runtime = FlowRuntime::default();
        *runtime.client.write() =
            FlowClient::new_url("http://127.0.0.1:9".into(), Duration::from_secs(1));
        assert!(!runtime.status_ready());
    }

    #[test]
    fn an_exited_child_invalidates_runtime_readiness() {
        #[cfg(windows)]
        let mut cmd = {
            use std::os::windows::process::CommandExt;
            let mut cmd = Command::new("cmd");
            cmd.args(["/C", "exit", "0"]);
            cmd.creation_flags(0x08000000);
            cmd
        };
        #[cfg(not(windows))]
        let mut cmd = {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "exit 0"]);
            cmd
        };
        let mut child = cmd
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child.wait().unwrap();
        let runtime = FlowRuntime::default();
        *runtime.client.write() =
            FlowClient::new_url("http://127.0.0.1:9".into(), Duration::from_secs(1));
        *runtime.child.lock() = Some(child);
        assert!(
            !runtime.status_ready(),
            "crashed server must not keep hitting the request cache"
        );
    }

    #[test]
    fn launch_failures_are_distinguishable() {
        assert_eq!(FlowLaunchFailure::BinaryMissing.kind(), "binary_missing");
        assert_eq!(FlowLaunchFailure::ModelMissing.kind(), "model_missing");
        assert_eq!(
            FlowLaunchFailure::HealthTimeout {
                stderr: String::new()
            }
            .kind(),
            "health_timeout"
        );

        // A bind failure reported through an early exit must be recognised as
        // a port conflict so the caller retries instead of falling back to CPU.
        let bind = FlowLaunchFailure::ExitedEarly {
            status: "exit code: 1".into(),
            stderr: "error: failed to bind: Address already in use".into(),
        };
        assert!(bind.is_port_conflict());

        // A CUDA failure is not a port conflict; it must reach the CPU ladder.
        let cuda = FlowLaunchFailure::ExitedEarly {
            status: "exit code: 1".into(),
            stderr: "CUDA error: no kernel image is available".into(),
        };
        assert!(!cuda.is_port_conflict());
        assert!(
            cuda.to_string().contains("no kernel image"),
            "the real llama.cpp error must survive into the message: {cuda}"
        );
    }

    #[test]
    fn stderr_ring_is_bounded_and_keeps_the_tail() {
        let mut ring = StderrRing::default();
        for i in 0..(STDERR_RING_LINES + 50) {
            ring.push(format!("line {i}"));
        }
        let text = ring.text();
        assert!(!text.contains("line 0"), "oldest lines must be evicted");
        assert!(text.contains(&format!("line {}", STDERR_RING_LINES + 49)));
        assert_eq!(ring.tail(2).lines().count(), 2);
        ring.clear();
        assert!(ring.is_empty());
    }

    #[test]
    fn reserve_port_returns_a_usable_loopback_port() {
        let a = reserve_port().expect("reserve");
        let b = reserve_port().expect("reserve");
        assert!(a > 0 && b > 0);
        // Re-binding a reserved port must succeed, proving it was released.
        let listener = TcpListener::bind(("127.0.0.1", a)).expect("rebind reserved port");
        drop(listener);
    }

    #[test]
    fn rewrite_client_timeout_is_long_enough_for_cpu() {
        let timeout = flow_http_timeout();
        assert!(
            timeout >= Duration::from_secs(8),
            "CPU llama-server rewrites need seconds, got {timeout:?}"
        );
        let client = FlowClient::new_url("http://127.0.0.1:9".into(), timeout);
        assert_eq!(client.timeout, timeout);
        assert_eq!(client.timeout, FlowClient::new_missing().timeout);
    }

    #[test]
    fn orphan_selection_only_accepts_managed_processes_with_dead_parents() {
        let managed = std::path::Path::new("reflow/bin");
        let owned = managed.join("llama-server.exe");
        let other = std::path::Path::new("another-app/bin/llama-server.exe");
        assert!(is_managed_orphan(Some(&owned), managed, 42, 1, false));
        assert!(!is_managed_orphan(Some(&owned), managed, 42, 1, true));
        assert!(!is_managed_orphan(Some(&owned), managed, 1, 1, false));
        assert!(!is_managed_orphan(Some(other), managed, 42, 1, false));
        assert!(!is_managed_orphan(None, managed, 42, 1, false));
    }

    /// `--threads` must be the physical core count when the CPU does real
    /// work, and absent when the GPU holds the whole model.
    #[test]
    fn llama_threads_track_physical_cores_and_offload() {
        let mut caps = Capabilities {
            cpu: crate::capability::CpuInfo {
                physical_cores: Some(6),
                logical_cores: 12,
                ..Default::default()
            },
            ..Default::default()
        };
        // CPU-only: physical, not logical.
        assert_eq!(llama_server_threads(&LlamaMode::Cpu, 0, 0, &caps), Some(6));
        // Partial offload still runs the remaining layers on the CPU.
        let gpu = LlamaMode::Gpu("Test GPU".into());
        assert_eq!(llama_server_threads(&gpu, 12, 24, &caps), Some(6));
        // Full offload, either via the 99 convention or an explicit layer
        // count equal to the model's: `-t` drives nothing, so it stays off.
        assert_eq!(
            llama_server_threads(&gpu, FULL_GPU_OFFLOAD, 24, &caps),
            None
        );
        assert_eq!(llama_server_threads(&gpu, 24, 24, &caps), None);
        // An OS that will not report physical cores gets the conservative
        // logical-minus-one answer the rest of the codebase already uses.
        caps.cpu.physical_cores = None;
        assert_eq!(llama_server_threads(&LlamaMode::Cpu, 0, 0, &caps), Some(11));
    }

    #[test]
    fn ensure_missing_binary_or_model_fails_fast() {
        let runtime = FlowRuntime::default();
        let start = std::time::Instant::now();
        let err = runtime
            .ensure("non-existent-model", "cpu", None, 0, 1024)
            .unwrap_err();
        let elapsed = start.elapsed();
        // The point is that ensure() must not hang or spawn anything: a real
        // failure path is a couple of stat() calls. The budget still has to
        // tolerate a cold filesystem cache, which on Windows can push two
        // stat calls over 2s while the rest of the suite runs in parallel.
        assert!(elapsed < std::time::Duration::from_secs(5));
        assert!(
            err.contains("not installed") || err.contains("missing") || err.contains("not found")
        );
        assert!(!runtime.status_ready());
    }
}
