use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use serde_json::{json, Value};

use super::engine::{ASREngine, EngineStatus};
use super::mock::MockASREngine;
use super::stabilizer::TranscriptStabilizer;
use crate::audio::resampler::AudioResampler;

const ASR_PUSH_CHUNK_SAMPLES: usize = 16_000;

fn audio_chunks(samples: &[f32]) -> impl Iterator<Item = &[f32]> {
    samples.chunks(ASR_PUSH_CHUNK_SAMPLES)
}

pub struct Qwen3AsrSidecar {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    reader: Option<BufReader<ChildStdout>>,
    backend_name: String,
    detected_language: String,
    stabilizer: TranscriptStabilizer,
    fallback_mock: MockASREngine,
    use_fallback: bool,
    resource_dir: Option<PathBuf>,
    status_cache: EngineStatus,
}

impl Qwen3AsrSidecar {
    pub fn new() -> Self {
        Self {
            child: None,
            stdin: None,
            reader: None,
            backend_name: "Qwen3-ASR (Local CUDA/CPU)".into(),
            detected_language: "en".into(),
            stabilizer: TranscriptStabilizer::new(2),
            fallback_mock: MockASREngine::new(),
            use_fallback: false,
            resource_dir: None,
            status_cache: EngineStatus::default(),
        }
    }

    fn find_python() -> Option<String> {
        let names: &[&str] = if cfg!(windows) {
            &["python", "python3"]
        } else {
            &["python3", "python"]
        };

        for name in names {
            let ok = Command::new(name)
                .arg("-c")
                .arg("import sys; sys.exit(0)")
                .status()
                .map(|status| status.success())
                .unwrap_or(false);
            if ok {
                // The Windows Store python.exe stub "succeeds" then fails to run.
                if let Ok(out) = Command::new(name).arg("-c").arg("import sys; print(sys.executable)").output() {
                    let exe = String::from_utf8_lossy(&out.stdout).to_lowercase();
                    if exe.contains("windowsapps") {
                        continue;
                    }
                }
                return Some((*name).to_string());
            }
        }
        None
    }

    fn find_runtime_script(resource_dir: Option<&Path>) -> Option<PathBuf> {
        let mut candidates = Vec::new();
        if let Some(dir) = resource_dir {
            candidates.push(dir.join("model-runtime").join("qwen3_asr_runtime.py"));
            candidates.push(dir.join("qwen3_asr_runtime.py"));
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                candidates.push(parent.join("model-runtime").join("qwen3_asr_runtime.py"));
                candidates.push(parent.join("../model-runtime").join("qwen3_asr_runtime.py"));
                candidates.push(parent.join("../../model-runtime").join("qwen3_asr_runtime.py"));
                candidates.push(
                    parent
                        .join("../resources/model-runtime")
                        .join("qwen3_asr_runtime.py"),
                );
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join("model-runtime").join("qwen3_asr_runtime.py"));
        }
        if let Some(manifest) = option_env!("CARGO_MANIFEST_DIR") {
            candidates.push(
                PathBuf::from(manifest)
                    .join("../model-runtime")
                    .join("qwen3_asr_runtime.py"),
            );
        }
        candidates.into_iter().find(|path| path.exists())
    }

    fn send_command(&mut self, payload: Value) -> Result<Value, String> {
        let stdin = self.stdin.as_mut().ok_or("Subprocess stdin is not open")?;
        let reader = self.reader.as_mut().ok_or("Subprocess stdout reader is not open")?;

        let line = payload.to_string();
        writeln!(stdin, "{}", line).map_err(|e| format!("Failed to write to sidecar stdin: {}", e))?;
        stdin.flush().map_err(|e| format!("Failed to flush sidecar stdin: {}", e))?;

        let mut response_line = String::new();
        reader
            .read_line(&mut response_line)
            .map_err(|e| format!("Failed to read sidecar stdout: {}", e))?;

        if response_line.trim().is_empty() {
            return Err("Received empty response from sidecar".into());
        }

        let resp: Value = serde_json::from_str(&response_line)
            .map_err(|e| format!("Invalid JSON response: {}: {}", e, response_line))?;

        Ok(resp)
    }

    fn fallback(&mut self, reason: &str) {
        // Never silently swap in the mock engine in a real session — that
        // reports loaded=true and inserts canned text unrelated to the mic.
        log::error!("ASR sidecar unavailable ({reason})");
        self.use_fallback = false;
        self.status_cache.loaded = false;
        self.status_cache.is_loading = false;
        self.status_cache.backend = "unavailable".into();
        self.status_cache.error = Some(reason.to_string());
    }

    fn kill_child(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.stdin = None;
        self.reader = None;
    }
}

impl ASREngine for Qwen3AsrSidecar {
    fn set_resource_dir(&mut self, dir: PathBuf) {
        self.resource_dir = Some(dir);
    }

    fn initialize(&mut self) -> Result<(), String> {
        log::info!("Initializing Qwen3-ASR sidecar process...");

        let Some(python) = Self::find_python() else {
            self.fallback("Python 3 was not found on PATH");
            return Err("Python 3 was not found on PATH. Install Python and restart Reflow.".into());
        };

        let Some(script) = Self::find_runtime_script(self.resource_dir.as_deref()) else {
            self.fallback("qwen3_asr_runtime.py was not found");
            return Err("qwen3_asr_runtime.py was not found.".into());
        };

        log::info!("Spawning ASR sidecar: {} {}", python, script.display());

        // Python can be slow to start (or die) when the machine is under
        // heavy load — e.g. a tauri dev build saturating the CPU. Retry a
        // few times before giving up on the real engine.
        for attempt in 1..=3 {
            let mut cmd = Command::new(&python);
            cmd.arg("-u")
                .arg(&script)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .env("PYTHONUNBUFFERED", "1");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
            }

            match cmd.spawn() {
                Ok(mut child) => {
                    let stdin = child.stdin.take();
                    let stdout = child.stdout.take().map(BufReader::new);

                    self.child = Some(child);
                    self.stdin = stdin;
                    self.reader = stdout;

                    // The sidecar answers ping before importing torch, so
                    // this returns in milliseconds and app startup stays fast.
                    if let Ok(resp) = self.send_command(json!({"cmd": "ping"})) {
                        if resp.get("pong") == Some(&Value::Bool(true)) {
                            log::info!("Qwen3-ASR sidecar ping successful (attempt {attempt})!");
                            let _ = self.refresh_status();
                            return Ok(());
                        }
                    }
                    log::warn!("Sidecar ping failed (attempt {attempt}/3); retrying…");
                    self.kill_child();
                }
                Err(err) => {
                    log::warn!(
                        "Could not spawn python sidecar (attempt {attempt}/3): {err}"
                    );
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }

        self.fallback("sidecar unresponsive after 3 attempts");
        Err("Qwen ASR sidecar failed to start. Check Settings and the qwen_asr.log.".into())
    }

    fn load_model(&mut self, model_dir: &str, backend: &str) -> Result<(), String> {
        // Back-compat: the trait default forwards with precision = "auto".
        self.load_model_with_precision(model_dir, backend, "auto")
    }

    fn load_model_with_precision(
        &mut self,
        model_dir: &str,
        backend: &str,
        precision: &str,
    ) -> Result<(), String> {
        if self.use_fallback {
            return self.fallback_mock.load_model(model_dir, backend);
        }

        let cmd = json!({
            "cmd": "load_model",
            "model_dir": model_dir,
            "device": backend,
            "precision": precision,
        });

        match self.send_command(cmd) {
            Ok(resp) => {
                if resp.get("status") == Some(&Value::String("error".into())) {
                    let err = resp
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown model load error");
                    return Err(err.to_string());
                }
                // "loading" | "ok" | "already-loading" — completion is
                // observable through engine_status().
                self.status_cache.loaded = false;
                self.status_cache.is_loading = true;
                self.status_cache.backend = "loading…".into();
                Ok(())
            }
            Err(e) => Err(format!("Could not reach the ASR sidecar: {e}")),
        }
    }

    fn install_model_dir(&mut self, model_dir: &str, repo: &str) -> Result<(), String> {
        if self.use_fallback {
            return Err("ASR runtime unavailable".into());
        }
        let cmd = json!({
            "cmd": "install_model",
            "model_dir": model_dir,
            "repo": repo
        });
        match self.send_command(cmd) {
            Ok(resp) => {
                if resp.get("status") == Some(&Value::String("error".into())) {
                    let err = resp
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown install error");
                    return Err(err.to_string());
                }
                Ok(())
            }
            Err(e) => Err(format!("Install failed: {e}")),
        }
    }

    fn unload_model(&mut self) -> Result<(), String> {
        if self.use_fallback {
            return self.fallback_mock.unload_model();
        }
        let _ = self.send_command(json!({"cmd": "unload_model"}));
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
        }
        self.stdin = None;
        self.reader = None;
        self.status_cache = EngineStatus::default();
        Ok(())
    }

    fn is_model_loaded(&self) -> bool {
        if self.use_fallback {
            return self.fallback_mock.is_model_loaded();
        }
        self.status_cache.loaded
    }

    fn start_stream(&mut self, language: &str, vocabulary: &[String]) -> Result<(), String> {
        self.stabilizer.reset();
        self.detected_language = if language == "auto" { "en".into() } else { language.to_string() };

        if self.use_fallback {
            return self.fallback_mock.start_stream(language, vocabulary);
        }

        let cmd = json!({
            "cmd": "start_stream",
            "language": language,
            "vocabulary": vocabulary
        });

        match self.send_command(cmd) {
            Ok(resp) => {
                if resp.get("status") == Some(&Value::String("error".into())) {
                    let err = resp
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("start_stream failed");
                    return Err(err.to_string());
                }
                Ok(())
            }
            Err(e) => {
                log::error!("start_stream failed: {e}");
                Err(e)
            }
        }
    }

    fn push_audio(&mut self, samples_16k_mono: &[f32]) -> Result<Option<String>, String> {
        if self.use_fallback {
            let res = self.fallback_mock.push_audio(samples_16k_mono)?;
            if let Some(ref text) = res {
                self.stabilizer.update(text);
            }
            return Ok(res);
        }

        use base64::Engine;
        let mut latest_text = None;

        // Keep each JSON/pipe write bounded. This also protects callers that
        // submit a large buffer (for example external audio), while normal
        // microphone capture still benefits from its ~0.5 s batching.
        for chunk in audio_chunks(samples_16k_mono) {
            let pcm_bytes = AudioResampler::f32_to_pcm16_bytes(chunk);
            let b64 = base64::engine::general_purpose::STANDARD.encode(&pcm_bytes);
            let cmd = json!({
                "cmd": "push_audio_b64",
                "audio_b64": b64
            });

            match self.send_command(cmd) {
                Ok(resp) => {
                    if let Some(text) = resp.get("text").and_then(|v| v.as_str()) {
                        if !text.is_empty() {
                            self.stabilizer.update(text);
                            latest_text = Some(text.to_string());
                        }
                    }
                }
                Err(e) => {
                    log::error!("push_audio failed: {e}");
                    return Err(e);
                }
            }
        }

        Ok(latest_text)
    }

    fn get_partial_transcript(&mut self) -> Result<String, String> {
        if self.use_fallback {
            return self.fallback_mock.get_partial_transcript();
        }
        Ok(self.stabilizer.full_transcript())
    }

    fn stop_stream(&mut self) -> Result<String, String> {
        if self.use_fallback {
            let res = self.fallback_mock.stop_stream()?;
            return Ok(self.stabilizer.finalize(&res));
        }

        let cmd = json!({"cmd": "stop_stream"});
        match self.send_command(cmd) {
            Ok(resp) => {
                if resp.get("status") == Some(&Value::String("error".into())) {
                    let err = resp
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Transcription failed");
                    log::error!("ASR stop_stream error: {err}");
                    return Err(err.to_string());
                }
                if let Some(lang) = resp.get("language").and_then(|v| v.as_str()) {
                    if !lang.is_empty() {
                        self.detected_language = lang.to_string();
                    }
                }
                let text = resp
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                Ok(self.stabilizer.finalize(text))
            }
            Err(e) => {
                log::error!("stop_stream failed: {e}");
                Err(e)
            }
        }
    }

    fn cancel_stream(&mut self) -> Result<(), String> {
        self.stabilizer.reset();
        if self.use_fallback {
            return self.fallback_mock.cancel_stream();
        }
        let _ = self.send_command(json!({"cmd": "cancel_stream"}));
        Ok(())
    }

    fn get_detected_language(&self) -> String {
        self.detected_language.clone()
    }

    fn get_backend_name(&self) -> String {
        if self.use_fallback {
            return self.fallback_mock.get_backend_name();
        }
        self.backend_name.clone()
    }

    fn engine_status(&mut self) -> EngineStatus {
        if self.use_fallback {
            return self.fallback_mock.engine_status();
        }
        let _ = self.refresh_status();
        self.status_cache.clone()
    }
}

impl Qwen3AsrSidecar {
    /// Pull the live status line from the sidecar without blocking long:
    /// the status command is answered instantly by the Python process.
    fn refresh_status(&mut self) -> Result<(), String> {
        let resp = self.send_command(json!({"cmd": "status"}))?;
        if resp.get("status") == Some(&Value::String("ok".into())) {
            let loaded = resp
                .get("loaded")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let backend = resp
                .get("backend")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            if loaded {
                self.backend_name = backend.clone();
            }
            let is_loading = resp
                .get("is_loading")
                .and_then(|v| v.as_bool())
                .unwrap_or_else(|| backend.to_ascii_lowercase().contains("loading"));
            self.status_cache = EngineStatus {
                loaded: loaded && !is_loading,
                device: resp
                    .get("device")
                    .and_then(|v| v.as_str())
                    .unwrap_or("none")
                    .to_string(),
                backend,
                vram_mb: resp
                    .get("vram_mb")
                    .and_then(|v| v.as_f64())
                    .map(|v| v as f32)
                    .unwrap_or(0.0),
                is_downloading: resp
                    .get("is_downloading")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                is_loading,
                download_progress_pct: resp
                    .get("download_progress_pct")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u8,
                error: resp
                    .get("error")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(String::from),
                cuda_available: resp
                    .get("cuda_available")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                torch_cuda_version: resp
                    .get("torch_cuda_version")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                gpu_hint: resp
                    .get("gpu_hint")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(String::from),
            };
        }
        Ok(())
    }
}

impl Drop for Qwen3AsrSidecar {
    fn drop(&mut self) {
        let _ = self.unload_model();
    }
}

#[cfg(test)]
mod tests {
    use super::{audio_chunks, ASR_PUSH_CHUNK_SAMPLES};

    #[test]
    fn audio_push_chunks_are_bounded_to_one_second() {
        let samples = vec![0.0f32; ASR_PUSH_CHUNK_SAMPLES * 2 + 1];
        let chunks: Vec<&[f32]> = audio_chunks(&samples).collect();

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), ASR_PUSH_CHUNK_SAMPLES);
        assert_eq!(chunks[1].len(), ASR_PUSH_CHUNK_SAMPLES);
        assert_eq!(chunks[2].len(), 1);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.len() <= ASR_PUSH_CHUNK_SAMPLES));
    }

    #[test]
    fn empty_audio_produces_no_push_chunks() {
        let samples: [f32; 0] = [];
        assert_eq!(audio_chunks(&samples).count(), 0);
    }
}
