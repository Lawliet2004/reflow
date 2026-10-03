use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[derive(Default)]
pub enum AppStateEnum {
    Uninitialized,
    Initializing,
    Idle,
    LoadingModel,
    #[default]
    Ready,
    Recording,
    Processing,
    Injecting,
    Error,
    Updating,
}

/// Timing for one ASR segment submission.
///
/// Under whole-utterance transcription there is exactly one segment
/// (`sequence_id == 0`) covering the entire session. Under chunk-on-silence
/// there is one entry per VAD-delimited segment plus one for the tail.
/// Offsets are relative to `speech_ended_at` for the tail and to
/// `recording_started_at` for mid-utterance segments, expressed as
/// milliseconds since the session's `hotkey_pressed_at`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SegmentTiming {
    pub sequence_id: u32,
    /// Milliseconds from session start to the moment the segment was handed
    /// to the ASR engine.
    pub submitted_at_ms: u64,
    /// Milliseconds from session start to the moment its transcript came
    /// back. `None` while the segment is still in flight (or was discarded).
    pub completed_at_ms: Option<u64>,
    /// Wall-clock milliseconds the engine spent on this segment.
    pub compute_ms: Option<u64>,
    /// Length of the audio the segment covers.
    pub audio_ms: u64,
    /// `compute_ms / audio_ms`. `None` until the segment completes.
    pub rtf: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyMetrics {
    #[serde(default)]
    pub llm_generation: Option<crate::rewrite::client::GenerationMetrics>,
    #[serde(default)]
    pub dropped_audio_chunks: u64,
    #[serde(default)]
    pub failed_audio_pushes: u64,
    #[serde(default)]
    pub peak_audio_queue_chunks: usize,
    #[serde(default)]
    pub llm_was_warm: bool,
    /// Hotkey press (captured inside the OS hook callback) to the audio
    /// stream actually running. Covers thread spawn, SendInput, engine IPC
    /// and tokio scheduling.
    pub hotkey_to_recording_ms: u64,
    pub recording_to_first_audio_ms: u64,
    pub audio_to_first_partial_ms: u64,
    /// Key release to the authoritative final transcript.
    pub speech_end_to_final_ms: u64,
    pub final_to_injection_ms: u64,
    /// Time spent getting the refinement runtime ready on the stop path.
    /// Non-zero means the LLM was cold-started inside the critical path,
    /// which pre-warming is meant to eliminate.
    #[serde(default)]
    pub llm_startup_ms: u64,
    /// Deterministic Stage 1 formatting only.
    #[serde(default)]
    pub formatting_ms: u64,
    /// Stage 2 LLM refinement only. Zero when no rewrite was attempted.
    #[serde(default)]
    pub rewrite_ms: u64,
    /// `true` when the Stage 2 LLM actually produced the injected text.
    #[serde(default)]
    pub rewrite_applied: bool,
    /// The headline number: key release to text in the target application.
    #[serde(default)]
    pub release_to_inserted_ms: u64,
    /// Hotkey press to insertion complete — the full session wall clock.
    pub total_duration_ms: u64,
    /// Length of the captured audio.
    #[serde(default)]
    pub audio_duration_ms: u64,
    /// ASR compute time divided by audio duration. `0.0` when unknown.
    /// Values above 1.0 mean transcription is slower than real time.
    #[serde(default)]
    pub rtf: f32,
    #[serde(default)]
    pub segments: Vec<SegmentTiming>,
    pub last_updated: String,
}

impl Default for LatencyMetrics {
    fn default() -> Self {
        Self {
            hotkey_to_recording_ms: 0,
            llm_generation: None,
            dropped_audio_chunks: 0,
            failed_audio_pushes: 0,
            peak_audio_queue_chunks: 0,
            llm_was_warm: false,
            recording_to_first_audio_ms: 0,
            audio_to_first_partial_ms: 0,
            speech_end_to_final_ms: 0,
            final_to_injection_ms: 0,
            llm_startup_ms: 0,
            formatting_ms: 0,
            rewrite_ms: 0,
            rewrite_applied: false,
            release_to_inserted_ms: 0,
            total_duration_ms: 0,
            audio_duration_ms: 0,
            rtf: 0.0,
            segments: Vec::new(),
            last_updated: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// Rolling p50/p95 across the recent dictations in this app session.
/// A single dictation says nothing about whether a threshold is met; the
/// acceptance table is written in p50/p95, so the app has to keep the
/// distribution rather than the last sample.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LatencyPercentiles {
    pub samples: usize,
    pub hotkey_to_recording_p50_ms: u64,
    pub hotkey_to_recording_p95_ms: u64,
    pub speech_end_to_final_p50_ms: u64,
    pub speech_end_to_final_p95_ms: u64,
    pub rewrite_p50_ms: u64,
    pub rewrite_p95_ms: u64,
    pub release_to_inserted_p50_ms: u64,
    pub release_to_inserted_p95_ms: u64,
    pub rtf_p50: f32,
    pub rtf_p95: f32,
    /// Fraction of dictations whose injected text came from the LLM. The
    /// Balanced preset's acceptance criterion is >= 0.9; a deadline that
    /// fires often is worse than no LLM because output stops being
    /// reproducible between dictations.
    pub llm_applied_rate: f32,
}

/// Nearest-rank percentile index: `ceil(pct * n) - 1`, clamped.
///
/// This is the conventional definition, and the one the acceptance thresholds
/// assume. Interpolating between samples would report a latency that was never
/// actually observed, which is the wrong thing for a p95 gate.
fn percentile_index(len: usize, pct: f32) -> usize {
    debug_assert!(len > 0);
    let rank = (pct.clamp(0.0, 1.0) * len as f32).ceil() as usize;
    rank.max(1).min(len) - 1
}

/// `sorted` must be ascending.
fn percentile_u64(sorted: &[u64], pct: f32) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[percentile_index(sorted.len(), pct)]
}

fn percentile_f32(sorted: &[f32], pct: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[percentile_index(sorted.len(), pct)]
}

#[derive(Debug)]
pub struct LatencyHistory {
    capacity: usize,
    entries: std::collections::VecDeque<LatencyMetrics>,
}

impl Default for LatencyHistory {
    fn default() -> Self {
        Self::new(50)
    }
}

impl LatencyHistory {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: std::collections::VecDeque::new(),
        }
    }

    pub fn push(&mut self, metrics: &LatencyMetrics) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(metrics.clone());
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Most recent entries, newest last.
    pub fn recent(&self, count: usize) -> Vec<LatencyMetrics> {
        let skip = self.entries.len().saturating_sub(count);
        self.entries.iter().skip(skip).cloned().collect()
    }

    pub fn percentiles(&self) -> LatencyPercentiles {
        if self.entries.is_empty() {
            return LatencyPercentiles::default();
        }
        let sorted_by = |f: fn(&LatencyMetrics) -> u64| -> Vec<u64> {
            let mut v: Vec<u64> = self.entries.iter().map(f).collect();
            v.sort_unstable();
            v
        };
        let hotkey = sorted_by(|m| m.hotkey_to_recording_ms);
        let final_ms = sorted_by(|m| m.speech_end_to_final_ms);
        let rewrite = sorted_by(|m| m.rewrite_ms);
        let inserted = sorted_by(|m| m.release_to_inserted_ms);
        let mut rtf: Vec<f32> = self
            .entries
            .iter()
            .map(|m| m.rtf)
            .filter(|r| *r > 0.0)
            .collect();
        rtf.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let applied = self.entries.iter().filter(|m| m.rewrite_applied).count();

        LatencyPercentiles {
            samples: self.entries.len(),
            hotkey_to_recording_p50_ms: percentile_u64(&hotkey, 0.50),
            hotkey_to_recording_p95_ms: percentile_u64(&hotkey, 0.95),
            speech_end_to_final_p50_ms: percentile_u64(&final_ms, 0.50),
            speech_end_to_final_p95_ms: percentile_u64(&final_ms, 0.95),
            rewrite_p50_ms: percentile_u64(&rewrite, 0.50),
            rewrite_p95_ms: percentile_u64(&rewrite, 0.95),
            release_to_inserted_p50_ms: percentile_u64(&inserted, 0.50),
            release_to_inserted_p95_ms: percentile_u64(&inserted, 0.95),
            rtf_p50: percentile_f32(&rtf, 0.50),
            rtf_p95: percentile_f32(&rtf, 0.95),
            llm_applied_rate: applied as f32 / self.entries.len() as f32,
        }
    }
}

/// Everything the diagnostics waterfall needs in one payload.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LatencyReport {
    pub last: LatencyMetrics,
    pub percentiles: LatencyPercentiles,
    /// Newest last.
    pub recent: Vec<LatencyMetrics>,
}

/// Build a report from the live context. Shared by the Tauri command and the
/// CLI so both report identical numbers.
pub fn build_latency_report(ctx: &crate::context::AppContext) -> LatencyReport {
    let history = ctx.latency_history.read();
    LatencyReport {
        last: ctx.last_latency_metrics.read().clone(),
        percentiles: history.percentiles(),
        recent: history.recent(20),
    }
}

/// Where the most recent report is persisted so a separate CLI invocation can
/// read real measurements instead of printing invented ones.
pub fn latency_report_path() -> std::path::PathBuf {
    crate::platform::PlatformSys::get_app_dir().join("latency-report.json")
}

pub fn persist_latency_report(report: &LatencyReport, path: &std::path::Path) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(report) {
        Ok(json) => {
            if let Err(err) = std::fs::write(path, json) {
                log::warn!("Could not persist latency report: {err}");
            }
        }
        Err(err) => log::warn!("Could not serialize latency report: {err}"),
    }
}

/// Load the persisted report. `None` when no dictation has completed yet —
/// callers must say so rather than substituting placeholder numbers.
pub fn load_latency_report() -> Option<LatencyReport> {
    let path = latency_report_path();
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

#[derive(Debug, Clone, Copy)]
pub struct SegmentTimer {
    pub sequence_id: u32,
    pub submitted_at: Instant,
    pub completed_at: Option<Instant>,
    pub audio_ms: u64,
}

#[derive(Debug, Default)]
pub struct LatencyTimer {
    pub dropped_audio_chunks: u64,
    pub failed_audio_pushes: u64,
    pub peak_audio_queue_chunks: usize,
    pub llm_was_warm: bool,
    pub hotkey_pressed_at: Option<Instant>,
    pub recording_started_at: Option<Instant>,
    pub first_audio_at: Option<Instant>,
    pub first_partial_at: Option<Instant>,
    pub speech_ended_at: Option<Instant>,
    /// The moment the capture pipeline confirmed every captured sample had
    /// been handed to the ASR engine. Distinct from `speech_ended_at`, which
    /// is the key release.
    pub audio_drained_at: Option<Instant>,
    pub final_asr_at: Option<Instant>,
    /// Set once the refinement runtime is confirmed ready on the stop path.
    /// When the runtime is already warm this lands microseconds after
    /// `final_asr_at`; a cold start shows up as `llm_startup_ms`.
    pub llm_ready_at: Option<Instant>,
    pub formatting_finished_at: Option<Instant>,
    /// Set only when a Stage 2 rewrite is actually dispatched, so
    /// `rewrite_ms` measures the LLM and not the formatter.
    pub rewrite_started_at: Option<Instant>,
    pub rewrite_finished_at: Option<Instant>,
    /// `true` only when the rewrite survived the safety gate and its text
    /// was the text actually injected. A rewrite that ran and was rejected
    /// still contributes to `rewrite_ms` — it was on the critical path.
    pub rewrite_applied: bool,
    pub injection_finished_at: Option<Instant>,
    pub segments: Vec<SegmentTimer>,
}

impl LatencyTimer {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Record that `sequence_id` was handed to the ASR engine.
    pub fn segment_submitted(&mut self, sequence_id: u32, audio_ms: u64) {
        self.segments.retain(|s| s.sequence_id != sequence_id);
        self.segments.push(SegmentTimer {
            sequence_id,
            submitted_at: Instant::now(),
            completed_at: None,
            audio_ms,
        });
    }

    /// Record that `sequence_id`'s transcript came back.
    pub fn segment_completed(&mut self, sequence_id: u32) {
        let now = Instant::now();
        if let Some(seg) = self
            .segments
            .iter_mut()
            .find(|s| s.sequence_id == sequence_id)
        {
            seg.completed_at = Some(now);
        }
    }

    /// Total audio covered by submitted segments.
    pub fn segment_audio_ms(&self) -> u64 {
        self.segments.iter().map(|s| s.audio_ms).sum()
    }

    /// Total engine compute time across completed segments. This is the
    /// numerator of RTF and is independent of whether the work overlapped
    /// with speech.
    pub fn segment_compute_ms(&self) -> u64 {
        self.segments
            .iter()
            .filter_map(|s| {
                s.completed_at
                    .map(|c| c.duration_since(s.submitted_at).as_millis() as u64)
            })
            .sum()
    }

    pub fn to_metrics(&self, audio_duration_ms: u64) -> LatencyMetrics {
        let delta = |from: Option<Instant>, to: Option<Instant>| -> u64 {
            match (from, to) {
                (Some(a), Some(b)) if b >= a => b.duration_since(a).as_millis() as u64,
                _ => 0,
            }
        };

        let session_start = self.hotkey_pressed_at.or(self.recording_started_at);
        let session_end = self
            .injection_finished_at
            .or(self.rewrite_finished_at)
            .or(self.formatting_finished_at)
            .or(self.llm_ready_at)
            .or(self.final_asr_at);

        // Prefer the measured per-segment compute time. Fall back to the
        // release→final window, which is the whole of ASR when nothing
        // overlaps with speech.
        let compute_ms = {
            let measured = self.segment_compute_ms();
            if measured > 0 {
                measured
            } else {
                delta(self.speech_ended_at, self.final_asr_at)
            }
        };
        let audio_ms = if audio_duration_ms > 0 {
            audio_duration_ms
        } else {
            self.segment_audio_ms()
        };
        let rtf = if audio_ms > 0 && compute_ms > 0 {
            compute_ms as f32 / audio_ms as f32
        } else {
            0.0
        };

        let segments = self
            .segments
            .iter()
            .map(|s| {
                let compute_ms = s
                    .completed_at
                    .map(|c| c.duration_since(s.submitted_at).as_millis() as u64);
                SegmentTiming {
                    sequence_id: s.sequence_id,
                    submitted_at_ms: delta(session_start, Some(s.submitted_at)),
                    completed_at_ms: s.completed_at.map(|c| delta(session_start, Some(c))),
                    compute_ms,
                    audio_ms: s.audio_ms,
                    rtf: compute_ms.and_then(|c| {
                        if s.audio_ms > 0 {
                            Some(c as f32 / s.audio_ms as f32)
                        } else {
                            None
                        }
                    }),
                }
            })
            .collect();

        LatencyMetrics {
            llm_generation: None,
            dropped_audio_chunks: self.dropped_audio_chunks,
            failed_audio_pushes: self.failed_audio_pushes,
            peak_audio_queue_chunks: self.peak_audio_queue_chunks,
            llm_was_warm: self.llm_was_warm,
            hotkey_to_recording_ms: delta(self.hotkey_pressed_at, self.recording_started_at),
            recording_to_first_audio_ms: delta(self.recording_started_at, self.first_audio_at),
            audio_to_first_partial_ms: delta(self.first_audio_at, self.first_partial_at),
            speech_end_to_final_ms: delta(self.speech_ended_at, self.final_asr_at),
            final_to_injection_ms: delta(self.final_asr_at, self.injection_finished_at),
            llm_startup_ms: delta(self.final_asr_at, self.llm_ready_at),
            formatting_ms: delta(
                self.llm_ready_at.or(self.final_asr_at),
                self.formatting_finished_at,
            ),
            rewrite_ms: delta(self.rewrite_started_at, self.rewrite_finished_at),
            rewrite_applied: self.rewrite_applied,
            release_to_inserted_ms: delta(self.speech_ended_at, self.injection_finished_at),
            total_duration_ms: delta(session_start, session_end),
            audio_duration_ms: audio_ms,
            rtf,
            segments,
            last_updated: chrono::Utc::now().to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetrics {
    pub cpu_usage_pct: f32,
    pub app_ram_mb: f32,
    pub model_ram_mb: f32,
    /// Physical system RAM. Previously this held *used* RAM, which made every
    /// memory-headroom calculation built on it wrong.
    pub total_ram_mb: f32,
    #[serde(default)]
    pub used_ram_mb: f32,
    #[serde(default)]
    pub available_ram_mb: f32,
    /// VRAM currently in use across the device. Kept for backwards
    /// compatibility with the existing UI; prefer the explicit fields below.
    pub vram_mb: f32,
    #[serde(default)]
    pub total_vram_mb: f32,
    #[serde(default)]
    pub used_vram_mb: f32,
    /// Free VRAM measured now. This is the only figure an admission decision
    /// may use.
    #[serde(default)]
    pub free_vram_mb: f32,
    pub gpu_name: String,
    #[serde(default)]
    pub gpu_vendor: String,
    /// `true` when an NVIDIA device was enumerated. Distinct from
    /// `cuda_available`.
    #[serde(default)]
    pub gpu_present: bool,
    /// `true` only when `torch.cuda.is_available()` in the sidecar.
    #[serde(default)]
    pub cuda_available: bool,
    #[serde(default)]
    pub vulkan_available: bool,
    #[serde(default)]
    pub cpu_model: String,
    #[serde(default)]
    pub physical_cores: usize,
    #[serde(default)]
    pub logical_cores: usize,
    /// RAM attributed to the Python ASR sidecar.
    #[serde(default)]
    pub asr_ram_mb: f32,
    /// RAM attributed to `llama-server`.
    #[serde(default)]
    pub refinement_ram_mb: f32,
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
    /// Why the running ASR model differs from the one selected in Settings.
    ///
    /// `None` when they match. Populated when the requested model could not fit
    /// free VRAM and a smaller one was loaded instead, so the UI can explain the
    /// substitution rather than leaving the user to interpret it as a fault.
    #[serde(default)]
    pub asr_selection_notice: Option<String>,
}

impl Default for ModelStatus {
    fn default() -> Self {
        // Unknown, not ready: a default must never claim a model is installed
        // and loaded, or every consumer that falls back to it (web preview,
        // tests, early startup) reports a healthy engine that does not exist.
        Self {
            installed: false,
            loaded: false,
            version: String::new(),
            name: String::new(),
            size_bytes: 0,
            download_progress_pct: 0,
            download_speed_mbps: 0.0,
            backend: "none".into(),
            is_downloading: false,
            is_loading: false,
            error: None,
            gpu_available: false,
            gpu_name: String::new(),
            cuda_available: false,
            torch_cuda_version: None,
            asr_gpu_hint: None,
            asr_selection_notice: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Build a timer whose stages are separated by known, distinct offsets so
    /// that a metric hardcoded to zero (or wired to the wrong pair of
    /// timestamps) is detectable.
    fn synthetic_timer() -> LatencyTimer {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        LatencyTimer {
            hotkey_pressed_at: Some(at(0)),
            dropped_audio_chunks: 2,
            failed_audio_pushes: 1,
            peak_audio_queue_chunks: 4,
            llm_was_warm: true,
            recording_started_at: Some(at(120)),
            first_audio_at: Some(at(150)),
            first_partial_at: Some(at(400)),
            speech_ended_at: Some(at(5000)),
            audio_drained_at: Some(at(5020)),
            final_asr_at: Some(at(5300)),
            llm_ready_at: Some(at(5305)),
            formatting_finished_at: Some(at(5310)),
            rewrite_started_at: Some(at(5310)),
            rewrite_finished_at: Some(at(5800)),
            rewrite_applied: true,
            injection_finished_at: Some(at(5850)),
            segments: vec![SegmentTimer {
                sequence_id: 0,
                submitted_at: at(5020),
                completed_at: Some(at(5300)),
                audio_ms: 4900,
            }],
        }
    }

    #[test]
    fn segment_lifecycle_records_submit_and_complete() {
        let mut timer = LatencyTimer {
            hotkey_pressed_at: Some(Instant::now()),
            ..LatencyTimer::default()
        };
        timer.segment_submitted(0, 1500);
        assert_eq!(timer.segments.len(), 1);
        assert!(timer.segments[0].completed_at.is_none());
        timer.segment_completed(0);
        assert!(timer.segments[0].completed_at.is_some());
        // Re-submitting the same sequence id replaces the stale entry rather
        // than double-counting its audio.
        timer.segment_submitted(0, 1500);
        assert_eq!(timer.segments.len(), 1);
        assert_eq!(timer.segment_audio_ms(), 1500);
    }

    #[test]
    fn partial_session_reports_zero_only_for_stages_that_did_not_run() {
        let t0 = Instant::now();
        let timer = LatencyTimer {
            hotkey_pressed_at: Some(t0),
            recording_started_at: Some(t0 + Duration::from_millis(90)),
            ..LatencyTimer::default()
        };
        let m = timer.to_metrics(0);
        assert_eq!(m.hotkey_to_recording_ms, 90);
        assert_eq!(m.speech_end_to_final_ms, 0);
        assert_eq!(m.rewrite_ms, 0);
        assert_eq!(m.audio_duration_ms, 0);
        assert_eq!(m.rtf, 0.0);
    }

    #[test]
    fn reset_clears_segments() {
        let mut timer = synthetic_timer();
        timer.reset();
        assert!(timer.segments.is_empty());
        assert!(timer.hotkey_pressed_at.is_none());
        assert!(!timer.rewrite_applied);
    }
}
