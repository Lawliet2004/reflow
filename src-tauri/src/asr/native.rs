use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use parking_lot::{Mutex, RwLock};
use serde_json::json;

use super::engine::{ASREngine, EngineStatus};
use crate::profile::manifest::ModelManifest;
use crate::rewrite::runtime_install::verify_sha256;
use crate::rewrite::FlowRuntime;

const SAMPLE_RATE: usize = 16_000;
const MAX_AUDIO_SAMPLES: usize = SAMPLE_RATE * 3600;

#[cfg(test)]
#[path = "native_cancellation_tests.rs"]
mod cancellation_tests;

#[cfg(test)]
#[path = "local_http_privacy_tests.rs"]
mod privacy_tests;

pub struct NativeAsrEngine {
    runtime: Arc<FlowRuntime>,
    status: Arc<RwLock<EngineStatus>>,
    audio: Vec<f32>,
    language: String,
    detected_language: String,
    vocabulary: Vec<String>,
    warning: Option<String>,
    truncated_samples: usize,
    cancelled: Arc<AtomicBool>,
    installation_active: Arc<Mutex<bool>>,
    progress: Option<super::engine::ProgressTracker>,
}

impl Default for NativeAsrEngine {
    fn default() -> Self {
        Self {
            runtime: Arc::new(FlowRuntime::default()),
            status: Arc::new(RwLock::new(EngineStatus {
                backend: "native ASR (not loaded)".into(),
                ..Default::default()
            })),
            audio: Vec::new(),
            language: "auto".into(),
            detected_language: String::new(),
            vocabulary: Vec::new(),
            warning: None,
            truncated_samples: 0,
            cancelled: Arc::new(AtomicBool::new(false)),
            installation_active: Arc::new(Mutex::new(true)),
            progress: None,
        }
    }
}

impl NativeAsrEngine {
    fn refresh_liveness(&self) {
        let mut status = self.status.write();
        if status.loaded && !self.runtime.status_ready() {
            status.loaded = false;
            status.error = Some("Native ASR server exited; reload the model to continue.".into());
        }
    }

    fn configure(&mut self, model_dir: &str) -> Result<&'static ModelManifest, String> {
        let dir = Path::new(model_dir);
        let manifest = crate::profile::manifest::NATIVE_ASR_MODELS
            .iter()
            .find(|m| dir.file_name().and_then(|n| n.to_str()) == Some(m.dir_name))
            .ok_or("Unknown native ASR model directory")?;
        *self.installation_active.lock() = false;
        self.runtime.shutdown();
        self.installation_active = Arc::new(Mutex::new(true));
        self.runtime = Arc::new(FlowRuntime::for_audio_model(
            dir.join(manifest.filename),
            dir.join(manifest.auxiliary_files[0].filename),
        ));
        self.status = Arc::new(RwLock::new(EngineStatus {
            backend: "native ASR (not loaded)".into(),
            ..Default::default()
        }));
        Ok(manifest)
    }
}

impl Drop for NativeAsrEngine {
    fn drop(&mut self) {
        *self.installation_active.lock() = false;
        self.runtime.shutdown();
    }
}

fn load_installed_native(
    runtime: &FlowRuntime,
    status: &RwLock<EngineStatus>,
    manifest: &ModelManifest,
    installation_active: &Mutex<bool>,
) -> Result<(), String> {
    // Hold the ownership gate through startup. Unload/reconfigure can then
    // invalidate a pending download or shut down a load that already won.
    let active = installation_active.lock();
    if !*active {
        return Ok(());
    }
    load_native(runtime, status, manifest, "auto")
}

fn load_native(
    runtime: &FlowRuntime,
    status: &RwLock<EngineStatus>,
    manifest: &ModelManifest,
    backend: &str,
) -> Result<(), String> {
    status.write().is_loading = true;
    let outcome = runtime.ensure(manifest.id, backend, Some(99), 0, 4096);
    let mut state = status.write();
    state.is_loading = false;
    match outcome {
        Ok(()) => {
            state.loaded = true;
            state.device = runtime
                .active_mode()
                .map(|m| {
                    if m.is_gpu() {
                        "vulkan".into()
                    } else {
                        "cpu".into()
                    }
                })
                .unwrap_or_else(|| "cpu".into());
            state.backend = format!("{} · {}", manifest.label, state.device);
            state.precision = "int8".into();
            state.error = runtime.last_error();
            Ok(())
        }
        Err(error) => {
            state.loaded = false;
            state.error = Some(error.clone());
            Err(error)
        }
    }
}

impl ASREngine for NativeAsrEngine {
    fn set_progress_tracker(&mut self, tracker: super::engine::ProgressTracker) {
        self.progress = Some(tracker);
    }
    fn initialize(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn load_model(&mut self, model_dir: &str, backend: &str) -> Result<(), String> {
        let manifest = self.configure(model_dir)?;
        for (filename, hash) in std::iter::once((manifest.filename, manifest.sha256)).chain(
            manifest
                .auxiliary_files
                .iter()
                .map(|f| (f.filename, f.sha256)),
        ) {
            if let Err(error) = verify_sha256(&Path::new(model_dir).join(filename), hash) {
                self.status.write().error =
                    Some(format!("Native ASR weight verification failed: {error}"));
                return Err(error);
            }
        }
        load_native(&self.runtime, &self.status, manifest, backend)
    }

    fn load_model_with_precision(
        &mut self,
        model_dir: &str,
        backend: &str,
        precision: &str,
    ) -> Result<(), String> {
        if !matches!(precision, "auto" | "int8") {
            return Err("Native ASR uses the pinned Q8_0 files; select Auto or INT8".into());
        }
        self.load_model(model_dir, backend)
    }

    fn unload_model(&mut self) -> Result<(), String> {
        *self.installation_active.lock() = false;
        self.runtime.shutdown();
        let mut state = self.status.read().clone();
        state.loaded = false;
        state.is_loading = false;
        state.is_downloading = false;
        self.status = Arc::new(RwLock::new(state));
        Ok(())
    }

    fn is_model_loaded(&self) -> bool {
        self.refresh_liveness();
        self.status.read().loaded
    }

    fn start_stream(&mut self, language: &str, vocabulary: &[String]) -> Result<(), String> {
        super::languages::language_name(language)?;
        if !self.is_model_loaded() {
            return Err("Native ASR model is not loaded".into());
        }
        self.audio.clear();
        self.cancelled.store(false, Ordering::Release);
        self.language = language.into();
        self.detected_language = if language == "auto" {
            "en".into()
        } else {
            language.into()
        };
        self.vocabulary = vocabulary.iter().take(60).cloned().collect();
        self.warning = None;
        self.truncated_samples = 0;
        Ok(())
    }

    fn push_audio(&mut self, samples: &[f32]) -> Result<Option<String>, String> {
        let keep = samples
            .len()
            .min(MAX_AUDIO_SAMPLES.saturating_sub(self.audio.len()));
        self.audio.extend_from_slice(&samples[..keep]);
        self.truncated_samples += samples.len() - keep;
        Ok(None)
    }

    fn get_partial_transcript(&mut self) -> Result<String, String> {
        Ok(String::new())
    }

    fn stop_stream(&mut self) -> Result<String, String> {
        let audio = std::mem::take(&mut self.audio);
        let url = self
            .runtime
            .client
            .read()
            .base_url
            .clone()
            .ok_or("Native ASR server is not ready")?;
        // The actor is synchronous, but each HTTP decode has abortable async
        // ownership. Dropping the selected future closes its connection when
        // cancellation wins, instead of waiting for a 180-second response.
        let executor = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("Native ASR request executor: {error}"))?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(180))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;
        let mut texts = Vec::new();
        let mut warnings = Vec::new();
        if self.truncated_samples > 0 {
            warnings.push("Only the first 3600s of this dictation was transcribed.".to_string());
        }
        let ranges = segment_ranges(&audio, SAMPLE_RATE * 8);
        if let Some(progress) = &self.progress {
            progress.write().total = ranges.len();
        }
        for (index, range) in ranges.into_iter().enumerate() {
            if self.cancelled.load(Ordering::Acquire) {
                return Ok(String::new());
            }
            if let Some(progress) = &self.progress {
                progress.write().completed = index;
            }
            let samples = &audio[range];
            let rms = (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt();
            if rms < 0.004 {
                if let Some(progress) = &self.progress {
                    progress.write().completed = index + 1;
                }
                continue;
            }
            let data = base64::engine::general_purpose::STANDARD.encode(wav_bytes(samples)?);
            let mut body = json!({
                "messages": [
                    {"role": "system", "content": if self.vocabulary.is_empty() { String::new() } else { format!("Vocabulary: {}.", self.vocabulary.join(", ")) }},
                    {"role": "user", "content": [{"type": "input_audio", "input_audio": {"data": data, "format": "wav"}}]}
                ],
                "temperature": 0.0,
                "repeat_penalty": 1.0,
                "max_tokens": 384,
                "stream": false
            });
            let language_name = super::languages::language_name(&self.language)?;
            if let Some(name) = language_name {
                body["generation_prompt"] = json!(format!("language {name}<asr_text>"));
            }
            let response = executor.block_on(async {
                let request = async {
                    http.post(format!("{url}/v1/chat/completions"))
                        .json(&body)
                        .send()
                        .await?
                        .error_for_status()?
                        .json::<serde_json::Value>()
                        .await
                };
                let cancelled = async {
                    while !self.cancelled.load(Ordering::Acquire) {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                };
                tokio::select! {
                    biased;
                    _ = cancelled => Ok(None),
                    response = request => response.map(Some),
                }
            });
            // A cancellation racing with a network error still means the
            // dictation was deliberately discarded, not that inference failed.
            if self.cancelled.load(Ordering::Acquire) {
                return Ok(String::new());
            }
            let Some(response) =
                response.map_err(|e| format!("Native ASR segment {}: {e}", index + 1))?
            else {
                return Ok(String::new());
            };
            let choice = response
                .get("choices")
                .and_then(|c| c.get(0))
                .ok_or("Native ASR response has no choice")?;
            if choice.get("finish_reason").and_then(|v| v.as_str()) == Some("length") {
                warnings.push(format!(
                    "Segment {} hit its token cap; text may be cut off.",
                    index + 1
                ));
            }
            let raw = choice
                .pointer("/message/content")
                .and_then(|v| v.as_str())
                .ok_or("Native ASR response has no transcript")?;
            let (text, detected) = parse_transcript(raw);
            if let Some(detected) = detected {
                self.detected_language = detected;
            }
            if !looks_like_vocab_echo(&text, &self.vocabulary)
                && !text.to_lowercase().starts_with("vocabulary:")
                && !text.is_empty()
            {
                texts.push(text);
            }
            if let Some(progress) = &self.progress {
                progress.write().completed = index + 1;
            }
        }
        self.warning = (!warnings.is_empty()).then(|| warnings.join(" "));
        Ok(texts.join(" "))
    }

    fn cancellation_signal(&self) -> Option<super::engine::InferenceCancellation> {
        let cancelled = Arc::clone(&self.cancelled);
        Some(Arc::new(move || cancelled.store(true, Ordering::Release)))
    }

    fn cancel_stream(&mut self) -> Result<(), String> {
        self.cancelled.store(true, Ordering::Release);
        self.audio.clear();
        Ok(())
    }
    fn get_detected_language(&self) -> String {
        self.detected_language.clone()
    }
    fn get_backend_name(&self) -> String {
        self.status.read().backend.clone()
    }
    fn take_last_warning(&mut self) -> Option<String> {
        self.warning.take()
    }
    fn engine_status(&mut self) -> EngineStatus {
        self.refresh_liveness();
        self.status.read().clone()
    }

    fn install_model_dir(&mut self, model_dir: &str, repo: &str) -> Result<(), String> {
        crate::network_policy::check_download()?;
        if self.status.read().is_downloading {
            return Err("A native ASR download is already running".into());
        }
        let manifest = self.configure(model_dir)?;
        if manifest.repo != repo {
            return Err("Native ASR repo does not match manifest".into());
        }
        let dir = PathBuf::from(model_dir);
        let runtime = Arc::clone(&self.runtime);
        let status = Arc::clone(&self.status);
        let installation_active = Arc::clone(&self.installation_active);
        status.write().is_downloading = true;
        std::thread::spawn(move || {
            let outcome =
                download_native(&dir, manifest, &status, &installation_active).and_then(|_| {
                    load_installed_native(&runtime, &status, manifest, &installation_active)
                });
            let mut state = status.write();
            state.is_downloading = false;
            if let Err(error) = outcome {
                state.error = Some(error);
            }
        });
        Ok(())
    }
}

fn download_native(
    dir: &Path,
    manifest: &ModelManifest,
    status: &RwLock<EngineStatus>,
    installation_active: &Mutex<bool>,
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60 * 60))
        .redirect(crate::network_policy::redirect_policy())
        .build()
        .map_err(|e| e.to_string())?;
    let mut downloaded = 0u64;
    for (filename, hash) in std::iter::once((manifest.filename, manifest.sha256)).chain(
        manifest
            .auxiliary_files
            .iter()
            .map(|f| (f.filename, f.sha256)),
    ) {
        if !*installation_active.lock() {
            return Err("Native ASR installation canceled".into());
        }
        let dest = dir.join(filename);
        if dest.is_file() && verify_sha256(&dest, hash).is_ok() {
            downloaded += std::fs::metadata(&dest).map_err(|e| e.to_string())?.len();
            continue;
        }
        let part = dest.with_extension("gguf.part");
        let mut response = crate::network_policy::send_download(client.get(format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            manifest.repo, manifest.revision, filename
        )))?;
        if !response.status().is_success() {
            return Err(format!("Model download failed: HTTP {}", response.status()));
        }
        let mut file = {
            let active = installation_active.lock();
            if !*active {
                return Err("Native ASR installation canceled".into());
            }
            std::fs::File::create(&part).map_err(|e| e.to_string())?
        };
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let count = response.read(&mut buffer).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            let active = installation_active.lock();
            if !*active {
                return Err("Native ASR installation canceled".into());
            }
            file.write_all(&buffer[..count])
                .map_err(|e| e.to_string())?;
            downloaded += count as u64;
            status.write().download_progress_pct =
                (downloaded * 100 / manifest.download_bytes).min(99) as u8;
        }
        file.flush().map_err(|e| e.to_string())?;
        drop(file);
        // Serialize the final artifact commit with invalidation so a retired
        // installer cannot replace files belonging to a newer installation.
        let active = installation_active.lock();
        if !*active {
            return Err("Native ASR installation canceled".into());
        }
        if let Err(error) = verify_sha256(&part, hash) {
            let _ = std::fs::remove_file(&part);
            return Err(format!(
                "Native ASR SHA-256 mismatch for {filename}: {error}"
            ));
        }
        if dest.exists() {
            std::fs::remove_file(&dest).map_err(|e| e.to_string())?;
        }
        std::fs::rename(part, dest).map_err(|e| e.to_string())?;
    }
    status.write().download_progress_pct = 100;
    Ok(())
}

fn segment_ranges(audio: &[f32], max_samples: usize) -> Vec<std::ops::Range<usize>> {
    assert!(max_samples > 0);
    let mut ranges = Vec::new();
    let mut start = 0;
    while audio.len() - start > max_samples {
        let target = start + max_samples;
        let window_start = (target.saturating_sub(SAMPLE_RATE * 2)).max(start + 1);
        let cut = audio[window_start..target]
            .windows(512)
            .step_by(256)
            .enumerate()
            .map(|(i, frame)| {
                (
                    window_start + i * 256 + 256,
                    frame.iter().map(|x| x * x).sum::<f32>(),
                )
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(cut, _)| cut)
            .unwrap_or(target);
        ranges.push(start..cut);
        start = cut;
    }
    if start < audio.len() {
        ranges.push(start..audio.len());
    }
    ranges
}

fn wav_bytes(samples: &[f32]) -> Result<Vec<u8>, String> {
    let mut data = Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(
        &mut data,
        hound::WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE as u32,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .map_err(|e| e.to_string())?;
    for sample in samples {
        writer
            .write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())?;
    Ok(data.into_inner())
}

fn looks_like_vocab_echo(text: &str, vocabulary: &[String]) -> bool {
    let terms: std::collections::HashSet<String> = vocabulary
        .iter()
        .flat_map(|term| term.split_whitespace().map(str::to_lowercase))
        .collect();
    let clean: String = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || c.is_whitespace())
        .collect();
    let words: Vec<&str> = clean.split_whitespace().collect();
    !words.is_empty() && words.iter().all(|word| terms.contains(*word))
}

fn parse_transcript(raw: &str) -> (String, Option<String>) {
    match raw.split_once("<asr_text>") {
        Some((prefix, text)) => (
            text.trim().to_string(),
            prefix.trim().strip_prefix("language ").map(str::to_string),
        ),
        None => (raw.trim().to_string(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuring_a_new_model_isolates_retired_install_status() {
        let mut engine = NativeAsrEngine::default();
        let retired = Arc::clone(&engine.status);
        engine
            .configure(crate::profile::manifest::NATIVE_ASR_MODELS[0].dir_name)
            .unwrap();
        retired.write().error = Some("retired download failed".into());
        retired.write().is_downloading = true;
        let current = engine.engine_status();
        assert!(current.error.is_none());
        assert!(!current.is_downloading);
    }

    #[test]
    fn unloading_isolates_retired_install_status() {
        let mut engine = NativeAsrEngine::default();
        let retired = Arc::clone(&engine.status);
        retired.write().is_downloading = true;
        engine.unload_model().unwrap();
        retired.write().error = Some("retired download failed".into());
        let current = engine.engine_status();
        assert!(current.error.is_none());
        assert!(!current.is_downloading);
    }

    #[test]
    fn a_retired_install_cannot_reload_an_unloaded_or_reconfigured_engine() {
        for reconfigure in [false, true] {
            let mut engine = NativeAsrEngine::default();
            let retired_runtime = Arc::clone(&engine.runtime);
            let retired_status = Arc::clone(&engine.status);
            let retired_gate = Arc::clone(&engine.installation_active);
            let manifest = &crate::profile::manifest::NATIVE_ASR_MODELS[0];
            if reconfigure {
                engine.configure(manifest.dir_name).unwrap();
            } else {
                engine.unload_model().unwrap();
            }
            // With no model files, attempting runtime startup would fail. A
            // retired completion must skip startup and leave both states alone.
            load_installed_native(&retired_runtime, &retired_status, manifest, &retired_gate)
                .unwrap();
            assert!(!retired_runtime.status_ready());
            assert!(retired_status.read().error.is_none());
            assert!(!engine.engine_status().loaded);
        }
    }

    fn stub_child(live: bool) -> std::process::Child {
        #[cfg(windows)]
        let mut command = {
            use std::os::windows::process::CommandExt;
            let mut command = std::process::Command::new("powershell");
            command.args([
                "-NoProfile",
                "-Command",
                if live {
                    "Start-Sleep -Seconds 60"
                } else {
                    "exit 0"
                },
            ]);
            command.creation_flags(0x08000000);
            command
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut command = std::process::Command::new("sh");
            command.args(["-c", if live { "sleep 60" } else { "exit 0" }]);
            command
        };
        command
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap()
    }

    #[test]
    fn native_status_reports_an_exited_server_and_blocks_new_streams() {
        let mut engine = NativeAsrEngine::default();
        let mut child = stub_child(false);
        child.wait().unwrap();
        engine.runtime.attach_test_child(child);
        *engine.runtime.client.write() = crate::rewrite::FlowClient::new_url(
            "http://127.0.0.1:9".into(),
            Duration::from_secs(1),
        );
        engine.status.write().loaded = true;
        let status = engine.engine_status();
        assert!(!status.loaded);
        assert!(status.error.unwrap().contains("exited"));
        assert!(!engine.is_model_loaded());
        assert!(engine.start_stream("en", &[]).is_err());
    }
    #[test]
    fn chat_audio_protocol_and_cap_warning() {
        use std::io::BufRead;
        for cancel in [false, true] {
            let engine = NativeAsrEngine::default();
            let signal = Arc::clone(&engine.cancelled);
            let (request_tx, request_rx) = std::sync::mpsc::channel();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = std::io::BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with("POST /v1/chat/completions "));
                let mut length = 0;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length: ")
                    {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(body["generation_prompt"], "language English<asr_text>");
                assert_eq!(body["messages"][0]["content"], "Vocabulary: Reflow.");
                assert_eq!(body["messages"][1]["content"][0]["type"], "input_audio");
                let pcm = base64::engine::general_purpose::STANDARD
                    .decode(
                        body["messages"][1]["content"][0]["input_audio"]["data"]
                            .as_str()
                            .unwrap(),
                    )
                    .unwrap();
                assert_eq!(
                    hound::WavReader::new(Cursor::new(pcm)).unwrap().duration(),
                    SAMPLE_RATE as u32
                );
                request_tx.send(()).unwrap();
                if cancel {
                    let deadline = std::time::Instant::now() + Duration::from_secs(5);
                    while !signal.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    assert!(
                        signal.load(Ordering::Acquire),
                        "Cancellation must reach a blocked decode"
                    );
                }
                let result = r#"{"choices":[{"message":{"content":"language English<asr_text>I use Reflow for notes."},"finish_reason":"length"}]}"#;
                write!(reader.get_mut(), "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{result}", result.len()).unwrap();
            });
            *engine.runtime.client.write() = crate::rewrite::FlowClient::new_url(
                format!("http://{address}"),
                Duration::from_secs(2),
            );
            engine.status.write().loaded = true;
            engine.runtime.attach_test_child(stub_child(true));
            let handle = crate::asr::AsrHandle::spawn(Box::new(engine), 8);
            handle
                .start_stream_blocking(7, "en", &["Reflow".into()])
                .unwrap();
            handle
                .push_audio_blocking(7, 0, &vec![0.1; SAMPLE_RATE])
                .unwrap();
            let stopping = handle.clone();
            let decode = std::thread::spawn(move || stopping.stop_stream_blocking(7).unwrap());
            request_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            if cancel {
                handle.cancel_stream_blocking(7).unwrap();
                assert_eq!(decode.join().unwrap(), "");
                assert!(handle.take_last_warning().is_none());
            } else {
                assert_eq!(decode.join().unwrap(), "I use Reflow for notes.");
                assert!(handle.take_last_warning().unwrap().contains("token cap"));
            }
            server.join().unwrap();
        }
    }

    #[test]
    fn silence_cuts_cover_audio_and_stay_bounded() {
        let mut audio = vec![0.2; SAMPLE_RATE * 21];
        audio[SAMPLE_RATE * 7..SAMPLE_RATE * 8].fill(0.0);
        audio[SAMPLE_RATE * 14..SAMPLE_RATE * 15].fill(0.0);
        let ranges = segment_ranges(&audio, SAMPLE_RATE * 8);
        assert_eq!(ranges.first().unwrap().start, 0);
        assert_eq!(ranges.last().unwrap().end, audio.len());
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
            assert_eq!(audio[pair[0].end], 0.0);
        }
        assert!(ranges.iter().all(|r| r.len() <= SAMPLE_RATE * 8));
    }
    #[test]
    fn vocabulary_echo_preserves_real_speech() {
        let vocab = vec!["Reflow".into(), "Qwen ASR".into()];
        assert!(looks_like_vocab_echo("Reflow, Qwen ASR!", &vocab));
        assert!(!looks_like_vocab_echo("I use Reflow for notes", &vocab));
    }
    #[test]
    fn encoded_audio_is_pcm16_mono_wav() {
        let bytes = wav_bytes(&[0.0, 0.5, -0.5]).unwrap();
        let reader = hound::WavReader::new(Cursor::new(bytes)).unwrap();
        assert_eq!(reader.spec().sample_rate, 16000);
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.duration(), 3);
    }
    #[test]
    fn transcript_metadata_is_removed() {
        assert_eq!(
            parse_transcript("language Hindi<asr_text>namaste"),
            ("namaste".into(), Some("Hindi".into()))
        );
        assert_eq!(parse_transcript("plain text"), ("plain text".into(), None));
    }
}
