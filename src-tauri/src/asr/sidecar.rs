use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::engine::{ASREngine, EngineStatus};
use super::mock::MockASREngine;
use crate::audio::resampler::AudioResampler;
use crate::profile::manifest::ASR_MODELS;

const ASR_PUSH_CHUNK_SAMPLES: usize = 16_000;
const CANCELLED_TRANSPORT: &str = "engine_cancelled: discarded dictation";
const CANCEL_GRACE: Duration = Duration::from_millis(400);

#[cfg(test)]
#[path = "sidecar_transport_tests.rs"]
mod transport_tests;

/// Marker prefix for a sidecar read timeout.
///
/// Timeouts have to be distinguishable from ordinary engine errors so the
/// supervisor can restart the process instead of surfacing an opaque failure,
/// and so callers never confuse "the model rejected this" with "the pipe is
/// wedged". Matching a prefix keeps the `Result<_, String>` trait boundary
/// intact while still being machine-checkable.
pub const ENGINE_TIMEOUT_PREFIX: &str = "engine_timeout:";

/// `true` when `err` came from a sidecar read timeout.
pub fn is_engine_timeout(err: &str) -> bool {
    err.starts_with(ENGINE_TIMEOUT_PREFIX)
}

fn engine_timeout_error(command: &str, waited: Duration) -> String {
    format!(
        "{ENGINE_TIMEOUT_PREFIX} '{command}' did not answer within {:.1}s",
        waited.as_secs_f32()
    )
}

/// Liveness check. Answered on the sidecar's reader thread before any heavy
/// import happens, so this genuinely is a millisecond-scale round trip.
const TIMEOUT_FAST: Duration = Duration::from_secs(5);
/// Status polling. Answered on the sidecar's main loop from cached state, so
/// normally a millisecond-scale round trip.
///
/// Deliberately short rather than generous. The sidecar cannot answer *any*
/// probe while its main thread is inside `_warm_imports` (9-52s, measured), so
/// a long budget does not make the answer arrive sooner — it just parks the
/// single-threaded ASR actor for the whole budget, and every queued command
/// (including the hotkey's `start_stream`) waits behind it. Abandoning a probe
/// early is safe because replies are matched by request id, so a late answer is
/// recognised and discarded rather than misread as the reply to a later
/// request. A timeout here means "no news yet", not "broken".
const TIMEOUT_STATUS: Duration = Duration::from_secs(8);
/// Stream control and model-load acknowledgements. These return an ack, not a
/// completed operation, so they are still fast — but `load_model` is acked
/// only after the sidecar's main thread has finished the one-time warm import
/// of torch/transformers/scipy (15-30s on a cold filesystem; see
/// `_warm_imports` in the runtime), so this budget has to cover that warmup.
const TIMEOUT_CONTROL: Duration = Duration::from_secs(90);
/// Incremental audio submission.
const TIMEOUT_PUSH_AUDIO: Duration = Duration::from_secs(30);
/// Whole-utterance transcription floor. The real budget scales with how much
/// audio was captured — see `transcribe_timeout`.
const TIMEOUT_TRANSCRIBE: Duration = Duration::from_secs(180);

/// Worst-case seconds of compute per second of audio.
///
/// Measured on the development machine: ~0.40 with the model on the GPU, ~2.4
/// with it on the CPU. The CPU figure is the one that has to be budgeted for,
/// because falling back to the CPU is exactly when a transcription is slowest and
/// least deserving of being killed mid-flight. Rounded up for headroom.
const WORST_CASE_RTF: f32 = 3.0;

/// Upper bound regardless of audio length, so a genuinely wedged sidecar is
/// still noticed eventually.
const TIMEOUT_TRANSCRIBE_MAX: Duration = Duration::from_secs(3 * 60 * 60);

/// How long to allow for transcribing `samples` of 16 kHz audio.
///
/// A fixed budget is wrong in both directions. 180s is far more than a
/// five-second utterance needs, and far less than ten minutes of hands-free
/// dictation needs — and because `stop_stream` is not a read-only probe, blowing
/// its budget tears the sidecar down and counts a crash strike. So raising the
/// audio limit without raising this turns a truncated transcript into a dead
/// sidecar and no transcript at all.
fn transcribe_timeout(samples: usize) -> Duration {
    let audio_secs = samples as f32 / 16_000.0;
    let scaled = Duration::from_secs_f32(audio_secs * WORST_CASE_RTF);
    scaled.clamp(TIMEOUT_TRANSCRIBE, TIMEOUT_TRANSCRIBE_MAX)
}

fn audio_chunks(samples: &[f32]) -> impl Iterator<Item = &[f32]> {
    samples.chunks(ASR_PUSH_CHUNK_SAMPLES)
}

struct PipeWrite {
    payload: PipePayload,
    reply: std::sync::mpsc::Sender<Result<(), String>>,
}

enum PipePayload {
    Bytes(Vec<u8>),
    Command(Value),
}

/// One owner serializes complete protocol frames. Producers never acquire the
/// pipe mutex, and at most two frames can wait behind the current write.
struct SidecarWriter {
    queue: std::sync::mpsc::SyncSender<PipeWrite>,
    closed: Arc<AtomicBool>,
    cancelled: AtomicBool,
}

impl SidecarWriter {
    fn spawn(stdin: Arc<parking_lot::Mutex<ChildStdin>>) -> Result<Arc<Self>, String> {
        let (queue, receiver) = std::sync::mpsc::sync_channel::<PipeWrite>(2);
        let closed = Arc::new(AtomicBool::new(false));
        let worker_closed = Arc::clone(&closed);
        std::thread::Builder::new()
            .name("asr-sidecar-writer".into())
            .spawn(move || {
                while !worker_closed.load(Ordering::Acquire) {
                    let write = match receiver.recv_timeout(Duration::from_millis(100)) {
                        Ok(write) => write,
                        Err(RecvTimeoutError::Timeout) => continue,
                        Err(RecvTimeoutError::Disconnected) => break,
                    };
                    if worker_closed.load(Ordering::Acquire) {
                        break;
                    }
                    let bytes = match write.payload {
                        PipePayload::Bytes(bytes) => bytes,
                        PipePayload::Command(command) => match serde_json::to_vec(&command) {
                            Ok(mut bytes) => {
                                bytes.push(b'\n');
                                bytes
                            }
                            Err(error) => {
                                let _ = write.reply.send(Err(error.to_string()));
                                continue;
                            }
                        },
                    };
                    if worker_closed.load(Ordering::Acquire) {
                        break;
                    }
                    let result = {
                        let mut stdin = stdin.lock();
                        stdin.write_all(&bytes).and_then(|_| stdin.flush())
                    }
                    .map_err(|error| format!("Failed to write to sidecar stdin: {error}"));
                    let failed = result.is_err();
                    let _ = write.reply.send(result);
                    if failed {
                        worker_closed.store(true, Ordering::Release);
                        break;
                    }
                }
            })
            .map_err(|error| format!("Could not start ASR pipe writer: {error}"))?;
        Ok(Arc::new(Self {
            queue,
            closed,
            cancelled: AtomicBool::new(false),
        }))
    }

    fn enqueue(&self, bytes: Vec<u8>) -> Result<Receiver<Result<(), String>>, String> {
        self.enqueue_payload(PipePayload::Bytes(bytes))
    }

    fn enqueue_payload(
        &self,
        payload: PipePayload,
    ) -> Result<Receiver<Result<(), String>>, String> {
        if self.closed.load(Ordering::Acquire) {
            return Err("ASR sidecar writer is closed".into());
        }
        let (reply, receiver) = std::sync::mpsc::channel();
        self.queue
            .try_send(PipeWrite { payload, reply })
            .map_err(|error| -> String {
                match error {
                    std::sync::mpsc::TrySendError::Full(_) => {
                        "ASR sidecar write queue is full".into()
                    }
                    std::sync::mpsc::TrySendError::Disconnected(_) => {
                        "ASR sidecar writer is disconnected".into()
                    }
                }
            })?;
        Ok(receiver)
    }
}

pub struct Qwen3AsrSidecar {
    child: Option<Child>,
    stdin: Option<Arc<parking_lot::Mutex<ChildStdin>>>,
    writer: Option<Arc<SidecarWriter>>,
    /// Lines produced by a dedicated reader thread. Reading through a channel
    /// rather than blocking on the pipe is what makes a bounded wait possible:
    /// `BufReader::read_line` has no timeout, so a wedged sidecar used to
    /// deadlock the caller forever while holding the engine lock.
    ///
    /// The `Mutex` is only there to satisfy `Sync`: `Receiver` is `Send` but
    /// not `Sync`, and the engine lives behind a shared `RwLock`.
    responses: Option<parking_lot::Mutex<Receiver<String>>>,
    backend_name: String,
    detected_language: String,
    /// Non-fatal notice from the last transcription, e.g. that the dictation
    /// exceeded the single-pass audio limit and was cut.
    last_warning: Option<String>,
    fallback_mock: MockASREngine,
    use_fallback: bool,
    resource_dir: Option<PathBuf>,
    status_cache: EngineStatus,
    crash_timestamps: Vec<std::time::Instant>,
    circuit_breaker_tripped_until: Option<std::time::Instant>,
    /// Monotonic request counter. Echoed by the sidecar so replies can be
    /// matched to requests instead of relying on arrival order.
    next_request_id: u64,
    /// Consecutive read-only probe timeouts. A single slow poll during a heavy
    /// load is normal and must not be reported as a fault, so the error is only
    /// surfaced once the sidecar has missed `PROBE_TIMEOUT_TOLERANCE` in a row.
    consecutive_probe_timeouts: u32,
    /// `true` between a `load_model` ack and the load resolving (ready or
    /// failed).
    ///
    /// While this is set, the sidecar is expected to go mute for tens of
    /// seconds: its main thread is blocked warming native imports and cannot
    /// answer probes at all. Silence in that window is the normal shape of a
    /// load in progress, so it must never be recorded as an engine error.
    /// Doing so is what made a healthy load surface as
    /// "Model failed to load: engine_timeout" and, because the recording path
    /// gates on that, silently disabled the hotkey.
    load_in_flight: bool,
    /// Samples handed to the engine since `start_stream`.
    ///
    /// Kept so the transcription timeout can be derived from how much audio
    /// actually has to be processed. A fixed budget is wrong in both directions:
    /// 180s is far more than a 5-second utterance needs, and far less than ten
    /// minutes of hands-free dictation needs.
    pushed_samples: usize,
    progress: Option<super::engine::ProgressTracker>,
}

/// How many consecutive `status`/`ping` timeouts to absorb before telling the
/// user something is wrong.
const PROBE_TIMEOUT_TOLERANCE: u32 = 3;

pub const CRASH_LOOP_WINDOW_SECS: u64 = 30;
pub const CRASH_LOOP_MAX_CRASHES: usize = 3;
pub const CRASH_LOOP_COOLDOWN_SECS: u64 = 60;

impl Default for Qwen3AsrSidecar {
    fn default() -> Self {
        Self::new()
    }
}

impl Qwen3AsrSidecar {
    pub fn new() -> Self {
        Self {
            child: None,
            stdin: None,
            writer: None,
            responses: None,
            backend_name: "Qwen3-ASR (Local CUDA/CPU)".into(),
            detected_language: "en".into(),
            last_warning: None,
            fallback_mock: MockASREngine::new(),
            use_fallback: false,
            resource_dir: None,
            status_cache: EngineStatus::default(),
            crash_timestamps: Vec::new(),
            circuit_breaker_tripped_until: None,
            next_request_id: 1,
            consecutive_probe_timeouts: 0,
            load_in_flight: false,
            pushed_samples: 0,
            progress: None,
        }
    }

    pub(crate) fn find_python() -> Option<String> {
        let names: &[&str] = if cfg!(windows) {
            &["python", "python3"]
        } else {
            &["python3", "python"]
        };

        for name in names {
            if let Some(out) = Self::bounded_python_probe(name, "import sys; print(sys.executable)")
            {
                let exe = String::from_utf8_lossy(&out).to_lowercase();
                if !exe.contains("windowsapps") {
                    return Some((*name).to_string());
                }
            }
        }
        None
    }

    pub(crate) fn bounded_python_probe(python: &str, code: &str) -> Option<Vec<u8>> {
        let mut command = Command::new(python);
        command
            .args(["-c", code])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().ok()?;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let output = child.wait_with_output().ok()?;
                    return status.success().then_some(output.stdout);
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
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
                candidates.push(
                    parent
                        .join("../../model-runtime")
                        .join("qwen3_asr_runtime.py"),
                );
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

    /// Timeout to use for a given command name.
    fn timeout_for(command: &str) -> Duration {
        match command {
            "ping" => TIMEOUT_FAST,
            "status" => TIMEOUT_STATUS,
            "push_audio_b64" => TIMEOUT_PUSH_AUDIO,
            "stop_stream" => TIMEOUT_TRANSCRIBE,
            _ => TIMEOUT_CONTROL,
        }
    }

    /// `true` for read-only probes whose failure says nothing about the health
    /// of an in-flight model load, and which therefore must not tear the child
    /// down. See `send_command_timeout`.
    fn is_probe(command: &str) -> bool {
        matches!(command, "ping" | "status")
    }

    fn send_command(&mut self, payload: Value) -> Result<Value, String> {
        let command = payload
            .get("cmd")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let timeout = Self::timeout_for(&command);
        self.send_command_timeout(payload, timeout)
    }

    /// Write a command and wait at most `timeout` for its reply.
    ///
    /// Replies are matched to requests by an echoed `id` rather than by arrival
    /// order. The sidecar answers `status`, `ping` and `cancel_stream` on its
    /// reader thread so they stay responsive while a long transcription is in
    /// flight, which means replies genuinely can arrive out of order. Matching
    /// on `id` also makes a timeout survivable: a late reply is recognised as
    /// belonging to an abandoned request and discarded, instead of being
    /// misread as the answer to the next one.
    ///
    /// Because of that, a timeout on a read-only probe (`status`/`ping`) no
    /// longer tears down the child. It used to, and that was the bug: the very
    /// first `status` after spawn had to wait on a cold `import torch` inside
    /// the sidecar, blew its budget, and killed the process before the model
    /// could finish loading — on every single launch.
    fn send_command_timeout(&mut self, payload: Value, timeout: Duration) -> Result<Value, String> {
        let deadline = std::time::Instant::now() + timeout;
        let command = payload
            .get("cmd")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);

        let mut payload = payload;
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("id".into(), Value::from(request_id));
        }
        // Serialization belongs to the supervised job: even a large external
        // control payload cannot postpone the caller's deadline.
        self.write_until(PipePayload::Command(payload), &command, deadline, timeout)?;
        let mut cancellation_deadline = None;

        // The sidecar may interleave non-JSON chatter; skip blank lines but
        // keep the overall wait bounded by `timeout`.
        loop {
            let now = std::time::Instant::now();
            let mut remaining = deadline.saturating_duration_since(now);
            if command == "stop_stream" && self.transport_cancelled() {
                let cancel_deadline = cancellation_deadline.get_or_insert(now + CANCEL_GRACE);
                remaining = remaining.min(cancel_deadline.saturating_duration_since(now));
                if remaining.is_zero() {
                    return Err(self.abandon_cancelled_transport());
                }
            }
            if remaining.is_zero() {
                return Err(self.on_command_timeout(&command, timeout));
            }
            let received = {
                let guard = self
                    .responses
                    .as_ref()
                    .ok_or("Subprocess stdout reader is not open")?
                    .lock();
                guard.recv_timeout(if command == "stop_stream" {
                    remaining.min(Duration::from_millis(20))
                } else {
                    remaining
                })
            };
            match received {
                Ok(line) if line.trim().is_empty() => continue,
                Ok(line) => {
                    let parsed: Value = match serde_json::from_str(&line) {
                        Ok(v) => v,
                        Err(e) => return Err(format!("Invalid JSON response: {}: {}", e, line)),
                    };
                    if parsed.get("event").and_then(Value::as_str) == Some("asr_progress") {
                        if command == "stop_stream"
                            && parsed.get("id").and_then(Value::as_u64) == Some(request_id)
                            && !self.transport_cancelled()
                        {
                            if let Some(progress) = &self.progress {
                                let mut progress = progress.write();
                                progress.total = parsed
                                    .get("total")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0)
                                    .min(1000)
                                    as usize;
                                progress.completed = parsed
                                    .get("completed")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0)
                                    .min(progress.total as u64)
                                    as usize;
                            }
                        }
                        continue;
                    }
                    // A reply carrying a different id belongs to a request we
                    // already gave up on. Drop it and keep waiting for ours.
                    match parsed.get("id").and_then(|v| v.as_u64()) {
                        Some(id) if id != request_id => {
                            log::debug!(
                                "Discarding stale sidecar reply for request {id} \
                                 while waiting on {request_id} ('{command}')"
                            );
                            continue;
                        }
                        _ => {
                            return if command == "stop_stream" && self.transport_cancelled() {
                                Ok(json!({"id": request_id, "status": "ok", "text": ""}))
                            } else {
                                Ok(parsed)
                            };
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if std::time::Instant::now() < deadline {
                        continue;
                    }
                    return if command == "stop_stream" && self.transport_cancelled() {
                        Err(self.abandon_cancelled_transport())
                    } else {
                        Err(self.on_command_timeout(&command, timeout))
                    };
                }
                Err(RecvTimeoutError::Disconnected) => {
                    if command == "stop_stream" && self.transport_cancelled() {
                        return Err(self.abandon_cancelled_transport());
                    }
                    let err = "ASR sidecar closed its output pipe".to_string();
                    self.kill_child();
                    self.record_crash_and_check_breaker(&err);
                    return Err(err);
                }
            }
        }
    }

    fn transport_cancelled(&self) -> bool {
        self.writer
            .as_ref()
            .is_some_and(|writer| writer.cancelled.load(Ordering::Acquire))
    }

    fn abandon_cancelled_transport(&mut self) -> String {
        self.kill_child();
        self.fallback("ASR did not acknowledge cancellation; reload the model to continue.");
        CANCELLED_TRANSPORT.into()
    }

    fn writer(&mut self) -> Result<Arc<SidecarWriter>, String> {
        if let Some(writer) = &self.writer {
            return Ok(Arc::clone(writer));
        }
        let stdin = Arc::clone(self.stdin.as_ref().ok_or("Subprocess stdin is not open")?);
        let writer = SidecarWriter::spawn(stdin)?;
        self.writer = Some(Arc::clone(&writer));
        Ok(writer)
    }

    fn write_until(
        &mut self,
        payload: PipePayload,
        command: &str,
        deadline: std::time::Instant,
        budget: Duration,
    ) -> Result<(), String> {
        let writer = self.writer()?;
        let receiver = match writer.enqueue_payload(payload) {
            Ok(receiver) => receiver,
            Err(error) => {
                self.kill_child();
                self.record_crash_and_check_breaker(&error);
                return Err(error);
            }
        };
        let mut cancellation_deadline = None;
        loop {
            let now = std::time::Instant::now();
            let mut remaining = deadline.saturating_duration_since(now);
            let watch_cancellation = matches!(command, "stop_stream" | "push_audio");
            if watch_cancellation && writer.cancelled.load(Ordering::Acquire) {
                let cancel_deadline = cancellation_deadline.get_or_insert(now + CANCEL_GRACE);
                remaining = remaining.min(cancel_deadline.saturating_duration_since(now));
                if remaining.is_zero() {
                    return Err(self.abandon_cancelled_transport());
                }
            }
            match receiver.recv_timeout(if watch_cancellation {
                remaining.min(Duration::from_millis(20))
            } else {
                remaining
            }) {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(error)) => {
                    if watch_cancellation && writer.cancelled.load(Ordering::Acquire) {
                        return Err(self.abandon_cancelled_transport());
                    }
                    self.kill_child();
                    self.record_crash_and_check_breaker(&error);
                    return Err(error);
                }
                Err(RecvTimeoutError::Disconnected) => {
                    if watch_cancellation && writer.cancelled.load(Ordering::Acquire) {
                        return Err(self.abandon_cancelled_transport());
                    }
                    self.kill_child();
                    let error = "ASR sidecar writer closed before completing its frame".to_string();
                    self.record_crash_and_check_breaker(&error);
                    return Err(error);
                }
                Err(RecvTimeoutError::Timeout) => {
                    if std::time::Instant::now() < deadline {
                        continue;
                    }
                    if watch_cancellation && writer.cancelled.load(Ordering::Acquire) {
                        return Err(self.abandon_cancelled_transport());
                    }
                    // A partially written frame cannot be retried safely, even for
                    // read-only probes. Terminate first; do not lock the full pipe.
                    let error = engine_timeout_error(command, budget);
                    self.kill_child();
                    self.record_crash_and_check_breaker(&error);
                    return Err(error);
                }
            }
        }
    }

    /// Decide what a read timeout means for the child process.
    ///
    /// Probes are non-destructive: id-matched replies mean a late answer cannot
    /// corrupt a later request, so a slow probe is reported and retried rather
    /// than escalated into a process kill and a crash-loop strike. Real
    /// operations still tear the child down, because a wedged transcription is
    /// evidence the runtime itself is stuck.
    fn on_command_timeout(&mut self, command: &str, timeout: Duration) -> String {
        let err = engine_timeout_error(command, timeout);
        if Self::is_probe(command) {
            self.consecutive_probe_timeouts = self.consecutive_probe_timeouts.saturating_add(1);
            // A load in progress is expected to be mute: the sidecar's main
            // thread is inside a blocking native import and physically cannot
            // answer. Recording that as an engine error makes a healthy load
            // look like a failed one to every consumer of `engine_status`,
            // including the gate that decides whether the hotkey may record.
            let loading = self.load_in_flight || self.status_cache.is_loading;
            if loading {
                log::debug!(
                    "{err}; sidecar is mid-load and cannot answer probes yet \
                     ({} consecutive)",
                    self.consecutive_probe_timeouts
                );
                return err;
            }
            log::warn!(
                "{err}; keeping the sidecar alive (read-only probe, {}/{} consecutive)",
                self.consecutive_probe_timeouts,
                PROBE_TIMEOUT_TOLERANCE
            );
            // Keep the last known state. A load in progress is still in
            // progress; only escalate once the sidecar is persistently mute.
            if self.consecutive_probe_timeouts >= PROBE_TIMEOUT_TOLERANCE {
                self.status_cache.error = Some(err.clone());
            }
            err
        } else {
            log::error!("{err}; tearing down the sidecar to avoid a desynchronised pipe");
            self.kill_child();
            self.record_crash_and_check_breaker(&err);
            err
        }
    }

    pub fn is_circuit_breaker_open(&mut self) -> bool {
        if let Some(deadline) = self.circuit_breaker_tripped_until {
            if std::time::Instant::now() < deadline {
                return true;
            }
            self.circuit_breaker_tripped_until = None;
            self.crash_timestamps.clear();
        }
        false
    }

    pub fn record_crash_and_check_breaker(&mut self, reason: &str) {
        let now = std::time::Instant::now();
        self.crash_timestamps.push(now);
        let cutoff = now
            .checked_sub(std::time::Duration::from_secs(CRASH_LOOP_WINDOW_SECS))
            .unwrap_or(now);
        self.crash_timestamps.retain(|t| *t >= cutoff);

        if self.crash_timestamps.len() >= CRASH_LOOP_MAX_CRASHES {
            let cooldown = now + std::time::Duration::from_secs(CRASH_LOOP_COOLDOWN_SECS);
            self.circuit_breaker_tripped_until = Some(cooldown);
            let msg = format!(
                "ASR sidecar crashed {} times within {}s. Crash loop circuit breaker tripped (cooldown: {}s). Last error: {}",
                CRASH_LOOP_MAX_CRASHES, CRASH_LOOP_WINDOW_SECS, CRASH_LOOP_COOLDOWN_SECS, reason
            );
            log::error!("{msg}");
            self.use_fallback = false;
            self.status_cache.loaded = false;
            self.status_cache.is_loading = false;
            self.status_cache.backend = "circuit_breaker_tripped".into();
            self.status_cache.error = Some(msg);
        } else {
            self.fallback(reason);
        }
    }

    fn fallback(&mut self, reason: &str) {
        // Never silently swap in the mock engine in a real session — that
        // reports loaded=true and inserts canned text unrelated to the mic.
        log::error!("ASR sidecar unavailable ({reason})");
        self.use_fallback = false;
        self.load_in_flight = false;
        self.status_cache.loaded = false;
        self.status_cache.is_loading = false;
        self.status_cache.backend = "unavailable".into();
        self.status_cache.error = Some(reason.to_string());
    }

    fn kill_child(&mut self) {
        if let Some(writer) = self.writer.take() {
            writer.closed.store(true, Ordering::Release);
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.stdin = None;
        self.responses = None;
    }
}

/// Pump the sidecar's stdout into a channel on a dedicated thread.
///
/// The thread ends when the pipe closes, which disconnects the receiver and
/// lets `send_command_timeout` report a dead sidecar instead of blocking.
fn spawn_response_reader(
    stdout: std::process::ChildStdout,
) -> parking_lot::Mutex<Receiver<String>> {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::Builder::new()
        .name("asr-sidecar-reader".into())
        .spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            // Receiver dropped: the sidecar was replaced.
                            break;
                        }
                    }
                    Err(err) => {
                        log::warn!("ASR sidecar stdout read error: {err}");
                        break;
                    }
                }
            }
        })
        .ok();
    parking_lot::Mutex::new(rx)
}

impl ASREngine for Qwen3AsrSidecar {
    fn set_progress_tracker(&mut self, tracker: super::engine::ProgressTracker) {
        self.progress = Some(tracker);
    }
    fn set_resource_dir(&mut self, dir: PathBuf) {
        self.resource_dir = Some(dir);
    }

    /// Ask the sidecar to compute its CUDA capability snapshot now.
    ///
    /// The probe only used to run inside the first `load_model`'s warm
    /// imports, so a load decision made before any load always read
    /// `cuda_available=false`/pending and sent GPU-capable machines to the
    /// CPU. This fires the warmup without loading anything.
    ///
    /// The ack means "started", not "done": the warm imports block the
    /// sidecar's main thread for 10-60s on a cold filesystem, during which
    /// `status` cannot answer at all. That silence is the normal shape of a
    /// probe in flight — the same semantics `load_in_flight` exists for —
    /// so this sets it, which keeps the mute window from being recorded as
    /// an engine error. Completion is observed through
    /// `engine_status().cuda_probe_pending` flipping to `false`.
    fn probe_cuda(&mut self) -> Result<(), String> {
        if self.use_fallback {
            return Ok(());
        }
        if self.child.is_none() {
            return Err("ASR sidecar is not running".into());
        }
        match self.send_command(json!({"cmd": "probe_cuda"})) {
            Ok(resp) => {
                if resp.get("status") == Some(&Value::String("error".into())) {
                    let err = resp
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("probe_cuda failed");
                    return Err(err.to_string());
                }
                // The sidecar's main thread now blocks in the warm imports
                // and cannot answer `status` until the probe lands. Treat
                // that silence as expected, exactly like a model load.
                self.load_in_flight = true;
                self.consecutive_probe_timeouts = 0;
                Ok(())
            }
            Err(e) => Err(format!("Could not reach the ASR sidecar: {e}")),
        }
    }

    fn initialize(&mut self) -> Result<(), String> {
        if self.is_circuit_breaker_open() {
            return Err(
                "ASR sidecar circuit breaker is open (cooling down after crash loop).".into(),
            );
        }

        log::info!("Initializing Qwen3-ASR sidecar process...");

        let Some(python) = Self::find_python() else {
            self.fallback("Python 3 was not found on PATH");
            return Err(
                "Python 3 was not found on PATH. Install Python and restart Reflow.".into(),
            );
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
                .env("PYTHONUNBUFFERED", "1")
                .env(
                    "REFLOW_NETWORK_POLICY",
                    crate::platform::PlatformSys::get_app_dir().join("network-policy.json"),
                )
                .env("HF_HUB_DISABLE_TELEMETRY", "1")
                .env("HF_HUB_DISABLE_XET", "1");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
            }

            match cmd.spawn() {
                Ok(mut child) => {
                    let stdin = child.stdin.take();
                    let responses = child.stdout.take().map(spawn_response_reader);

                    self.child = Some(child);
                    self.stdin = stdin.map(|stdin| Arc::new(parking_lot::Mutex::new(stdin)));
                    self.responses = responses;

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
                    log::warn!("Could not spawn python sidecar (attempt {attempt}/3): {err}");
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

        let manifest = ASR_MODELS.iter().find(|m| {
            Path::new(model_dir).file_name().and_then(|n| n.to_str()) == Some(m.dir_name)
        });
        let cmd = json!({
            "cmd": "load_model",
            "model_id": manifest.map(|m| m.id),
            "expected_bytes": manifest.map(|m| m.download_bytes),
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
                    self.load_in_flight = false;
                    return Err(err.to_string());
                }
                // "loading" | "ok" | "already-loading" — completion is
                // observable through engine_status().
                self.load_in_flight = true;
                self.consecutive_probe_timeouts = 0;
                self.status_cache.loaded = false;
                self.status_cache.is_loading = true;
                self.status_cache.backend = "loading…".into();
                self.status_cache.error = None;
                Ok(())
            }
            Err(e) => {
                self.load_in_flight = false;
                Err(format!("Could not reach the ASR sidecar: {e}"))
            }
        }
    }

    fn install_model_dir(&mut self, model_dir: &str, repo: &str) -> Result<(), String> {
        crate::network_policy::check_download()?;
        if self.use_fallback {
            return Err("ASR runtime unavailable".into());
        }
        let manifest = ASR_MODELS.iter().find(|m| m.repo == repo);
        let cmd = json!({
            "cmd": "install_model",
            "revision": manifest.map(|m| m.revision),
            "weight_files": manifest.map(|m| vec![json!({"filename": m.filename, "sha256": m.sha256})]),
            "model_id": manifest.map(|m| m.id),
            "expected_bytes": manifest.map(|m| m.download_bytes),
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
        // The process owns all model allocations. Killing it frees them without
        // waiting behind inference or a wedged input/output pipe.
        self.kill_child();
        self.load_in_flight = false;
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
        super::languages::language_name(language)?;
        self.detected_language = if language == "auto" {
            "en".into()
        } else {
            language.to_string()
        };

        if self.use_fallback {
            return self.fallback_mock.start_stream(language, vocabulary);
        }

        // New utterance, new audio budget.
        self.pushed_samples = 0;
        if let Some(writer) = &self.writer {
            writer.cancelled.store(false, Ordering::Release);
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
            return self.fallback_mock.push_audio(samples_16k_mono);
        }

        self.pushed_samples = self.pushed_samples.saturating_add(samples_16k_mono.len());
        let deadline = std::time::Instant::now() + TIMEOUT_PUSH_AUDIO;

        // Keep each binary frame write bounded. This also protects callers that
        // submit a large buffer (for example external audio), while normal
        // microphone capture still benefits from its ~0.5 s batching.
        for chunk in audio_chunks(samples_16k_mono) {
            let pcm_bytes = AudioResampler::f32_to_pcm16_bytes(chunk);
            let len = pcm_bytes.len() as u32;

            let mut header = [0u8; 13];
            header[0] = 0x01; // binary frame marker
            header[1..5].copy_from_slice(&0u32.to_le_bytes()); // session_id
            header[5..9].copy_from_slice(&0u32.to_le_bytes()); // sequence_id
            header[9..13].copy_from_slice(&len.to_le_bytes()); // len

            let mut frame = Vec::with_capacity(header.len() + pcm_bytes.len());
            frame.extend_from_slice(&header);
            frame.extend_from_slice(&pcm_bytes);
            if let Err(error) = self.write_until(
                PipePayload::Bytes(frame),
                "push_audio",
                deadline,
                TIMEOUT_PUSH_AUDIO,
            ) {
                if error == CANCELLED_TRANSPORT {
                    return Ok(None);
                }
                return Err(error);
            }
        }

        Ok(None)
    }

    fn get_partial_transcript(&mut self) -> Result<String, String> {
        if self.use_fallback {
            return self.fallback_mock.get_partial_transcript();
        }
        Ok(String::new())
    }

    fn stop_stream(&mut self) -> Result<String, String> {
        if self.use_fallback {
            return self.fallback_mock.stop_stream();
        }

        let cmd = json!({"cmd": "stop_stream"});
        // Budget derived from the audio actually captured, not a constant.
        let budget = transcribe_timeout(self.pushed_samples);
        log::debug!(
            "Transcribing {:.1}s of audio with a {:.0}s budget",
            self.pushed_samples as f32 / 16_000.0,
            budget.as_secs_f32()
        );
        match self.send_command_timeout(cmd, budget) {
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
                // A transcript that silently omits the end of what was said is
                // worse than one that says it was cut. Keep the notice so the
                // session can show it.
                self.last_warning = resp
                    .get("warning")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                if let Some(warning) = &self.last_warning {
                    log::warn!("ASR: {warning}");
                }
                let text = resp
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                Ok(text.to_string())
            }
            Err(e) if e == CANCELLED_TRANSPORT => {
                self.last_warning = None;
                Ok(String::new())
            }
            Err(e) => {
                log::error!("stop_stream failed: {e}");
                Err(e)
            }
        }
    }

    fn cancellation_signal(&self) -> Option<super::engine::InferenceCancellation> {
        let writer = Arc::clone(self.writer.as_ref()?);
        Some(Arc::new(move || {
            writer.cancelled.store(true, Ordering::Release);
            // Never acquire the pipe lock on the caller's thread. The callback
            // remains safe while audio submission holds a blocked OS write.
            if let Err(error) = writer.enqueue(b"{\"cmd\":\"cancel_stream\",\"id\":0}\n".to_vec()) {
                log::warn!("Could not interrupt ASR inference: {error}");
            }
        }))
    }

    fn cancel_stream(&mut self) -> Result<(), String> {
        if self.use_fallback {
            return self.fallback_mock.cancel_stream();
        }
        let _ = self.send_command(json!({"cmd": "cancel_stream"}));
        Ok(())
    }

    fn get_detected_language(&self) -> String {
        self.detected_language.clone()
    }

    fn take_last_warning(&mut self) -> Option<String> {
        self.last_warning.take()
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
        // The sidecar answered, so any earlier slow polls were transient.
        self.consecutive_probe_timeouts = 0;
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
            // The sidecar has spoken and the load is no longer pending, so
            // future probe silence is a real fault rather than warmup.
            if !is_loading {
                self.load_in_flight = false;
            }
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
                cuda_probe_pending: resp
                    .get("cuda_probe_pending")
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
                spill_detected: resp
                    .get("spill_detected")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                spill_reasons: resp
                    .get("spill_reasons")
                    .and_then(|v| v.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                spill_checked: resp
                    .get("spill_checked")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                failure_kind: resp
                    .get("failure_kind")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(String::from),
                precision: resp
                    .get("precision")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                load_seconds: resp
                    .get("load_seconds")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as f32,
                warmup_rtf: resp
                    .get("warmup_rtf")
                    .and_then(|v| v.as_f64())
                    .map(|v| v as f32),
            };
            if self.status_cache.spill_detected {
                log::error!(
                    "VRAM spill detected: {}",
                    self.status_cache.spill_reasons.join("; ")
                );
            }
            // The sidecar is the only place that can answer
            // `torch.cuda.is_available()`. Publish it so the capability probe
            // can distinguish "no GPU" from "GPU present, CUDA unusable"
            // instead of inferring one from the other.
            crate::capability::set_torch_cuda(
                self.status_cache.cuda_available,
                self.status_cache.torch_cuda_version.clone(),
            );
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
    use super::*;

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

    /// A wedged sidecar used to deadlock the caller forever while holding the
    /// engine write lock. Every command must now have a bounded wait, and the
    /// resulting error must be recognisable as a timeout.
    #[test]
    fn every_command_has_a_bounded_timeout() {
        for command in [
            "ping",
            "status",
            "load_model",
            "install_model",
            "start_stream",
            "push_audio_b64",
            "stop_stream",
            "cancel_stream",
            "unload_model",
            "something_new",
        ] {
            let timeout = Qwen3AsrSidecar::timeout_for(command);
            assert!(timeout > Duration::ZERO, "{command} has no timeout");
            assert!(
                timeout <= TIMEOUT_TRANSCRIBE,
                "{command} timeout {timeout:?} exceeds the transcription bound"
            );
        }
    }

    #[test]
    fn hot_path_commands_are_not_given_transcription_timeouts() {
        // The status poller and the hotkey path must fail fast; waiting three
        // minutes on a status query is what made the UI hang.
        assert_eq!(Qwen3AsrSidecar::timeout_for("ping"), TIMEOUT_FAST);
        assert!(Qwen3AsrSidecar::timeout_for("status") < TIMEOUT_CONTROL);
        // Only the whole-utterance transcription gets the long bound.
        assert_eq!(
            Qwen3AsrSidecar::timeout_for("stop_stream"),
            TIMEOUT_TRANSCRIBE
        );
    }

    /// `status` is polled while a multi-gigabyte checkpoint is being quantized
    /// and warmed, which starves the interpreter for seconds at a time. It gets
    /// more headroom than `ping` for that reason — but still nothing like the
    /// transcription bound, because a genuinely mute sidecar must be noticed.
    #[test]
    fn status_has_more_headroom_than_ping_but_stays_bounded() {
        let status = Qwen3AsrSidecar::timeout_for("status");
        assert!(
            status > TIMEOUT_FAST,
            "status budget {status:?} must exceed the bare liveness budget"
        );
        assert!(
            status < TIMEOUT_TRANSCRIBE,
            "status budget {status:?} must stay far below the transcription bound"
        );
    }

    /// The teardown policy is the whole point of the fix: a read-only probe
    /// timing out must not kill a sidecar that is legitimately mid-load.
    #[test]
    fn only_real_operations_are_treated_as_fatal_on_timeout() {
        assert!(Qwen3AsrSidecar::is_probe("status"));
        assert!(Qwen3AsrSidecar::is_probe("ping"));
        for command in [
            "load_model",
            "install_model",
            "start_stream",
            "push_audio_b64",
            "stop_stream",
            "unload_model",
        ] {
            assert!(
                !Qwen3AsrSidecar::is_probe(command),
                "{command} must still tear the child down when it wedges"
            );
        }
    }

    /// The budget has to grow with the audio, or raising the audio limit just
    /// converts a truncated transcript into a killed sidecar.
    ///
    /// `stop_stream` is not a read-only probe: exceeding its budget tears the
    /// child down and counts a crash strike. Against the old fixed 180s, the
    /// measured worst case of 2.4x realtime on CPU covered only ~75s of audio.
    #[test]
    fn the_transcription_budget_scales_with_the_audio() {
        let one_second = 16_000;

        // Short utterances keep the floor; there is no reason to shrink below it.
        assert_eq!(transcribe_timeout(one_second * 5), TIMEOUT_TRANSCRIBE);

        // Ten minutes of audio must get far more than the old fixed budget.
        let ten_minutes = transcribe_timeout(one_second * 600);
        assert!(
            ten_minutes > TIMEOUT_TRANSCRIBE,
            "600s of audio still only got {ten_minutes:?}"
        );
        // And enough to cover the measured worst-case CPU throughput.
        assert!(
            ten_minutes.as_secs_f32() >= 600.0 * 2.4,
            "budget {ten_minutes:?} is under the measured CPU cost of 600s of audio"
        );

        // An hour is admitted by the Python side, so it must be budgeted for.
        let one_hour = transcribe_timeout(one_second * 3600);
        assert!(one_hour > ten_minutes);
        assert!(one_hour.as_secs_f32() >= 3600.0 * WORST_CASE_RTF);

        // Still bounded, so a wedged sidecar is eventually noticed.
        assert!(transcribe_timeout(one_second * 100_000) <= TIMEOUT_TRANSCRIBE_MAX);
    }

    #[test]
    fn timeout_errors_are_distinguishable_from_engine_errors() {
        let err = engine_timeout_error("stop_stream", Duration::from_secs(180));
        assert!(is_engine_timeout(&err), "{err}");
        assert!(err.contains("stop_stream"));
        assert!(!is_engine_timeout("Transcription failed"));
        assert!(!is_engine_timeout("Python 3 was not found on PATH"));
    }

    /// The sidecar cannot answer any probe while its main thread is blocked
    /// warming native imports, which takes 9-52s in practice. That silence is
    /// the normal shape of a load in progress. Recording it as an engine error
    /// made `engine_status().error` report "Model failed to load: engine_timeout"
    /// for a model that was loading perfectly well — and because
    /// `session::start_microphone_at` refuses to record when the engine reports
    /// an error or `!loaded`, it silently killed the hotkey for the session.
    #[test]
    fn probe_timeouts_during_a_load_are_not_reported_as_errors() {
        let mut sidecar = Qwen3AsrSidecar::new();
        sidecar.load_in_flight = true;
        sidecar.status_cache.is_loading = true;

        for _ in 0..(PROBE_TIMEOUT_TOLERANCE + 5) {
            let err = sidecar.on_command_timeout("status", TIMEOUT_STATUS);
            assert!(is_engine_timeout(&err));
        }

        assert!(
            sidecar.status_cache.error.is_none(),
            "a warming sidecar must not be reported as failed, got {:?}",
            sidecar.status_cache.error
        );
        assert!(
            sidecar.status_cache.is_loading,
            "the load must still be reported as in progress"
        );
    }

    /// The tolerance must still work once the load has resolved: a sidecar that
    /// has gone permanently mute is a real fault and has to be surfaced.
    #[test]
    fn persistent_probe_timeouts_outside_a_load_still_surface() {
        let mut sidecar = Qwen3AsrSidecar::new();
        sidecar.load_in_flight = false;
        sidecar.status_cache.is_loading = false;

        for _ in 0..(PROBE_TIMEOUT_TOLERANCE - 1) {
            sidecar.on_command_timeout("status", TIMEOUT_STATUS);
        }
        assert!(
            sidecar.status_cache.error.is_none(),
            "a couple of slow polls are not a fault"
        );

        sidecar.on_command_timeout("status", TIMEOUT_STATUS);
        assert!(
            sidecar.status_cache.error.is_some(),
            "a persistently mute sidecar must be reported"
        );
    }

    /// A short status budget is only safe because replies are id-matched, and it
    /// is necessary because every status poll occupies the single-threaded ASR
    /// actor for its whole duration — including the hotkey's `start_stream`.
    #[test]
    fn status_budget_stays_short_enough_not_to_stall_the_actor() {
        let status = Qwen3AsrSidecar::timeout_for("status");
        assert!(
            status <= Duration::from_secs(10),
            "status budget {status:?} would park the ASR actor too long"
        );
    }

    #[test]
    fn actor_interrupts_python_decode_and_matches_out_of_order_replies() {
        let Some(python) = Qwen3AsrSidecar::find_python() else {
            return;
        };
        let script = r#"
import sys,json,struct
pending=None
audio_samples=0
stream=sys.stdin.buffer
while True:
    first=stream.read(1)
    if not first: break
    if first==b'\x01':
        _,_,length=struct.unpack('<III',stream.read(12))
        pcm=stream.read(length)
        assert len(pcm)==length
        assert all(v==8191 for v in struct.unpack('<'+'h'*(length//2),pcm))
        audio_samples+=length//2
        continue
    request=json.loads(first+stream.readline())
    cmd=request['cmd']
    if cmd=='quit': break
    if cmd=='stop_stream':
        assert audio_samples==1000
        pending=request
        print('decoding',file=sys.stderr,flush=True)
    else:
        print(json.dumps({'id':request['id'],'status':'ok'}),flush=True)
        if cmd=='cancel_stream' and pending:
            print(json.dumps({'id':pending['id'],'status':'ok','text':''}),flush=True)
            pending=None
"#;
        let mut command = Command::new(python);
        command
            .arg("-u")
            .arg("-c")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().unwrap();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let stderr = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            let mut line = String::new();
            BufReader::new(stderr).read_line(&mut line).unwrap();
            ready_tx.send(line).unwrap();
        });
        let mut engine = Qwen3AsrSidecar::new();
        engine.stdin = child
            .stdin
            .take()
            .map(|stdin| Arc::new(parking_lot::Mutex::new(stdin)));
        engine.responses = child.stdout.take().map(spawn_response_reader);
        engine.child = Some(child);
        let handle = crate::asr::AsrHandle::spawn(Box::new(engine), 8);
        handle.start_stream_blocking(7, "en", &[]).unwrap();
        handle.push_audio_blocking(7, 0, &vec![0.25; 1000]).unwrap();
        let decoding = handle.clone();
        let stop = std::thread::spawn(move || decoding.stop_stream_blocking(7).unwrap());
        assert_eq!(
            ready_rx
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .trim(),
            "decoding"
        );
        handle.cancel_stream_blocking(7).unwrap();
        assert_eq!(stop.join().unwrap(), "");
        handle.start_stream_blocking(8, "en", &[]).unwrap();
        handle.cancel_stream_blocking(8).unwrap();
    }

    /// Sending a command with no live subprocess must return an error rather
    /// than panic or block.
    #[test]
    fn command_without_a_subprocess_fails_fast() {
        let mut sidecar = Qwen3AsrSidecar::new();
        let started = std::time::Instant::now();
        let result = sidecar.send_command(serde_json::json!({"cmd": "status"}));
        assert!(result.is_err());
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "must not wait on a pipe that was never opened"
        );
    }

    #[test]
    fn binary_audio_frame_header_format() {
        let pcm_bytes = [0x12, 0x34, 0x56, 0x78];
        let len = pcm_bytes.len() as u32;

        let mut header = [0u8; 13];
        header[0] = 0x01;
        header[1..5].copy_from_slice(&7u32.to_le_bytes());
        header[5..9].copy_from_slice(&3u32.to_le_bytes());
        header[9..13].copy_from_slice(&len.to_le_bytes());

        assert_eq!(header[0], 0x01);
        assert_eq!(u32::from_le_bytes(header[1..5].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(header[5..9].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(header[9..13].try_into().unwrap()), 4);
    }

    /// A status reply that predates `cuda_probe_pending` (an older sidecar)
    /// must deserialize with the field defaulting to `false`, not fail or
    /// read as pending forever.
    #[test]
    fn engine_status_tolerates_a_reply_without_the_probe_field() {
        let legacy = serde_json::json!({
            "status": "ok",
            "loaded": false,
            "device": "none",
            "backend": "not loaded",
            "is_loading": false,
            "cuda_available": true,
        });
        let parsed: EngineStatus = serde_json::from_value(legacy).expect("legacy reply parses");
        assert!(parsed.cuda_available);
        assert!(
            !parsed.cuda_probe_pending,
            "an absent field must not read as probe-pending, or the startup \
             load would wait on a reply that will never change"
        );
    }

    /// The startup load decision waits on this field, so its two meanings
    /// must stay distinct: `pending=true` means "not looked yet" and
    /// `cuda_available=false` with `pending=false` means "looked, and no".
    #[test]
    fn probe_pending_is_distinct_from_an_answered_no() {
        let base = serde_json::json!({
            "status": "ok",
            "loaded": false,
            "device": "none",
            "backend": "not loaded",
            "is_loading": false,
            "cuda_available": false,
        });
        let mut pending = base.clone();
        pending["cuda_probe_pending"] = serde_json::json!(true);
        let mut answered_no = base;
        answered_no["cuda_probe_pending"] = serde_json::json!(false);
        let pending: EngineStatus = serde_json::from_value(pending).unwrap();
        let answered_no: EngineStatus = serde_json::from_value(answered_no).unwrap();
        assert!(pending.cuda_probe_pending);
        assert!(!answered_no.cuda_probe_pending);
        assert!(!pending.cuda_available && !answered_no.cuda_available);
    }
}
