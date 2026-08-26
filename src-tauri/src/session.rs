use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use uuid::Uuid;

use crate::audio::AudioResampler;
use crate::context::AppContext;
use crate::dory::{CaptureKind, DoryEvent, Stage};
use crate::formatting::{
    assemble_asr_vocabulary, format_transcript_ex, CleanupLevel, CustomReplacements, FormatRequest,
    TextCleaner, VoiceStyle,
};
use crate::history::HistoryEntry;
use crate::injection::TextInjector;
use crate::rewrite::{polish_or_fallback, FlowClient, RewriteRequest};
use crate::settings::AppSettings;
use crate::state::{AppStateEnum, InjectionFeedback, StreamingTranscriptPayload};

pub fn session_vocabulary(settings: &AppSettings) -> Vec<String> {
    let terms: Vec<(String, String)> = settings
        .dictionary_terms
        .iter()
        .map(|t| (t.term.clone(), t.preferred_spelling.clone()))
        .collect();
    let afters: Vec<String> = settings
        .custom_replacements
        .iter()
        .filter(|r| r.enabled)
        .map(|r| r.after.clone())
        .collect();
    assemble_asr_vocabulary(&terms, &afters, 60)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostprocessOutcome {
    pub smart: String,
    pub final_text: String,
    pub rewriter_used: bool,
    /// LLM rewrite error, if any. `Some` when the rewriter was attempted
    /// but the request failed or the safety gate rejected the candidate.
    /// Callers surface this to the user via `InjectionFeedback::tier_status`.
    pub rewriter_error: Option<String>,
}

/// Shared stop-path used by the live session. Tests call this directly.
pub fn postprocess_transcript(
    raw: &str,
    settings: &AppSettings,
    focused_process: &str,
    client: &FlowClient,
) -> PostprocessOutcome {
    let intent = settings.resolve_intent();
    let replacement_rules = CustomReplacements::new(settings.custom_replacements.clone());
    let glossary: Vec<(String, String)> = settings
        .dictionary_terms
        .iter()
        .map(|t| (t.term.clone(), t.preferred_spelling.clone()))
        .collect();
    let cleanup_level = intent.cleanup_level.clone();
    let level = CleanupLevel::parse(&cleanup_level);
    let style = if settings.auto_style_from_app {
        style_from_process(focused_process, &settings.style)
    } else {
        VoiceStyle::parse(&settings.style)
    };

    let mut smart = format_transcript_ex(
        raw,
        FormatRequest {
            cleanup_level: level,
            dictation_mode: &settings.dictation_mode,
            style,
            filler_removal_enabled: settings.filler_removal_enabled,
            spoken_punctuation_enabled: settings.spoken_punctuation_enabled,
            custom_replacements: &replacement_rules,
            focused_process: Some(focused_process),
        },
    );

    if level != CleanupLevel::Raw {
        smart = TextCleaner::apply_glossary(&smart, &glossary);
    }

    let wants_flow = !intent.tier.eq_ignore_ascii_case("raw_verbatim")
        && !intent.flow_model.eq_ignore_ascii_case("none")
        && matches!(level, CleanupLevel::Medium | CleanupLevel::High)
        && !settings.dictation_mode.eq_ignore_ascii_case("coding")
        && !smart.is_empty();

    if !wants_flow {
        return PostprocessOutcome {
            smart: smart.clone(),
            final_text: smart,
            rewriter_used: false,
            rewriter_error: None,
        };
    }

    let req = RewriteRequest {
        text: smart.clone(),
        cleanup_level,
        style: settings.style.clone(),
        dictation_mode: settings.dictation_mode.clone(),
        vocabulary: session_vocabulary(settings),
        app_process: focused_process.to_string(),
        model_id: intent.flow_model.clone(),
    };
    let outcome = polish_or_fallback(client, &smart, &req);
    let polished = outcome.final_text;
    let used = outcome.used;
    let rewriter_error = outcome.error;
    let final_text = TextCleaner::apply_glossary(&polished, &glossary);
    PostprocessOutcome {
        smart,
        final_text,
        rewriter_used: used,
        rewriter_error,
    }
}

#[derive(Debug, Clone)]
pub struct StopOutcome {
    pub raw: String,
    pub final_text: String,
    pub language: String,
    pub injected: bool,
}

#[derive(Debug, Clone)]
pub struct SessionError {
    pub code: String,
    pub message: String,
}

impl SessionError {
    pub fn busy() -> Self {
        Self {
            code: "session_busy".into(),
            message: "A dictation session is already running".into(),
        }
    }

    fn other(message: impl Into<String>) -> Self {
        Self {
            code: "error".into(),
            message: message.into(),
        }
    }
}

impl From<SessionError> for String {
    fn from(value: SessionError) -> Self {
        value.message
    }
}

pub async fn start_microphone(ctx: &AppContext) -> Result<(), SessionError> {
    let state = *ctx.state_enum.read();
    if state == AppStateEnum::Recording {
        return Ok(());
    }
    if !matches!(state, AppStateEnum::Ready | AppStateEnum::Idle) {
        return Err(SessionError::busy());
    }

    let engine = ctx.asr_engine.write().engine_status();
    if engine.is_loading || !engine.loaded {
        return Err(SessionError::other(
            "Model is still loading. Wait until Settings shows CUDA/CPU ready.",
        ));
    }

    begin_session(ctx, CaptureKind::Microphone)?;

    let settings = ctx.settings_store.get();
    let language = if settings.auto_detect_language {
        "auto".into()
    } else {
        settings.language.clone()
    };
    let vocabulary = session_vocabulary(&settings);
    if let Err(err) = ctx.asr_engine.write().start_stream(&language, &vocabulary) {
        reset_ready(ctx);
        return Err(SessionError::other(err));
    }
    ctx.bus.emit(DoryEvent::Stage(Stage::Asr));

    let (tx_samples, rx_samples) = mpsc::unbounded_channel::<Vec<f32>>();
    let (tx_auto_stop, mut rx_auto_stop) = mpsc::channel::<()>(1);
    *ctx.recording_sample_sender.write() = Some(tx_samples.clone());
    ctx.recording_pcm.lock().clear();

    if let Err(err) = ctx.audio_engine.write().start_capture(
        settings.microphone_device_id.clone(),
        settings.input_gain,
        settings.vad_sensitivity,
        // In push-to-talk the key release is the stop signal; silence
        // auto-stop would cut users off mid-pause. Keep it for toggle mode.
        if settings.push_to_talk {
            0
        } else {
            settings.auto_stop_silence_ms
        },
        tx_samples,
        tx_auto_stop,
    ) {
        let _ = ctx.asr_engine.write().cancel_stream();
        reset_ready(ctx);
        return Err(SessionError::other(err));
    }
    ctx.bus.emit(DoryEvent::Stage(Stage::Capture));

    spawn_sample_loop(ctx, rx_samples);

    let ctx_stop = ctx.clone();
    tokio::spawn(async move {
        if rx_auto_stop.recv().await.is_some() {
            if *ctx_stop.state_enum.read() == AppStateEnum::Recording {
                ctx_stop.bus.emit(DoryEvent::AutoStop);
                let _ = stop(&ctx_stop, true, None).await;
            }
        }
    });

    spawn_max_duration_watch(ctx, settings.max_duration_sec);
    Ok(())
}

pub async fn start_external(ctx: &AppContext, language: Option<String>) -> Result<(), SessionError> {
    let state = *ctx.state_enum.read();
    if !matches!(state, AppStateEnum::Ready | AppStateEnum::Idle) {
        return Err(SessionError::busy());
    }

    begin_session(ctx, CaptureKind::External)?;

    let settings = ctx.settings_store.get();
    let language = language.unwrap_or_else(|| {
        if settings.auto_detect_language {
            "auto".into()
        } else {
            settings.language.clone()
        }
    });
    let vocabulary = session_vocabulary(&settings);
    if let Err(err) = ctx.asr_engine.write().start_stream(&language, &vocabulary) {
        reset_ready(ctx);
        return Err(SessionError::other(err));
    }
    ctx.bus.emit(DoryEvent::Stage(Stage::Asr));

    let (tx_samples, rx_samples) = mpsc::unbounded_channel::<Vec<f32>>();
    *ctx.recording_sample_sender.write() = Some(tx_samples);
    ctx.recording_pcm.lock().clear();
    spawn_sample_loop(ctx, rx_samples);
    spawn_max_duration_watch(ctx, settings.max_duration_sec);
    Ok(())
}

pub fn push_pcm_s16le(ctx: &AppContext, bytes: &[u8], sample_rate: u32) -> Result<(), SessionError> {
    if *ctx.state_enum.read() != AppStateEnum::Recording {
        return Err(SessionError::other("Not recording"));
    }
    let mut samples = AudioResampler::pcm16_bytes_to_f32(bytes);
    if sample_rate != 0 && sample_rate != 16000 {
        let mut resampler = AudioResampler::new(sample_rate, 1);
        samples = resampler.resample_f32(&samples);
    }
    push_f32(ctx, &samples)
}

pub fn push_f32(ctx: &AppContext, samples: &[f32]) -> Result<(), SessionError> {
    if samples.is_empty() {
        return Ok(());
    }
    ctx.bus.emit(DoryEvent::Stage(Stage::Resample));
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    let level = (rms * 4.0).min(1.0);
    *ctx.last_audio_level.write() = level;
    ctx.bus.emit(DoryEvent::AudioLevel(level));

    let Some(tx) = ctx.recording_sample_sender.read().clone() else {
        return Err(SessionError::other("No active audio channel"));
    };
    let _ = tx.send(samples.to_vec());
    Ok(())
}

pub async fn stop(
    ctx: &AppContext,
    inject: bool,
    app: Option<&tauri::AppHandle>,
) -> Result<StopOutcome, SessionError> {
    if *ctx.state_enum.read() != AppStateEnum::Recording {
        return Ok(StopOutcome {
            raw: String::new(),
            final_text: String::new(),
            language: ctx.asr_engine.read().get_detected_language(),
            injected: false,
        });
    }

    ctx.latency_timer.write().speech_ended_at = Some(Instant::now());
    ctx.audio_engine.write().stop_capture();
    *ctx.recording_sample_sender.write() = None;
    let capture_kind = *ctx.capture_kind.read();
    *ctx.capture_kind.write() = CaptureKind::None;

    *ctx.state_enum.write() = AppStateEnum::Processing;
    ctx.bus.emit(DoryEvent::State(AppStateEnum::Processing));

    // Let the sample loop drain remaining chunks into recording_pcm.
    tokio::time::sleep(Duration::from_millis(80)).await;

    let pcm = std::mem::take(&mut *ctx.recording_pcm.lock());
    let n = pcm.len().max(1);
    let rms = (pcm.iter().map(|s| s * s).sum::<f32>() / n as f32).sqrt();
    let peak = pcm.iter().copied().map(f32::abs).fold(0.0f32, f32::max);
    let capture_device = ctx.audio_engine.read().last_device_name();
    log::info!(
        "Captured {:.2}s of audio (rms={:.4}, peak={:.3}, {} samples) from '{capture_device}'",
        pcm.len() as f32 / 16000.0,
        rms,
        peak,
        pcm.len()
    );

    let silent_mic = peak < 5e-3;
    if silent_mic {
        log::warn!(
            "Microphone produced near-silence. Device='{capture_device}' peak={peak:.4} rms={rms:.4}. \
             Windows default is often Bluetooth Hands-Free, Communications, or Stereo Mix — pick a real mic in Settings → Audio."
        );
        let _ = ctx.asr_engine.write().cancel_stream();
    }
    // Audio was already streamed to the sidecar during recording via
    // spawn_sample_loop; no redundant bulk push needed at stop time.

    let raw_transcript = if silent_mic {
        String::new()
    } else {
        ctx.asr_engine
            .write()
            .stop_stream()
            .map_err(SessionError::other)?
    };
    ctx.latency_timer.write().final_asr_at = Some(Instant::now());
    log::info!(
        "Dictation finished: {} chars of transcript, held {:.1}s",
        raw_transcript.chars().count(),
        ctx.latency_timer
            .read()
            .recording_started_at
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0)
    );

    let settings = ctx.settings_store.get();
    let intent = settings.resolve_intent();
    ctx.bus.emit(DoryEvent::Stage(Stage::Format));
    let cleanup_level = intent.cleanup_level.clone();
    let focused_process = if capture_kind == CaptureKind::External {
        "companion".into()
    } else {
        crate::platform::active_window().1
    };
    let wants_flow = !intent.tier.eq_ignore_ascii_case("raw_verbatim")
        && !intent.flow_model.eq_ignore_ascii_case("none")
        && matches!(
            CleanupLevel::parse(&cleanup_level),
            CleanupLevel::Medium | CleanupLevel::High
        )
        && !settings.dictation_mode.eq_ignore_ascii_case("coding")
        && !raw_transcript.is_empty();
    if wants_flow {
        ctx.bus.emit(DoryEvent::Partial(StreamingTranscriptPayload {
            committed_prefix: String::new(),
            mutable_suffix: String::new(),
            full_text: String::new(),
            language: ctx.asr_engine.read().get_detected_language(),
            audio_level: 0.0,
            stage: "polishing".into(),
        }));
        let flow_model = intent.flow_model.clone();
        let compute_backend = settings.compute_backend.clone();
        let override_layers = if settings.flow_n_gpu_layers < 0 {
            None
        } else {
            Some(settings.flow_n_gpu_layers.max(0) as u32)
        };
        let runtime = Arc::clone(&ctx.flow_runtime);
        let ensure_result = tokio::task::block_in_place(|| {
            runtime.ensure(&flow_model, &compute_backend, override_layers)
        });
        if let (Some(app_handle), Err(err)) = (app, &ensure_result) {
            crate::rewrite::auto_install_if_missing(app_handle, ctx, &compute_backend, err);
        }
    }
    let client = ctx.flow_runtime.client.read().clone();
    let processed = postprocess_transcript(
        &raw_transcript,
        &settings,
        &focused_process,
        &client,
    );
    let smart_transcript = processed.smart;
    let final_transcript = processed.final_text;
    let rewriter_used = processed.rewriter_used;
    let rewriter_error = processed.rewriter_error;
    ctx.latency_timer.write().rewrite_finished_at = Some(Instant::now());

    let mut injected = false;
    let (mut app_title, mut process_name) = ("Android".to_string(), "companion".to_string());

    if inject && !silent_mic && !final_transcript.is_empty() {
        *ctx.state_enum.write() = AppStateEnum::Injecting;
        ctx.bus.emit(DoryEvent::State(AppStateEnum::Injecting));
        ctx.bus.emit(DoryEvent::Stage(Stage::Inject));
        if capture_kind == CaptureKind::Microphone {
            let hwnd = *ctx.dictation_target_hwnd.read();
            crate::platform::focus_hwnd(hwnd);
        }
        let outcome = TextInjector::inject(
            &final_transcript,
            settings.clipboard_restore_enabled,
            *ctx.dictation_target_hwnd.read(),
        )
        .map_err(SessionError::other)?;
        ctx.latency_timer.write().injection_finished_at = Some(Instant::now());
        injected = outcome.pasted;
        app_title = outcome.app_title.clone();
        process_name = outcome.process_name.clone();
        let tier_status = rewriter_error.as_ref().map(|err| {
            format!("rewriter_error: {err}")
        });
        let feedback = InjectionFeedback {
            pasted: outcome.pasted,
            fallback_copy: outcome.fallback_copy,
            paste_chord: outcome.paste_chord.clone(),
            process_name: outcome.process_name.clone(),
            message: if silent_mic {
                if capture_device.is_empty() {
                    "No mic signal".into()
                } else {
                    format!("No mic signal · {capture_device}")
                }
            } else if final_transcript.is_empty() {
                "No speech".into()
            } else if outcome.fallback_copy {
                format!("Copied — press {}", outcome.paste_chord)
            } else if let Some(err) = rewriter_error.as_deref() {
                format!("Inserted (LLM error: {err})")
            } else if wants_flow && !rewriter_used {
                "Inserted (unpolished)".into()
            } else {
                "Inserted".into()
            },
            tier_status,
        };
        ctx.bus.emit(DoryEvent::Injection(feedback));
    } else {
        ctx.latency_timer.write().injection_finished_at = Some(Instant::now());
        if inject {
            let tier_status = rewriter_error.as_ref().map(|err| {
                format!("rewriter_error: {err}")
            });
            let message = if silent_mic {
                if capture_device.is_empty() {
                    "No mic signal".into()
                } else {
                    format!("No mic signal · {capture_device}")
                }
            } else if final_transcript.is_empty() {
                "No speech".into()
            } else if let Some(err) = rewriter_error.as_deref() {
                format!("Inserted (LLM error: {err})")
            } else if wants_flow && !rewriter_used {
                "Inserted (unpolished)".into()
            } else {
                "Inserted".into()
            };
            ctx.bus.emit(DoryEvent::Injection(InjectionFeedback {
                pasted: false,
                fallback_copy: false,
                paste_chord: String::new(),
                process_name: process_name.clone(),
                message,
                tier_status,
            }));
        }
    }

    let metrics = ctx.latency_timer.read().to_metrics(0);
    *ctx.last_latency_metrics.write() = metrics;

    if settings.history_retention != "disabled" && !final_transcript.is_empty() {
        ctx.bus.emit(DoryEvent::Stage(Stage::History));
        let session_duration_ms = ctx
            .latency_timer
            .read()
            .recording_started_at
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0);
        let entry = HistoryEntry {
            id: Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            duration_ms: session_duration_ms,
            language: ctx.asr_engine.read().get_detected_language(),
            raw_transcript: raw_transcript.clone(),
            final_transcript: final_transcript.clone(),
            application_name: app_title,
            application_process: process_name,
            word_count: final_transcript.split_whitespace().count(),
            character_count: final_transcript.chars().count(),
            model_version: crate::model::spec_for(&settings.asr_model)
                .label
                .to_string(),
            processing_mode: cleanup_level.clone(),
            smart_transcript: smart_transcript.clone(),
            rewriter_used,
        };
        let _ = ctx.history_store.insert_entry(&entry);
    }

    let language = ctx.asr_engine.read().get_detected_language();
    let payload = StreamingTranscriptPayload {
        committed_prefix: final_transcript.clone(),
        mutable_suffix: String::new(),
        full_text: final_transcript.clone(),
        language: language.clone(),
        audio_level: 0.0,
        stage: String::new(),
    };
    ctx.bus.emit(DoryEvent::Final(payload));
    reset_ready(ctx);

    Ok(StopOutcome {
        raw: raw_transcript,
        final_text: final_transcript,
        language,
        injected,
    })
}

pub fn cancel(ctx: &AppContext) -> Result<(), SessionError> {
    ctx.audio_engine.write().stop_capture();
    *ctx.recording_sample_sender.write() = None;
    *ctx.capture_kind.write() = CaptureKind::None;
    ctx.recording_pcm.lock().clear();
    *ctx.dictation_target_hwnd.write() = 0;
    ctx.asr_engine
        .write()
        .cancel_stream()
        .map_err(SessionError::other)?;
    reset_ready(ctx);
    Ok(())
}

fn style_from_process(process: &str, fallback: &str) -> VoiceStyle {
    let p = process.to_lowercase();
    if p.contains("discord") || p.contains("slack") || p.contains("telegram") || p.contains("whatsapp")
    {
        return VoiceStyle::Chat;
    }
    if p.contains("outlook") || p.contains("mail") || p.contains("thunderbird") {
        return VoiceStyle::Email;
    }
    if p.contains("code") || p.contains("devenv") || p.contains("idea64") {
        return VoiceStyle::parse(fallback);
    }
    VoiceStyle::parse(fallback)
}

fn begin_session(ctx: &AppContext, kind: CaptureKind) -> Result<(), SessionError> {
    let mut timer = ctx.latency_timer.write();
    timer.reset();
    timer.hotkey_pressed_at = Some(Instant::now());
    timer.recording_started_at = Some(Instant::now());
    drop(timer);

    *ctx.dictation_target_hwnd.write() = crate::platform::foreground_hwnd();
    ctx.recording_pcm.lock().clear();
    *ctx.state_enum.write() = AppStateEnum::Recording;
    *ctx.capture_kind.write() = kind;
    *ctx.last_audio_level.write() = 0.0;
    ctx.bus.emit(DoryEvent::State(AppStateEnum::Recording));
    log::info!("Dictation session started ({:?})", kind);
    Ok(())
}

fn reset_ready(ctx: &AppContext) {
    crate::hotkey::hook::reset_mode();
    *ctx.state_enum.write() = AppStateEnum::Ready;
    *ctx.capture_kind.write() = CaptureKind::None;
    *ctx.last_audio_level.write() = 0.0;
    ctx.bus.emit(DoryEvent::State(AppStateEnum::Ready));
}

fn spawn_sample_loop(
    ctx: &AppContext,
    mut rx_samples: mpsc::UnboundedReceiver<Vec<f32>>,
) {
    let state_ref = Arc::clone(&ctx.state_enum);
    let latency_ref = Arc::clone(&ctx.latency_timer);
    let bus = ctx.bus.clone();
    let level_ref = Arc::clone(&ctx.last_audio_level);
    let pcm_ref = Arc::clone(&ctx.recording_pcm);
    let asr_ref = Arc::clone(&ctx.asr_engine);

    tokio::spawn(async move {
        let mut first_audio = true;
        let mut last_level_emit = Instant::now();
        // Accumulate samples before pushing to the sidecar so we don't
        // send hundreds of tiny JSON messages per second.  ~0.5 s at
        // 16 kHz = 8 000 samples is a good trade-off between latency
        // and IPC overhead.
        let mut pending_for_asr: Vec<f32> = Vec::with_capacity(16000);
        const ASR_PUSH_THRESHOLD: usize = 8000; // ~0.5 s at 16 kHz

        loop {
            if *state_ref.read() != AppStateEnum::Recording {
                break;
            }

            let Some(chunk) = rx_samples.recv().await else {
                break;
            };

            if first_audio {
                latency_ref.write().first_audio_at = Some(Instant::now());
                first_audio = false;
            }

            let rms = if chunk.is_empty() {
                0.0
            } else {
                (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt()
            };
            let level = (rms * 6.0).min(1.0);
            *level_ref.write() = level;
            pcm_ref.lock().extend_from_slice(&chunk);
            pending_for_asr.extend_from_slice(&chunk);

            // Push to sidecar in ~0.5 s batches.
            if pending_for_asr.len() >= ASR_PUSH_THRESHOLD {
                let _ = asr_ref.write().push_audio(&pending_for_asr);
                pending_for_asr.clear();
            }

            if last_level_emit.elapsed() >= Duration::from_millis(40) {
                bus.emit(DoryEvent::AudioLevel(level));
                last_level_emit = Instant::now();
            }
        }

        // Drain chunks that raced with stop_capture, including the buffered
        // tail when the capture sender closes while the loop awaits recv().
        while let Ok(chunk) = rx_samples.try_recv() {
            pcm_ref.lock().extend_from_slice(&chunk);
            pending_for_asr.extend_from_slice(&chunk);
        }
        if !pending_for_asr.is_empty() {
            let _ = asr_ref.write().push_audio(&pending_for_asr);
        }
    });
}

fn spawn_max_duration_watch(ctx: &AppContext, max_duration_sec: u64) {
    if max_duration_sec == 0 {
        return;
    }
    let ctx = ctx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(max_duration_sec)).await;
        if *ctx.state_enum.read() == AppStateEnum::Recording {
            log::info!("Max recording duration reached; stopping.");
            let inject_on_stop = *ctx.capture_kind.read() == CaptureKind::Microphone;
            let _ = stop(&ctx, inject_on_stop, None).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asr::ASREngine;
    use crate::context::AppContext;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct PushProbe {
        pushed_samples: Arc<AtomicUsize>,
    }

    impl ASREngine for PushProbe {
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
            self.pushed_samples
                .fetch_add(samples_16k_mono.len(), Ordering::SeqCst);
            Ok(None)
        }

        fn get_partial_transcript(&mut self) -> Result<String, String> {
            Ok(String::new())
        }

        fn stop_stream(&mut self) -> Result<String, String> {
            Ok(String::new())
        }

        fn cancel_stream(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn get_detected_language(&self) -> String {
            "en".into()
        }

        fn get_backend_name(&self) -> String {
            "push-probe".into()
        }
    }

    #[tokio::test]
    async fn sample_loop_flushes_pending_audio_when_channel_closes() {
        let dir = std::env::temp_dir().join(format!("reflow_stream_{}", uuid::Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        let pushed_samples = Arc::new(AtomicUsize::new(0));
        *ctx.asr_engine.write() = Box::new(PushProbe {
            pushed_samples: Arc::clone(&pushed_samples),
        });
        *ctx.state_enum.write() = AppStateEnum::Recording;

        let (tx_samples, rx_samples) = mpsc::unbounded_channel();
        spawn_sample_loop(&ctx, rx_samples);
        tx_samples.send(vec![0.1; 100]).unwrap();
        drop(tx_samples);

        for _ in 0..20 {
            if pushed_samples.load(Ordering::SeqCst) == 100 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert_eq!(pushed_samples.load(Ordering::SeqCst), 100);
        *ctx.state_enum.write() = AppStateEnum::Ready;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn external_session_formats_and_records_history() {
        let dir = std::env::temp_dir().join(format!("reflow_sess_{}", uuid::Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        start_external(&ctx, Some("en".into())).await.expect("start");
        push_f32(&ctx, &vec![0.05; 6400]).expect("push");
        let outcome = stop(&ctx, false, None).await.expect("stop");
        assert!(!outcome.final_text.is_empty());
        assert!(!outcome.injected);
        let entries = ctx.history_store.get_entries(10, 0).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(*ctx.state_enum.read(), AppStateEnum::Ready);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_vocabulary_includes_dictionary_terms() {
        let settings = crate::settings::AppSettings::default();
        let vocab = session_vocabulary(&settings);
        let joined = vocab.join(" ").to_lowercase();
        assert!(joined.contains("qwen"), "{vocab:?}");
        assert!(joined.contains("tauri"), "{vocab:?}");
        assert!(joined.contains("supabase"), "{vocab:?}");
    }

    #[test]
    fn raw_mode_does_not_apply_glossary() {
        let mut settings = crate::settings::AppSettings::default();
        settings.cleanup_level = "raw".into();
        settings.processing_mode = "raw".into();
        settings.dictionary_terms = vec![crate::settings::DictionaryTerm {
            id: "1".into(),
            term: "tauri".into(),
            preferred_spelling: "Tauri".into(),
            category: "x".into(),
        }];
        let out = postprocess_transcript(
            "ship this tauri app",
            &settings,
            "notepad",
            &FlowClient::new_missing(),
        );
        assert_eq!(out.final_text, "ship this tauri app");
        assert_eq!(out.smart, "ship this tauri app");
        assert!(!out.rewriter_used);
    }

    #[test]
    fn light_mode_applies_glossary() {
        let mut settings = crate::settings::AppSettings::default();
        settings.cleanup_level = "light".into();
        settings.processing_mode = "smart".into();
        settings.dictionary_terms = vec![crate::settings::DictionaryTerm {
            id: "1".into(),
            term: "tauri".into(),
            preferred_spelling: "Tauri".into(),
            category: "x".into(),
        }];
        let out = postprocess_transcript(
            "ship this tauri app",
            &settings,
            "notepad",
            &FlowClient::new_missing(),
        );
        assert!(
            out.final_text.contains("Tauri"),
            "light should glossary: {}",
            out.final_text
        );
        assert!(!out.final_text.contains("tauri"));
        assert!(!out.rewriter_used);
    }

    #[tokio::test]
    async fn second_external_start_is_busy() {
        let dir = std::env::temp_dir().join(format!("reflow_busy_{}", uuid::Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        start_external(&ctx, None).await.unwrap();
        let err = start_external(&ctx, None).await.unwrap_err();
        assert_eq!(err.code, "session_busy");
        cancel(&ctx).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
