use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::{Mutex, RwLock};

use super::client::FlowClient;
use crate::platform::PlatformSys;

pub struct FlowModelSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub filename: &'static str,
    pub repo: &'static str,
    pub approx_bytes: u64,
}

/// Pick a `llama-server` execution mode from the user's `compute_backend`
/// setting and the actual hardware on this machine.
///
/// Mirrors the audio sidecar's `pick_device` semantics:
///   * `"auto"` → use the GPU if one is present, otherwise fall back to CPU.
///   * `"cpu"`  → always run on the CPU.
///   * `"gpu"`  → require the GPU; if no GPU is detected we still attempt the
///                launch (the user explicitly asked for it) but flip to CPU
///                automatically if llama-server exits before becoming ready,
///                matching the audio sidecar's fallback behaviour.
pub fn pick_llama_mode(requested: &str) -> LlamaMode {
    let r = requested.trim().to_ascii_lowercase();
    let (gpu_name, _) = PlatformSys::detect_gpu();
    let gpu_available = !gpu_name.is_empty() && gpu_name != "CPU";

    match r.as_str() {
        "cpu" => LlamaMode::Cpu,
        "gpu" | "cuda" => {
            if gpu_available {
                LlamaMode::Gpu(gpu_name)
            } else {
                // User asked for GPU but none is present. The startup-fallback
                // path will retry on CPU if the child exits early.
                LlamaMode::Gpu(gpu_name)
            }
        }
        _ => {
            // "auto" and any unknown value: prefer GPU when available.
            if gpu_available {
                LlamaMode::Gpu(gpu_name)
            } else {
                LlamaMode::Cpu
            }
        }
    }
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

pub const FLOW_MODELS: [FlowModelSpec; 3] = [
    FlowModelSpec {
        id: "none",
        label: "None",
        filename: "",
        repo: "",
        approx_bytes: 0,
    },
    FlowModelSpec {
        id: "lfm2.5-1.2b",
        label: "LFM2.5 1.2B Instruct",
        filename: "LFM2.5-1.2B-Instruct-Q4_K_M.gguf",
        repo: "LiquidAI/LFM2.5-1.2B-Instruct-GGUF",
        approx_bytes: 790_000_000,
    },
    FlowModelSpec {
        id: "qwen3.5-2b",
        label: "Qwen3.5 2B Instruct",
        filename: "Qwen3.5-2B-Instruct-Q4_K_M.gguf",
        repo: "Qwen/Qwen3.5-2B-GGUF",
        approx_bytes: 1_380_000_000,
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

pub struct FlowRuntime {
    child: Mutex<Option<Child>>,
    pub client: parking_lot::RwLock<FlowClient>,
    pub active_model: RwLock<Option<String>>,
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
    /// Last `ensure()` error. Cleared on every successful launch and on
    /// `shutdown()`. Surfaces in `FlowStatus.last_error` so the UI can
    /// show a one-line "LLM runtime error" hint with a Reinstall button.
    pub last_error: RwLock<Option<String>>,
}

impl Default for FlowRuntime {
    fn default() -> Self {
        Self {
            child: Mutex::new(None),
            client: parking_lot::RwLock::new(FlowClient::new_missing()),
            active_model: RwLock::new(None),
            active_mode: RwLock::new(None),
            active_n_gpu_layers: RwLock::new(None),
            is_starting: AtomicBool::new(false),
            last_error: RwLock::new(None),
        }
    }
}

impl Drop for FlowRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl FlowRuntime {
    pub fn status_ready(&self) -> bool {
        self.client.read().base_url.is_some()
    }

    pub fn active_model(&self) -> Option<String> {
        self.active_model.read().clone()
    }

    pub fn active_mode(&self) -> Option<LlamaMode> {
        self.active_mode.read().clone()
    }

    pub fn active_n_gpu_layers(&self) -> Option<u32> {
        self.active_n_gpu_layers.read().clone()
    }

    pub fn is_starting(&self) -> bool {
        self.is_starting.load(Ordering::Acquire)
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.read().clone()
    }

    pub fn shutdown(&self) {
        if let Some(mut child) = self.child.lock().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        *self.client.write() = FlowClient::new_missing();
        *self.active_model.write() = None;
        *self.active_mode.write() = None;
        *self.active_n_gpu_layers.write() = None;
        self.is_starting.store(false, Ordering::Release);
        *self.last_error.write() = None;
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
    /// `override_layers`, when `Some`, forces a specific `--n-gpu-layers`
    /// value. `None` keeps the legacy 0/99 choice tied to `compute_backend`.
    /// A change in either the mode or the override triggers a relaunch.
    pub fn ensure(
        &self,
        flow_model: &str,
        compute_backend: &str,
        override_layers: Option<u32>,
    ) -> Result<(), String> {
        if flow_model == "none" {
            self.shutdown();
            return Ok(());
        }
        let mode = pick_llama_mode(compute_backend);
        if self.status_ready()
            && self.active_model.read().as_deref() == Some(flow_model)
            && self.active_mode.read().as_ref() == Some(&mode)
            && self.active_n_gpu_layers.read().as_ref().copied() == override_layers
        {
            return Ok(());
        }
        if self.status_ready() || self.active_model.read().is_some() {
            self.shutdown();
        }
        // Mark the runtime as starting so the UI can show a transient
        // "loading" state. Cleared by `shutdown()` on every exit path.
        self.is_starting.store(true, Ordering::Release);

        let bin = llama_server_bin();
        let gguf = flow_gguf_path(flow_model);
        if !bin.exists() {
            *self.client.write() = FlowClient::new_missing();
            self.is_starting.store(false, Ordering::Release);
            let err = "llama-server is not installed".to_string();
            *self.last_error.write() = Some(err.clone());
            return Err(err);
        }
        if !gguf.exists() {
            *self.client.write() = FlowClient::new_missing();
            self.is_starting.store(false, Ordering::Release);
            let err = "Flow model is not installed".to_string();
            *self.last_error.write() = Some(err.clone());
            return Err(err);
        }

        // Try the requested mode first; if it was a GPU request that fails
        // (binary built without CUDA, or no driver), retry once on CPU. The
        // CPU-only path is always safe. CPU fallback also drops the user's
        // layer override so the binary doesn't try to offload layers to a
        // non-existent GPU.
        let attempts: Vec<LlamaMode> = if mode.is_gpu() {
            vec![mode.clone(), LlamaMode::Cpu]
        } else {
            vec![mode.clone()]
        };
        let mut last_err: Option<String> = None;
        for attempt in attempts {
            let layers_for_attempt = if attempt.is_gpu() {
                override_layers
            } else {
                Some(0)
            };
            match self.launch(flow_model, &attempt, layers_for_attempt) {
                Ok(()) => {
                    *self.active_mode.write() = Some(attempt);
                    *self.active_n_gpu_layers.write() = layers_for_attempt;
                    self.is_starting.store(false, Ordering::Release);
                    *self.last_error.write() = None;
                    return Ok(());
                }
                Err(err) => {
                    log::warn!(
                        "llama-server launch attempt ({}) failed: {}",
                        attempt.backend_label(),
                        err
                    );
                    last_err = Some(err);
                }
            }
        }
        *self.client.write() = FlowClient::new_missing();
        self.is_starting.store(false, Ordering::Release);
        let err = last_err.unwrap_or_else(|| "llama-server failed to start".into());
        *self.last_error.write() = Some(err.clone());
        Err(err)
    }

    fn launch(
        &self,
        flow_model: &str,
        mode: &LlamaMode,
        override_layers: Option<u32>,
    ) -> Result<(), String> {
        let bin = llama_server_bin();
        let gguf = flow_gguf_path(flow_model);
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        drop(listener);

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
            .arg("--ctx-size")
            .arg("1024")
            .arg("--parallel")
            .arg("1")
            .arg("--jinja")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // On a multi-GPU host, pin llama-server to GPU 0 by default. The user
        // can layer-offload around this with `--n-gpu-layers`; explicit
        // `--tensor-split` is intentionally out of scope for v1.
        if mode.is_gpu() {
            cmd.arg("--main-gpu").arg("0");
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let child = cmd
            .spawn()
            .map_err(|e| format!("failed to start llama-server: {e}"))?;
        *self.child.lock() = Some(child);

        let url = format!("http://127.0.0.1:{port}");
        let client = FlowClient::new_url(url.clone(), flow_http_timeout());
        let health = format!("{url}/health");
        // CPU llama-server can take noticeably longer to load weights than
        // the GPU build; give the slower path a wider window before we declare
        // the launch a failure and fall back.
        let timeout_secs: u64 = if mode.is_gpu() { 8 } else { 30 };
        let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(300))
            .build()
            .map_err(|e| e.to_string())?;
        while std::time::Instant::now() < deadline {
            // If the child has already exited, surface that as a launch
            // failure so the GPU→CPU fallback can take over.
            if let Some(child) = self.child.lock().as_mut() {
                if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                    return Err(format!(
                        "llama-server exited before becoming ready (status {status:?})"
                    ));
                }
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
        Err("llama-server did not become ready".into())
    }
}

pub fn llama_server_bin() -> PathBuf {
    // Delegate to the runtime-install module so the env-var override
    // (`REFLOW_LLAMA_BIN`) is honored everywhere in the codebase.
    crate::rewrite::runtime_install::llama_server_bin()
}

pub fn flow_gguf_path(flow_model: &str) -> PathBuf {
    let file = flow_model_spec(flow_model).filename;
    PlatformSys::get_models_dir().join("flow").join(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_registry_resolves_tier_models() {
        assert_eq!(flow_model_spec("lfm2.5-1.2b").filename, "LFM2.5-1.2B-Instruct-Q4_K_M.gguf");
        assert_eq!(flow_model_spec("qwen3.5-2b").repo, "Qwen/Qwen3.5-2B-GGUF");
        assert_eq!(flow_model_spec("unknown").id, "none");
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
}
