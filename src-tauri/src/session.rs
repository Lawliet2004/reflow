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

/// Bounded capacity for audio streaming channel to prevent unbounded memory growth.
pub const SAMPLE_CHANNEL_CAPACITY: usize = 256;

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
    /// Wall-clock duration of the deterministic Stage 1 pipeline.
    pub formatting_ms: u64,
    /// Wall-clock duration of the Stage 2 LLM call. `None` when no rewrite
    /// was dispatched, which is what keeps `rewrite_ms` from absorbing
    /// formatting time.
    pub rewrite_ms: Option<u64>,
}

/// Shared stop-path used by the live session. Tests call this directly.
pub fn postprocess_transcript(
    raw: &str,
    settings: &AppSettings,
    focused_process: &str,
    client: &FlowClient,
) -> PostprocessOutcome {
    let formatting_started = Instant::now();
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

    // `intent.run_llm` is the single authority. Deriving this from
    // `cleanup_level` is what made the default install advertise the
    // smart_flow tier while never running Stage 2.
    let wants_flow = intent.run_llm
        && !settings.dictation_mode.eq_ignore_ascii_case("coding")
        && !smart.is_empty();

    let formatting_ms = formatting_started.elapsed().as_millis() as u64;

    if !wants_flow {
        return PostprocessOutcome {
            smart: smart.clone(),
            final_text: smart,
            rewriter_used: false,
            rewriter_error: None,
            formatting_ms,
            rewrite_ms: None,
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
    let rewrite_started = Instant::now();
    let outcome = polish_or_fallback(client, &smart, &req);
    let rewrite_ms = rewrite_started.elapsed().as_millis() as u64;
    let polished = outcome.final_text;
    let used = outcome.used;
    let rewriter_error = outcome.error;
    // Mirror the pre-rewrite guard: raw means verbatim, so the glossary must
    // not be applied here either. Keeping the check local means the invariant
    // does not silently depend on how `run_llm` is defined elsewhere.
    let final_text = if level == CleanupLevel::Raw {
        polished
    } else {
        TextCleaner::apply_glossary(&polished, &glossary)
    };
    PostprocessOutcome {
        smart,
        final_text,
        rewriter_used: used,
        rewriter_error,
        formatting_ms,
        rewrite_ms: Some(rewrite_ms),
    }
}

/// Everything the user-facing one-liner depends on.
///
/// Extracted because the injecting and non-injecting paths were maintaining two
/// near-identical copies of this cascade, and they had already drifted: only one
/// of them mentioned the clipboard fallback.
pub struct InjectionMessageInput<'a> {
    pub silent_mic: bool,
    pub capture_device: &'a str,
    pub transcript_empty: bool,
    pub fallback_copy: bool,
    pub paste_chord: &'a str,
    pub rewriter_error: Option<&'a str>,
    pub wants_flow: bool,
    pub rewriter_used: bool,
    /// Non-fatal ASR notice, e.g. that the dictation was longer than the
    /// single-pass limit. Appended so an incomplete transcript is never
    /// presented as a complete one.
    pub asr_warning: Option<&'a str>,
}

/// Build the single line the user sees after a dictation.
pub fn injection_message(input: &InjectionMessageInput<'_>) -> String {
    let base = if input.silent_mic {
        if input.capture_device.is_empty() {
            "No mic signal".to_string()
        } else {
            format!("No mic signal · {}", input.capture_device)
        }
    } else if input.transcript_empty {
        "No speech".to_string()
    } else if input.fallback_copy {
        format!("Copied — press {}", input.paste_chord)
    } else if let Some(err) = input.rewriter_error {
        format!("Inserted (LLM error: {err})")
    } else if input.wants_flow && !input.rewriter_used {
        "Inserted (unpolished)".to_string()
    } else {
        "Inserted".to_string()
    };

    match input.asr_warning {
        Some(warning) if !input.silent_mic && !input.transcript_empty => {
            format!("{base} · {warning}")
        }
        _ => base,
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
    start_microphone_at(ctx, None).await
}

/// `pressed_at` is the `Instant` captured inside the OS hotkey callback.
/// Passing it in is what makes `hotkey_to_recording_ms` cover thread spawn,
/// `SendInput`, engine IPC and tokio scheduling instead of reporting zero.
pub async fn start_microphone_at(
    ctx: &AppContext,
    pressed_at: Option<Instant>,
) -> Result<(), SessionError> {
    let _operation = ctx.session_operation.lock().await;
    let state = *ctx.state_enum.read();
    if state == AppStateEnum::Recording {
        return Ok(());
    }
    if !matches!(state, AppStateEnum::Ready | AppStateEnum::Idle) {
        return Err(SessionError::busy());
    }

    // Loading completes asynchronously inside the Python sidecar. Refresh the
    // actor cache at the point of use so a stale UI/status snapshot can never
    // block an otherwise-ready model from starting dictation.
    let engine = ctx.asr_handle.refresh_status().await.unwrap_or_else(|err| {
        log::warn!("Could not refresh ASR readiness before recording: {err}");
        ctx.asr_handle.engine_status()
    });
    if engine.is_loading {
        return Err(SessionError::other(
            "Model is still loading. Wait until Settings shows CUDA/CPU ready.",
        ));
    }
    if !engine.loaded {
        return Err(SessionError::other(match engine.error {
            Some(err) => format!("Model failed to load: {err}"),
            None => "Model is not loaded. Open Settings → Model and reload it.".into(),
        }));
    }

    let session_id = ctx.asr_handle.next_session_id();
    *ctx.current_session_id.write() = Some(session_id);

    begin_session(ctx, CaptureKind::Microphone, pressed_at)?;

    let settings = ctx.settings_store.get();
    let language = if settings.auto_detect_language {
        "auto".into()
    } else {
        settings.language.clone()
    };
    let vocabulary = session_vocabulary(&settings);
    if let Err(err) = ctx
        .asr_handle
        .start_stream(session_id, &language, &vocabulary)
        .await
    {
        reset_ready(ctx);
        return Err(SessionError::other(err));
    }
    ctx.bus.emit(DoryEvent::Stage(Stage::Asr));

    let (tx_samples, rx_samples) = mpsc::channel::<Vec<f32>>(SAMPLE_CHANNEL_CAPACITY);
    let (tx_auto_stop, mut rx_auto_stop) = mpsc::channel::<()>(1);
    *ctx.recording_sample_sender.write() = Some(tx_samples.clone());
    ctx.recording_pcm.lock().clear();

    // Start the drain loop before the capture stream so no chunk can be
    // produced before there is a consumer for it.
    spawn_sample_loop(ctx, session_id, rx_samples);

    let capture_res = ctx.audio_engine.write().start_capture(
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
    );
    if let Err(err) = capture_res {
        let _ = ctx.asr_handle.cancel_stream(session_id).await;
        reset_ready(ctx);
        return Err(SessionError::other(err));
    }
    ctx.bus.emit(DoryEvent::Stage(Stage::Capture));

    // The stream is live now — this, not `begin_session`, is when recording
    // actually starts. Setting it here is what gives `hotkey_to_recording_ms`
    // a real value.
    ctx.latency_timer.write().recording_started_at = Some(Instant::now());

    let ctx_stop = ctx.clone();
    tokio::spawn(async move {
        if rx_auto_stop.recv().await.is_some()
            && *ctx_stop.state_enum.read() == AppStateEnum::Recording
        {
            ctx_stop.bus.emit(DoryEvent::AutoStop);
            let _ = stop_owned(&ctx_stop, true, session_id).await;
        }
    });

    spawn_max_duration_watch(ctx, settings.max_duration_sec, session_id);
    Ok(())
}

pub async fn start_external(
    ctx: &AppContext,
    language: Option<String>,
) -> Result<(), SessionError> {
    start_external_owned(ctx, language).await.map(|_| ())
}

pub async fn start_external_owned(
    ctx: &AppContext,
    language: Option<String>,
) -> Result<u64, SessionError> {
    let _operation = ctx.session_operation.lock().await;
    let state = *ctx.state_enum.read();
    if !matches!(state, AppStateEnum::Ready | AppStateEnum::Idle) {
        return Err(SessionError::busy());
    }

    let session_id = ctx.asr_handle.next_session_id();
    *ctx.current_session_id.write() = Some(session_id);

    begin_session(ctx, CaptureKind::External, None)?;

    let settings = ctx.settings_store.get();
    let language = language.unwrap_or_else(|| {
        if settings.auto_detect_language {
            "auto".into()
        } else {
            settings.language.clone()
        }
    });
    let vocabulary = session_vocabulary(&settings);
    if let Err(err) = ctx
        .asr_handle
        .start_stream(session_id, &language, &vocabulary)
        .await
    {
        reset_ready(ctx);
        return Err(SessionError::other(err));
    }
    ctx.bus.emit(DoryEvent::Stage(Stage::Asr));

    let (tx_samples, rx_samples) = mpsc::channel::<Vec<f32>>(SAMPLE_CHANNEL_CAPACITY);
    *ctx.recording_sample_sender.write() = Some(tx_samples);
    ctx.recording_pcm.lock().clear();
    spawn_sample_loop(ctx, session_id, rx_samples);
    // There is no local capture device for an external session; the stream is
    // "recording" as soon as the ASR stream is open and a consumer exists.
    ctx.latency_timer.write().recording_started_at = Some(Instant::now());
    spawn_max_duration_watch(ctx, settings.max_duration_sec, session_id);
    Ok(session_id)
}

pub fn validate_sample_rate(sample_rate: u32) -> Result<(), SessionError> {
    if !(8_000..=192_000).contains(&sample_rate) {
        return Err(SessionError::other(
            "Sample rate must be between 8000 and 192000 Hz",
        ));
    }
    Ok(())
}

pub fn push_pcm_s16le(
    ctx: &AppContext,
    bytes: &[u8],
    sample_rate: u32,
) -> Result<(), SessionError> {
    if *ctx.state_enum.read() != AppStateEnum::Recording {
        return Err(SessionError::other("Not recording"));
    }
    validate_sample_rate(sample_rate)?;
    if bytes.len() % 2 != 0 {
        return Err(SessionError::other(
            "PCM audio must contain complete 16-bit samples",
        ));
    }
    let mut samples = AudioResampler::pcm16_bytes_to_f32(bytes);
    if sample_rate != 0 && sample_rate != 16000 {
        let mut resampler = AudioResampler::new(sample_rate, 1);
        samples = resampler.resample_f32(&samples);
    }
    push_f32(ctx, &samples)
}

pub fn push_f32_owned(
    ctx: &AppContext,
    session_id: u64,
    samples: &[f32],
) -> Result<(), SessionError> {
    let active = ctx.current_session_id.read();
    if *active != Some(session_id) || *ctx.state_enum.read() != AppStateEnum::Recording {
        return Err(SessionError::other("Session has already ended"));
    }
    push_f32(ctx, samples)
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
    match tx.try_send(samples.to_vec()) {
        Ok(()) => Ok(()),
        Err(mpsc::error::TrySendError::Full(_)) => {
            log::warn!("Audio sample queue full; dropped chunk to prevent memory runaway");
            Ok(())
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            Err(SessionError::other("Audio sample receiver closed"))
        }
    }
}

/// Upper bound on how long the stop path waits for the capture pipeline to
/// confirm it has handed every sample to the ASR engine. Reaching it means
/// the sample loop is wedged, which is a bug worth logging rather than a
/// normal timing variation.
const AUDIO_DRAIN_TIMEOUT: Duration = Duration::from_secs(3);

pub async fn stop(ctx: &AppContext, inject: bool) -> Result<StopOutcome, SessionError> {
    stop_at(ctx, inject, None).await
}

/// `released_at` is the `Instant` captured inside the OS hotkey callback on
/// key release. It is the honest start of `speech_end_to_final_ms`; without
/// it the metric silently excludes thread spawn and tokio scheduling.
pub async fn stop_at(
    ctx: &AppContext,
    inject: bool,
    released_at: Option<Instant>,
) -> Result<StopOutcome, SessionError> {
    stop_owned_at(ctx, inject, released_at, None).await
}

pub async fn stop_owned(
    ctx: &AppContext,
    inject: bool,
    session_id: u64,
) -> Result<StopOutcome, SessionError> {
    stop_owned_at(ctx, inject, None, Some(session_id)).await
}

async fn stop_owned_at(
    ctx: &AppContext,
    inject: bool,
    released_at: Option<Instant>,
    expected_session: Option<u64>,
) -> Result<StopOutcome, SessionError> {
    let _operation = ctx.session_operation.lock().await;
    if expected_session.is_some() && *ctx.current_session_id.read() != expected_session {
        return Err(SessionError::other("Session has already ended"));
    }
    if *ctx.state_enum.read() != AppStateEnum::Recording {
        return Ok(StopOutcome {
            raw: String::new(),
            final_text: String::new(),
            language: ctx.asr_handle.get_detected_language(),
            injected: false,
        });
    }

    let session_id = ctx.current_session_id.read().unwrap_or(0);

    ctx.latency_timer.write().speech_ended_at = Some(released_at.unwrap_or_else(Instant::now));
    ctx.audio_engine.write().stop_capture();
    // Dropping the last sender is what closes the sample channel and lets the
    // drain loop finish; it must happen before we await drain-complete.
    *ctx.recording_sample_sender.write() = None;
    let capture_kind = *ctx.capture_kind.read();
    *ctx.capture_kind.write() = CaptureKind::None;

    *ctx.state_enum.write() = AppStateEnum::Processing;
    ctx.bus.emit(DoryEvent::State(AppStateEnum::Processing));

    // Wait for an explicit drain-complete signal rather than sleeping. An
    // 80 ms sleep is not synchronization: under load the tail push_audio
    // could land after the sidecar had already reset its stream buffer,
    // silently dropping the last fraction of a second of speech.
    await_audio_drain(ctx).await;
    ctx.latency_timer.write().audio_drained_at = Some(Instant::now());

    let pcm = ctx.recording_pcm.lock().take_all();
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

    let audio_duration_ms = (pcm.len() as u64 * 1000) / 16_000;
    let silent_mic = peak < 5e-3;
    if silent_mic {
        log::warn!(
            "Microphone produced near-silence. Device='{capture_device}' peak={peak:.4} rms={rms:.4}. \
             Windows default is often Bluetooth Hands-Free, Communications, or Stereo Mix — pick a real mic in Settings → Audio."
        );
        let _ = ctx.asr_handle.cancel_stream(session_id).await;
    }
    // Audio was already streamed to the sidecar during recording via
    // spawn_sample_loop; no redundant bulk push needed at stop time.

    // The whole-utterance transcription is segment 0. Milestone 6 replaces
    // this single entry with one per VAD-delimited segment plus the tail.
    let tail_sequence_id = {
        let mut timer = ctx.latency_timer.write();
        let seq = timer.segments.len() as u32;
        timer.segment_submitted(seq, audio_duration_ms);
        seq
    };
    let raw_transcript = if silent_mic {
        String::new()
    } else {
        ctx.asr_handle
            .stop_stream(session_id)
            .await
            .map_err(SessionError::other)?
    };
    {
        let mut timer = ctx.latency_timer.write();
        timer.segment_completed(tail_sequence_id);
        timer.final_asr_at = Some(Instant::now());
    }
    // Non-fatal notices, e.g. that the dictation exceeded the single-pass audio
    // limit. Surfaced rather than logged so the user knows the transcript is
    // incomplete.
    let asr_warning = ctx.asr_handle.take_last_warning();
    if let Some(warning) = &asr_warning {
        log::warn!("ASR warning surfaced to the user: {warning}");
    }
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
    // Must match `postprocess_transcript`'s gate exactly, or the runtime is
    // warmed for a rewrite that never happens (or skipped for one that does).
    let wants_flow = intent.run_llm
        && !settings.dictation_mode.eq_ignore_ascii_case("coding")
        && !raw_transcript.is_empty();
    if wants_flow {
        ctx.bus.emit(DoryEvent::Partial(StreamingTranscriptPayload {
            committed_prefix: String::new(),
            mutable_suffix: String::new(),
            full_text: String::new(),
            language: ctx.asr_handle.get_detected_language(),
            audio_level: 0.0,
            stage: "polishing".into(),
        }));
        let flow_model = intent.flow_model.clone();
        let compute_backend = settings.refinement.device.clone();
        let override_layers = if settings.refinement.gpu_layers < 0 {
            None
        } else {
            Some(settings.refinement.gpu_layers.max(0) as u32)
        };
        let vram_reserve_mb = settings.memory_policy.vram_reserve_mb;
        let context_size = settings.refinement.context_size;
        let runtime = Arc::clone(&ctx.flow_runtime);
        // `spawn_blocking`, not `block_in_place`: the latter panics outright on
        // a current-thread runtime, so the stop path was only ever safe on
        // Tauri's multi-threaded executor.
        let backend_for_task = compute_backend.clone();
        let ensure_result = tokio::task::spawn_blocking(move || {
            runtime.ensure(
                &flow_model,
                &backend_for_task,
                override_layers,
                vram_reserve_mb,
                context_size,
            )
        })
        .await
        .unwrap_or_else(|err| Err(format!("refinement runtime task failed: {err}")));
        if let Err(err) = &ensure_result {
            // Hand recovery to whoever installed the hook. The session layer
            // does not know (or need to know) that this ends in a download
            // driven by the GUI runtime.
            let hook = ctx.runtime_recovery.read().clone();
            if let Some(hook) = hook {
                hook(crate::context::RuntimeRecoveryRequest {
                    compute_backend: compute_backend.clone(),
                    reason: err.clone(),
                });
            }
        }
    }
    // Attribute runtime-readiness cost to its own stage. When the runtime is
    // pre-warmed this is ~0; a multi-second value is the cold start that
    // Milestone 4 removes.
    ctx.latency_timer.write().llm_ready_at = Some(Instant::now());

    let client = ctx.flow_runtime.client.read().clone();
    let stage_start = Instant::now();
    // Stage 1 formatting and the Stage 2 HTTP call are both blocking. Running
    // them on a blocking thread keeps the async executor free and removes the
    // asymmetry where `ensure()` was wrapped and the rewrite was not.
    let processed = {
        let raw = raw_transcript.clone();
        let settings = settings.clone();
        let focused = focused_process.clone();
        tokio::task::spawn_blocking(move || {
            postprocess_transcript(&raw, &settings, &focused, &client)
        })
        .await
        .map_err(|err| SessionError::other(format!("postprocess task failed: {err}")))?
    };
    let smart_transcript = processed.smart;
    let final_transcript = processed.final_text;
    let rewriter_used = processed.rewriter_used;
    let rewriter_error = processed.rewriter_error;
    {
        let mut timer = ctx.latency_timer.write();
        let formatting_done = stage_start + Duration::from_millis(processed.formatting_ms);
        timer.formatting_finished_at = Some(formatting_done);
        if let Some(rewrite_ms) = processed.rewrite_ms {
            timer.rewrite_started_at = Some(formatting_done);
            timer.rewrite_finished_at = Some(formatting_done + Duration::from_millis(rewrite_ms));
        }
        timer.rewrite_applied = rewriter_used;
    }

    let mut injected = false;
    let (mut app_title, mut process_name) = ("Android".to_string(), "companion".to_string());

    if inject && !silent_mic && !final_transcript.is_empty() {
        *ctx.state_enum.write() = AppStateEnum::Injecting;
        ctx.bus.emit(DoryEvent::State(AppStateEnum::Injecting));
        ctx.bus.emit(DoryEvent::Stage(Stage::Inject));
        // Focus is taken (and confirmed) inside `TextInjector::inject`, which
        // is the only place that can act on it failing.
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
        let tier_status = rewriter_error
            .as_ref()
            .map(|err| format!("rewriter_error: {err}"));
        let feedback = InjectionFeedback {
            pasted: outcome.pasted,
            fallback_copy: outcome.fallback_copy,
            paste_chord: outcome.paste_chord.clone(),
            process_name: outcome.process_name.clone(),
            message: injection_message(&InjectionMessageInput {
                silent_mic,
                capture_device: &capture_device,
                transcript_empty: final_transcript.is_empty(),
                fallback_copy: outcome.fallback_copy,
                paste_chord: &outcome.paste_chord,
                rewriter_error: rewriter_error.as_deref(),
                wants_flow,
                rewriter_used,
                asr_warning: asr_warning.as_deref(),
            }),
            tier_status,
        };
        ctx.bus.emit(DoryEvent::Injection(feedback));
    } else {
        ctx.latency_timer.write().injection_finished_at = Some(Instant::now());
        if inject {
            let tier_status = rewriter_error
                .as_ref()
                .map(|err| format!("rewriter_error: {err}"));
            let message = injection_message(&InjectionMessageInput {
                silent_mic,
                capture_device: &capture_device,
                transcript_empty: final_transcript.is_empty(),
                fallback_copy: false,
                paste_chord: "",
                rewriter_error: rewriter_error.as_deref(),
                wants_flow,
                rewriter_used,
                asr_warning: asr_warning.as_deref(),
            });
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

    let metrics = ctx.latency_timer.read().to_metrics(audio_duration_ms);
    log::info!(
        "Latency: hotkey→rec {} ms, rec→audio {} ms, audio→partial {} ms, \
         release→final {} ms, llm start {} ms, format {} ms, rewrite {} ms (applied={}), \
         release→inserted {} ms, total {} ms, audio {} ms, rtf {:.3}",
        metrics.hotkey_to_recording_ms,
        metrics.recording_to_first_audio_ms,
        metrics.audio_to_first_partial_ms,
        metrics.speech_end_to_final_ms,
        metrics.llm_startup_ms,
        metrics.formatting_ms,
        metrics.rewrite_ms,
        metrics.rewrite_applied,
        metrics.release_to_inserted_ms,
        metrics.total_duration_ms,
        metrics.audio_duration_ms,
        metrics.rtf,
    );
    ctx.latency_history.write().push(&metrics);
    *ctx.last_latency_metrics.write() = metrics;
    // Persist so `reflow --latency-json` reports real measurements from a
    // separate process instead of inventing them.
    crate::state::persist_latency_report(&crate::state::build_latency_report(ctx));

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
            language: ctx.asr_handle.get_detected_language(),
            raw_transcript: raw_transcript.clone(),
            final_transcript: final_transcript.clone(),
            application_name: app_title,
            application_process: process_name,
            word_count: final_transcript.split_whitespace().count(),
            character_count: final_transcript.chars().count(),
            model_version: crate::model::spec_for(&settings.asr.model)
                .label
                .to_string(),
            processing_mode: cleanup_level.clone(),
            smart_transcript: smart_transcript.clone(),
            rewriter_used,
        };
        let _ = ctx.history_store.insert_entry(&entry);
    }

    let language = ctx.asr_handle.get_detected_language();
    let payload = StreamingTranscriptPayload {
        committed_prefix: final_transcript.clone(),
        mutable_suffix: String::new(),
        full_text: final_transcript.clone(),
        language: language.clone(),
        audio_level: 0.0,
        stage: String::new(),
    };
    ctx.bus.emit(DoryEvent::Final(payload));
    ctx.bus.emit(DoryEvent::SessionFinished {
        session_id,
        raw: raw_transcript.clone(),
        text: final_transcript.clone(),
        language: language.clone(),
        metrics: ctx.last_latency_metrics.read().clone(),
    });
    reset_ready(ctx);

    Ok(StopOutcome {
        raw: raw_transcript,
        final_text: final_transcript,
        language,
        injected,
    })
}

pub fn cancel(ctx: &AppContext) -> Result<(), SessionError> {
    cancel_owned(ctx, None)
}

pub fn cancel_owned(ctx: &AppContext, expected_session: Option<u64>) -> Result<(), SessionError> {
    let _operation = ctx
        .session_operation
        .try_lock()
        .map_err(|_| SessionError::busy())?;
    if expected_session.is_some() && *ctx.current_session_id.read() != expected_session {
        return Ok(());
    }
    let session_id = ctx.current_session_id.write().take().unwrap_or(0);
    ctx.audio_engine.write().stop_capture();
    *ctx.recording_sample_sender.write() = None;
    *ctx.capture_kind.write() = CaptureKind::None;
    ctx.recording_pcm.lock().clear();
    // Drop the drain waiter: nothing will await it, and leaving it behind
    // would let the next session's stop path resolve against a stale signal.
    ctx.audio_drain_done.lock().take();
    *ctx.dictation_target_hwnd.write() = 0;
    ctx.asr_handle
        .cancel_stream_blocking(session_id)
        .map_err(SessionError::other)?;
    reset_ready(ctx);
    Ok(())
}

fn style_from_process(process: &str, fallback: &str) -> VoiceStyle {
    let p = process.to_lowercase();
    if p.contains("discord")
        || p.contains("slack")
        || p.contains("telegram")
        || p.contains("whatsapp")
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

fn begin_session(
    ctx: &AppContext,
    kind: CaptureKind,
    pressed_at: Option<Instant>,
) -> Result<(), SessionError> {
    let mut timer = ctx.latency_timer.write();
    timer.reset();
    // Only the press instant is known here. `recording_started_at` is
    // assigned by the caller once the capture stream is actually running;
    // setting both to `now` is what pinned hotkey_to_recording_ms to zero.
    timer.hotkey_pressed_at = Some(pressed_at.unwrap_or_else(Instant::now));
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
    *ctx.current_session_id.write() = None;
    crate::hotkey::hook::reset_mode();
    *ctx.state_enum.write() = AppStateEnum::Ready;
    *ctx.capture_kind.write() = CaptureKind::None;
    *ctx.last_audio_level.write() = 0.0;
    ctx.bus.emit(DoryEvent::State(AppStateEnum::Ready));
}

/// Block the stop path until the sample loop confirms every captured chunk
/// reached the ASR engine, or until `AUDIO_DRAIN_TIMEOUT` elapses.
async fn await_audio_drain(ctx: &AppContext) {
    let Some(rx) = ctx.audio_drain_done.lock().take() else {
        // No loop was running for this session (or it already finished and
        // consumed its slot); nothing to wait for.
        return;
    };
    match tokio::time::timeout(AUDIO_DRAIN_TIMEOUT, rx).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => {
            // The loop task was dropped without signalling. Its audio is
            // already in the sidecar or lost; either way, do not hang.
            log::warn!("Audio drain signal channel closed before completion");
        }
        Err(_) => {
            log::error!(
                "Audio drain did not complete within {:?}; the tail of this \
                 utterance may be missing",
                AUDIO_DRAIN_TIMEOUT
            );
        }
    }
}

fn spawn_sample_loop(ctx: &AppContext, session_id: u64, mut rx_samples: mpsc::Receiver<Vec<f32>>) {
    let state_ref = Arc::clone(&ctx.state_enum);
    let session_ref = Arc::clone(&ctx.current_session_id);
    let settings_ref = Arc::clone(&ctx.settings_store);
    let latency_ref = Arc::clone(&ctx.latency_timer);
    let bus = ctx.bus.clone();
    let level_ref = Arc::clone(&ctx.last_audio_level);
    let pcm_ref = Arc::clone(&ctx.recording_pcm);
    let asr_handle = ctx.asr_handle.clone();
    let language_hint = ctx.asr_handle.get_detected_language();

    let (drain_tx, drain_rx) = tokio::sync::oneshot::channel::<()>();
    *ctx.audio_drain_done.lock() = Some(drain_rx);

    tokio::spawn(async move {
        let mut first_audio = true;
        let mut last_level_emit = Instant::now();
        // Accumulate samples before pushing to the sidecar so we don't
        // send hundreds of tiny JSON messages per second.  ~0.5 s at
        // 16 kHz = 8 000 samples is a good trade-off between latency
        // and IPC overhead.
        let mut pending_for_asr: Vec<f32> = Vec::with_capacity(16000);
        const ASR_PUSH_THRESHOLD: usize = 8000; // ~0.5 s at 16 kHz
        let mut sequence_id = 0u32;

        loop {
            if *state_ref.read() != AppStateEnum::Recording {
                break;
            }

            let Some(chunk) = rx_samples.recv().await else {
                break;
            };

            {
                let active = session_ref.read();
                if *active != Some(session_id) {
                    break;
                }
            }
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
            {
                let active = session_ref.read();
                if *active != Some(session_id) {
                    break;
                }
                pcm_ref.lock().extend_from_slice(&chunk);
                pending_for_asr.extend_from_slice(&chunk);
            }

            // Push to sidecar in ~0.5 s batches.
            if pending_for_asr.len() >= ASR_PUSH_THRESHOLD {
                let samples = std::mem::take(&mut pending_for_asr);
                let current_seq = sequence_id;
                sequence_id += 1;
                match asr_handle
                    .push_audio(session_id, current_seq, samples)
                    .await
                {
                    Ok(Some(text))
                        if !text.trim().is_empty() && *session_ref.read() == Some(session_id) =>
                    {
                        // First live partial for this session. Until
                        // chunk-on-silence lands this only fires for engines
                        // that return incremental text, but the metric and
                        // the event path are live and testable now.
                        let mut timer = latency_ref.write();
                        if timer.first_partial_at.is_none() {
                            timer.first_partial_at = Some(Instant::now());
                        }
                        drop(timer);

                        let settings = settings_ref.get();
                        let replacements = crate::formatting::CustomReplacements::new(
                            settings.custom_replacements.clone(),
                        );
                        let formatted = crate::formatting::format_partial(
                            &text,
                            &settings.dictation_mode,
                            settings.spoken_punctuation_enabled,
                            &replacements,
                        );

                        bus.emit(DoryEvent::Partial(StreamingTranscriptPayload {
                            committed_prefix: String::new(),
                            mutable_suffix: formatted.clone(),
                            full_text: formatted,
                            language: language_hint.clone(),
                            audio_level: level,
                            stage: "transcribing".into(),
                        }));
                    }
                    Ok(_) => {}
                    Err(err) => log::warn!("push_audio failed mid-session: {err}"),
                }
            }

            if last_level_emit.elapsed() >= Duration::from_millis(40) {
                bus.emit(DoryEvent::AudioLevel(level));
                last_level_emit = Instant::now();
            }
        }

        // Drain chunks that raced with stop_capture, including the buffered
        // tail when the capture sender closes while the loop awaits recv().
        while let Ok(chunk) = rx_samples.try_recv() {
            {
                let active = session_ref.read();
                if *active != Some(session_id) {
                    break;
                }
                pcm_ref.lock().extend_from_slice(&chunk);
                pending_for_asr.extend_from_slice(&chunk);
            }
        }
        if !pending_for_asr.is_empty() && *session_ref.read() == Some(session_id) {
            let samples = std::mem::take(&mut pending_for_asr);
            let current_seq = sequence_id;
            if let Err(err) = asr_handle
                .push_audio(session_id, current_seq, samples)
                .await
            {
                log::error!("Final audio push failed; tail may be missing: {err}");
            }
        }

        // Only now is it safe for the stop path to call stop_stream(): every
        // sample this session captured is inside the engine.
        let _ = drain_tx.send(());
    });
}

/// Stop a runaway recording.
///
/// This is a stuck-key guard, not a feature limit: hold-to-talk depends on seeing
/// a key-up, and a missed one would otherwise record until the disk filled.
///
/// It deliberately does not apply to a hands-free session. Hands-free is entered
/// by double-tapping and left by tapping again, so there is no key-up to miss —
/// and the whole point of it is dictation measured in minutes or hours. Cutting
/// that off at a timeout would silently discard the recording the user was in the
/// middle of making. Because the promotion to hands-free happens on the *second*
/// tap, after this guard is already armed, the check has to happen when the
/// deadline expires rather than when it is set.
fn spawn_max_duration_watch(ctx: &AppContext, max_duration_sec: u64, session_id: u64) {
    if max_duration_sec == 0 {
        return;
    }
    let ctx = ctx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(max_duration_sec)).await;
        if *ctx.state_enum.read() != AppStateEnum::Recording {
            return;
        }
        if crate::hotkey::hook::hands_free_engaged() {
            log::info!(
                "Max recording duration ({max_duration_sec}s) reached but this is a \
                 hands-free session; continuing."
            );
            return;
        }
        log::info!("Max recording duration reached; stopping.");
        let inject_on_stop = *ctx.capture_kind.read() == CaptureKind::Microphone;
        let _ = stop_owned(&ctx, inject_on_stop, session_id).await;
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
        /// Samples the engine had received at the moment `stop_stream()` was
        /// called. This is the number that matters: anything pushed after
        /// `stop_stream()` lands in a reset buffer and is silently lost.
        samples_at_stop: Arc<AtomicUsize>,
        /// Text returned from `push_audio`, simulating an engine that emits
        /// live partials.
        partial_text: Option<String>,
    }

    impl PushProbe {
        fn new(pushed_samples: Arc<AtomicUsize>, samples_at_stop: Arc<AtomicUsize>) -> Self {
            Self {
                pushed_samples,
                samples_at_stop,
                partial_text: None,
            }
        }
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
            Ok(self.partial_text.clone())
        }

        fn get_partial_transcript(&mut self) -> Result<String, String> {
            Ok(String::new())
        }

        fn stop_stream(&mut self) -> Result<String, String> {
            self.samples_at_stop
                .store(self.pushed_samples.load(Ordering::SeqCst), Ordering::SeqCst);
            Ok("probe transcript".into())
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
        ctx.asr_handle
            .swap_engine(Box::new(PushProbe::new(
                Arc::clone(&pushed_samples),
                Arc::new(AtomicUsize::new(0)),
            )))
            .await
            .unwrap();
        ctx.asr_handle.start_stream(1, "en", &[]).await.unwrap();
        *ctx.current_session_id.write() = Some(1);
        *ctx.state_enum.write() = AppStateEnum::Recording;

        let (tx_samples, rx_samples) = mpsc::channel(64);
        spawn_sample_loop(&ctx, 1, rx_samples);
        let _ = tx_samples.try_send(vec![0.1; 100]);
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
        start_external(&ctx, Some("en".into()))
            .await
            .expect("start");
        push_f32(&ctx, &vec![0.05; 6400]).expect("push");
        let outcome = stop(&ctx, false).await.expect("stop");
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
    async fn stale_watchdog_cannot_stop_a_new_recording() {
        let dir = std::env::temp_dir().join(format!("reflow_watch_{}", uuid::Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        let old = start_external_owned(&ctx, None).await.unwrap();
        spawn_max_duration_watch(&ctx, 1, old);
        cancel_owned(&ctx, Some(old)).unwrap();
        let current = start_external_owned(&ctx, None).await.unwrap();
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert_eq!(*ctx.current_session_id.read(), Some(current));
        assert_eq!(*ctx.state_enum.read(), AppStateEnum::Recording);
        cancel_owned(&ctx, Some(current)).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn stale_owner_cannot_stop_or_cancel_a_new_recording() {
        let dir = std::env::temp_dir().join(format!("reflow_owner_{}", uuid::Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        let old = start_external_owned(&ctx, None).await.unwrap();
        cancel_owned(&ctx, Some(old)).unwrap();
        let current = start_external_owned(&ctx, None).await.unwrap();
        cancel_owned(&ctx, Some(old)).unwrap();
        assert!(stop_owned(&ctx, false, old).await.is_err());
        assert_eq!(*ctx.current_session_id.read(), Some(current));
        assert_eq!(*ctx.state_enum.read(), AppStateEnum::Recording);
        cancel_owned(&ctx, Some(current)).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn simultaneous_starts_admit_only_one_session() {
        let dir = std::env::temp_dir().join(format!("reflow_concurrent_{}", uuid::Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        let (a, b) = tokio::join!(
            start_external_owned(&ctx, None),
            start_external_owned(&ctx, None)
        );
        assert_ne!(a.is_ok(), b.is_ok());
        let id = a.ok().or_else(|| b.ok()).unwrap();
        assert_eq!(*ctx.current_session_id.read(), Some(id));
        cancel_owned(&ctx, Some(id)).unwrap();
        let _ = std::fs::remove_dir_all(dir);
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

    #[test]
    fn postprocess_falls_back_to_smart_text_when_rewriter_fails() {
        let settings = crate::settings::AppSettings {
            cleanup_level: "smart".into(),
            processing_mode: "smart_flow".into(),
            ..Default::default()
        };
        let out = postprocess_transcript(
            // Long enough that polishing is genuinely wanted: a short, already
            // well-formed sentence is now skipped before the rewriter is
            // consulted, so it would not exercise this failure path.
            "this is a raw dictation test without punctuation and it runs on long \
             enough that sentence splitting and phrasing would actually benefit \
             from the model",
            &settings,
            "notepad",
            &FlowClient::new_missing(),
        );
        // The raw/smart text is always preserved and never dropped on LLM failure
        assert!(!out.final_text.is_empty());
        assert!(!out.rewriter_used);
        assert!(out.rewriter_error.is_some());
        assert_eq!(out.final_text, out.smart);
    }
}
