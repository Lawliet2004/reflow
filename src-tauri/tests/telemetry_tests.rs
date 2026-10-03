//! Milestone 0 / Task 1: the latency pipeline must be measurable end to end.
//!
//! These live in the integration target rather than in-module because they
//! drive a real `AppContext` through a full session, which is the only place
//! the stage attribution can actually be wrong.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use reflow_lib::asr::{ASREngine, EngineStatus};
use reflow_lib::context::AppContext;
use reflow_lib::dory::DoryEvent;
use reflow_lib::session;
use reflow_lib::state::{LatencyHistory, LatencyMetrics, LatencyTimer, SegmentTimer};

fn temp_ctx(tag: &str) -> (AppContext, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("reflow_{tag}_{}", uuid::Uuid::new_v4()));
    let ctx = AppContext::bootstrap_test(dir.clone());
    (ctx, dir)
}

/// Select the verbatim tier so the stop path never tries to start
/// `llama-server`.
///
/// Without this a test's runtime depends on whether the machine happens to
/// have the refinement runtime installed, which makes timing assertions
/// meaningless and adds tens of seconds of health polling.
fn deterministic_no_llm(ctx: &AppContext) {
    let mut settings = ctx.settings_store.get();
    settings.intelligence_tier = "raw_verbatim".into();
    settings.cleanup_level = "raw".into();
    ctx.settings_store.update(settings).expect("settings");
}

/// Engine stub that records exactly how much audio had arrived when
/// `stop_stream()` was called, and can emit live partials.
struct AudioProbe {
    pushed: Arc<AtomicUsize>,
    at_stop: Arc<AtomicUsize>,
    partial: Option<String>,
}

impl AudioProbe {
    fn new(pushed: Arc<AtomicUsize>, at_stop: Arc<AtomicUsize>) -> Self {
        Self {
            pushed,
            at_stop,
            partial: None,
        }
    }

    fn with_partial(mut self, text: &str) -> Self {
        self.partial = Some(text.into());
        self
    }
}

impl ASREngine for AudioProbe {
    fn initialize(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn load_model_with_precision(
        &mut self,
        _model_dir: &str,
        _backend: &str,
        _precision: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    fn unload_model(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn is_model_loaded(&self) -> bool {
        true
    }

    fn start_stream(&mut self, _language: &str, _vocabulary: &[String]) -> Result<(), String> {
        Ok(())
    }

    fn push_audio(&mut self, samples_16k_mono: &[f32]) -> Result<Option<String>, String> {
        self.pushed
            .fetch_add(samples_16k_mono.len(), Ordering::SeqCst);
        Ok(self.partial.clone())
    }

    fn get_partial_transcript(&mut self) -> Result<String, String> {
        Ok(String::new())
    }

    fn stop_stream(&mut self) -> Result<String, String> {
        self.at_stop
            .store(self.pushed.load(Ordering::SeqCst), Ordering::SeqCst);
        Ok("probe transcript".into())
    }

    fn cancel_stream(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn get_detected_language(&self) -> String {
        "en".into()
    }

    fn get_backend_name(&self) -> String {
        "audio-probe".into()
    }

    fn engine_status(&mut self) -> EngineStatus {
        EngineStatus {
            loaded: true,
            backend: "audio-probe".into(),
            ..Default::default()
        }
    }
}

fn synthetic_timer() -> LatencyTimer {
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    LatencyTimer {
        dropped_audio_chunks: 0,
        failed_audio_pushes: 0,
        peak_audio_queue_chunks: 0,
        llm_was_warm: true,
        hotkey_pressed_at: Some(at(0)),
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
fn latency_waterfall_attributes_every_stage() {
    let m = synthetic_timer().to_metrics(4900);

    assert_eq!(m.hotkey_to_recording_ms, 120);
    assert_eq!(m.recording_to_first_audio_ms, 30);
    assert_eq!(m.audio_to_first_partial_ms, 250);
    assert_eq!(m.speech_end_to_final_ms, 300);
    assert_eq!(m.llm_startup_ms, 5);
    assert_eq!(m.formatting_ms, 5);
    assert_eq!(m.rewrite_ms, 490);
    assert_eq!(m.final_to_injection_ms, 550);
    assert_eq!(m.release_to_inserted_ms, 850);
    assert_eq!(m.total_duration_ms, 5850);
    assert_eq!(m.audio_duration_ms, 4900);
    assert!(m.rewrite_applied);
}

/// Regression guard for the three metrics the review found pinned to zero:
/// `hotkey_to_recording_ms`, `audio_to_first_partial_ms`, `total_duration_ms`.
#[test]
fn no_latency_metric_is_structurally_zero() {
    let m = synthetic_timer().to_metrics(4900);
    let named: [(&str, u64); 10] = [
        ("hotkey_to_recording_ms", m.hotkey_to_recording_ms),
        ("recording_to_first_audio_ms", m.recording_to_first_audio_ms),
        ("audio_to_first_partial_ms", m.audio_to_first_partial_ms),
        ("speech_end_to_final_ms", m.speech_end_to_final_ms),
        ("final_to_injection_ms", m.final_to_injection_ms),
        ("llm_startup_ms", m.llm_startup_ms),
        ("formatting_ms", m.formatting_ms),
        ("rewrite_ms", m.rewrite_ms),
        ("release_to_inserted_ms", m.release_to_inserted_ms),
        ("total_duration_ms", m.total_duration_ms),
    ];
    for (name, value) in named {
        assert!(value > 0, "{name} is zero for a fully populated session");
    }
    assert!(m.rtf > 0.0, "rtf must be derived, not hardcoded");
}

#[test]
fn total_duration_is_session_wall_clock_not_audio_length() {
    let m = synthetic_timer().to_metrics(4900);
    assert_ne!(m.total_duration_ms, 0);
    assert_ne!(m.total_duration_ms, m.audio_duration_ms);
    assert!(m.total_duration_ms >= m.release_to_inserted_ms);
}

#[test]
fn rewrite_ms_is_zero_when_no_rewrite_ran() {
    let mut timer = synthetic_timer();
    timer.rewrite_started_at = None;
    timer.rewrite_finished_at = None;
    timer.rewrite_applied = false;
    let m = timer.to_metrics(4900);
    assert_eq!(m.rewrite_ms, 0);
    assert!(!m.rewrite_applied);
    assert_eq!(m.formatting_ms, 5, "formatting is still attributed");
}

#[test]
fn rtf_uses_measured_segment_compute_time() {
    let mut timer = synthetic_timer();
    let t0 = timer.hotkey_pressed_at.unwrap();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    timer.segments = vec![
        SegmentTimer {
            sequence_id: 0,
            submitted_at: at(2000),
            completed_at: Some(at(2280)),
            audio_ms: 2000,
        },
        SegmentTimer {
            sequence_id: 1,
            submitted_at: at(5020),
            completed_at: Some(at(5220)),
            audio_ms: 2900,
        },
    ];
    let m = timer.to_metrics(4900);
    assert_eq!(m.segments.len(), 2);
    assert_eq!(m.segments[0].compute_ms, Some(280));
    assert_eq!(m.segments[1].compute_ms, Some(200));
    let expected = 480.0 / 4900.0;
    assert!(
        (m.rtf - expected).abs() < 1e-4,
        "rtf {} != {expected}",
        m.rtf
    );
}

#[test]
fn rolling_percentiles_track_the_distribution() {
    let mut history = LatencyHistory::new(4);
    for (ms, applied) in [(100u64, true), (200, true), (300, false), (400, true)] {
        history.push(&LatencyMetrics {
            release_to_inserted_ms: ms,
            rewrite_applied: applied,
            rtf: 0.25,
            ..LatencyMetrics::default()
        });
    }
    let pct = history.percentiles();
    assert_eq!(pct.samples, 4);
    assert_eq!(pct.release_to_inserted_p50_ms, 200);
    assert_eq!(pct.release_to_inserted_p95_ms, 400);
    assert!((pct.llm_applied_rate - 0.75).abs() < 1e-6);
    assert!((pct.rtf_p50 - 0.25).abs() < 1e-6);

    // The window is bounded: a fifth entry evicts the oldest.
    history.push(&LatencyMetrics {
        release_to_inserted_ms: 500,
        ..LatencyMetrics::default()
    });
    assert_eq!(history.len(), 4);
    assert_eq!(history.percentiles().release_to_inserted_p50_ms, 300);
}

/// Task 2: the tail of the utterance must reach the engine *before*
/// `stop_stream()` resets the stream buffer. An 80 ms sleep is not
/// synchronization.
#[tokio::test]
async fn tail_audio_reaches_the_engine_before_stop_stream() {
    let (ctx, dir) = temp_ctx("tail");
    deterministic_no_llm(&ctx);
    let pushed = Arc::new(AtomicUsize::new(0));
    let at_stop = Arc::new(AtomicUsize::new(0));
    ctx.asr_handle
        .swap_engine(Box::new(AudioProbe::new(
            Arc::clone(&pushed),
            Arc::clone(&at_stop),
        )))
        .await
        .expect("swap");

    session::start_external(&ctx, Some("en".into()))
        .await
        .expect("start");

    // 0.5 s crosses the batching threshold; the 0.3 s tail does not, so it is
    // only delivered by the post-loop drain.
    session::push_f32(&ctx, &vec![0.2; 8_000]).expect("push body");
    session::push_f32(&ctx, &vec![0.2; 4_800]).expect("push tail");

    session::stop(&ctx, false).await.expect("stop");

    assert_eq!(
        at_stop.load(Ordering::SeqCst),
        12_800,
        "stop_stream() ran before the whole utterance was delivered"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn completed_session_reports_a_full_waterfall() {
    let (ctx, dir) = temp_ctx("waterfall");
    deterministic_no_llm(&ctx);
    ctx.asr_handle
        .swap_engine(Box::new(AudioProbe::new(
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
        )))
        .await
        .expect("swap");

    session::start_external(&ctx, Some("en".into()))
        .await
        .expect("start");
    session::push_f32(&ctx, &vec![0.2; 16_000]).expect("push");
    tokio::time::sleep(Duration::from_millis(5)).await;
    session::stop(&ctx, false).await.expect("stop");

    let m = ctx.last_latency_metrics.read().clone();
    assert_eq!(m.audio_duration_ms, 1_000);
    assert!(m.total_duration_ms > 0, "total_duration_ms was hardcoded 0");
    assert_eq!(m.segments.len(), 1, "whole-utterance ASR is one segment");
    assert!(m.segments[0].completed_at_ms.is_some());
    assert!(m.segments[0].compute_ms.is_some());
    assert_eq!(m.segments[0].audio_ms, 1_000);
    assert!(m.release_to_inserted_ms <= m.total_duration_ms);
    assert_eq!(ctx.latency_history.read().percentiles().samples, 1);

    let _ = std::fs::remove_dir_all(dir);
}

/// `first_partial_at` was declared and never written. It must now be assigned
/// when a partial is actually emitted, and the event must carry the text.
#[tokio::test]
async fn first_partial_is_timestamped_and_emitted() {
    let (ctx, dir) = temp_ctx("partial");
    deterministic_no_llm(&ctx);
    ctx.asr_handle
        .swap_engine(Box::new(
            AudioProbe::new(Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)))
                .with_partial("hello there"),
        ))
        .await
        .expect("swap");

    let mut events = ctx.bus.subscribe();
    session::start_external(&ctx, Some("en".into()))
        .await
        .expect("start");
    // Two pushes: the first only sets `first_audio_at` (it is under the
    // batching threshold), the second crosses it and produces the partial.
    // Without the gap both timestamps land in the same millisecond and the
    // metric is legitimately 0, which would not prove it is wired.
    session::push_f32(&ctx, &vec![0.2; 4_000]).expect("push head");
    tokio::time::sleep(Duration::from_millis(20)).await;
    session::push_f32(&ctx, &vec![0.2; 4_000]).expect("push rest");

    for _ in 0..200 {
        if ctx.latency_timer.read().first_partial_at.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        ctx.latency_timer.read().first_partial_at.is_some(),
        "first_partial_at must be assigned when a partial arrives"
    );

    let mut saw_text = false;
    while let Ok(event) = events.try_recv() {
        if let DoryEvent::Partial(payload) = event {
            if payload.full_text.to_lowercase() == "hello there" {
                saw_text = true;
            }
        }
    }
    assert!(saw_text, "transcript:partial must carry the partial text");

    session::stop(&ctx, false).await.expect("stop");
    assert!(
        ctx.last_latency_metrics.read().audio_to_first_partial_ms > 0,
        "audio_to_first_partial_ms was structurally zero before this task"
    );

    let _ = std::fs::remove_dir_all(dir);
}
