use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use uuid::Uuid;

use crate::audio::AudioResampler;
use crate::context::AppContext;
use crate::dory::{CaptureKind, DoryEvent, Stage};
use crate::formatting::{
    assemble_asr_vocabulary, format_transcript_with_capitalization, CleanupLevel,
    CustomReplacements, FormatRequest, TextCleaner, VoiceStyle,
};
use crate::history::HistoryEntry;
use crate::injection::TextInjector;
use crate::rewrite::{FlowClient, RewriteRequest};
use crate::settings::AppSettings;
use crate::state::{AppStateEnum, InjectionFeedback, StreamingTranscriptPayload};

/// Bounded capacity for audio streaming channel to prevent unbounded memory growth.
pub const SAMPLE_CHANNEL_CAPACITY: usize = 256;

#[derive(Debug, Clone, Default)]
pub struct SessionContext {
    pub automation: bool,
    pub explicit_mode: bool,
    pub selected_text: Option<String>,
    pub clipboard: Option<String>,
    pub window_title: Option<String>,
}

impl SessionContext {
    pub fn inputs(&self, context_size: u32) -> Vec<(&str, String)> {
        // Reserve half the window for the instruction, transcript and completion.
        let per_section = context_size as usize * 2 / 3;
        [
            ("Selected text", &self.selected_text),
            ("Clipboard", &self.clipboard),
            ("Window", &self.window_title),
        ]
        .into_iter()
        .filter_map(|(label, text)| {
            text.as_ref()
                .filter(|s| !s.trim().is_empty())
                .map(|s| (label, s.chars().take(per_section).collect()))
        })
        .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceCommand {
    Scratch,
    Undo,
}
pub fn voice_command(text: &str) -> Option<VoiceCommand> {
    match text
        .trim()
        .trim_end_matches(['.', '!', '?'])
        .to_lowercase()
        .as_str()
    {
        "scratch that" => Some(VoiceCommand::Scratch),
        "undo that" => Some(VoiceCommand::Undo),
        _ => None,
    }
}

/// Recognize saved audio without injecting text or creating another history row.
/// Callers hold `session_operation` until the retry has been persisted.
pub async fn transcribe_saved_audio(ctx: &AppContext, id: &str) -> Result<String, String> {
    transcribe_saved_audio_with_language(ctx, id, None).await
}
pub async fn transcribe_saved_audio_with_language(
    ctx: &AppContext,
    id: &str,
    language_override: Option<&str>,
) -> Result<String, String> {
    if !matches!(
        *ctx.state_enum.read(),
        AppStateEnum::Ready | AppStateEnum::Idle
    ) {
        return Err("Finish the current dictation before retrying a transcript.".into());
    }
    let store = ctx.history_store.clone();
    let id = id.to_owned();
    let samples = tokio::task::spawn_blocking(move || store.load_audio(&id))
        .await
        .map_err(|e| e.to_string())??;
    let status = ctx.asr_handle.refresh_status().await?;
    if !status.loaded || status.is_loading {
        return Err("The speech model is not ready. Reload it in Settings before retrying.".into());
    }
    let settings = ctx.settings_store.get();
    let session_id = ctx.asr_handle.next_session_id();
    let language = language_override.unwrap_or(if settings.auto_detect_language {
        "auto"
    } else {
        &settings.language
    });
    ctx.asr_handle
        .start_stream(session_id, language, &session_vocabulary(&settings))
        .await?;
    let result = async {
        for (sequence, chunk) in samples.chunks(16_000).enumerate() {
            ctx.asr_handle
                .push_audio(session_id, sequence as u32, chunk.to_vec())
                .await?;
        }
        let text = ctx.asr_handle.stop_stream(session_id).await?;
        if text.trim().is_empty() {
            return Err("No speech was recognized. The saved transcript was kept.".into());
        }
        if let Some(warning) = ctx.asr_handle.take_last_warning() {
            return Err(format!(
                "Retry was incomplete: {warning}. The saved transcript was kept."
            ));
        }
        Ok(text)
    }
    .await;
    if result.is_err() {
        let _ = ctx.asr_handle.cancel_stream(session_id).await;
    }
    result
}

fn recover_insertion(
    result: Result<crate::injection::InjectionOutcome, String>,
) -> (crate::injection::InjectionOutcome, Option<String>) {
    match result {
        Ok(outcome) => (outcome, None),
        Err(error) => (
            crate::injection::InjectionOutcome {
                app_title: String::new(),
                process_name: String::new(),
                pasted: false,
                fallback_copy: false,
                paste_chord: String::new(),
            },
            Some(error),
        ),
    }
}

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
    let mut vocabulary = assemble_asr_vocabulary(&terms, &afters, 60);
    for snippet in settings.snippets.iter().filter(|s| s.enabled) {
        if vocabulary.len() >= 60 {
            break;
        }
        if !vocabulary
            .iter()
            .any(|s| s.eq_ignore_ascii_case(&snippet.trigger))
        {
            vocabulary.push(snippet.trigger.clone());
        }
    }
    vocabulary
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
    postprocess_with_context(
        raw,
        settings,
        focused_process,
        client,
        None,
        &SessionContext::default(),
    )
}

pub fn postprocess_with_context(
    raw: &str,
    settings: &AppSettings,
    focused_process: &str,
    client: &FlowClient,
    mode: Option<&crate::settings::Mode>,
    context: &SessionContext,
) -> PostprocessOutcome {
    let effective = settings.for_application(focused_process);
    let settings = &effective;
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

    let mut smart = format_transcript_with_capitalization(
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
        settings.capitalize_first,
    );

    if level != CleanupLevel::Raw {
        smart = TextCleaner::apply_glossary(&smart, &glossary);
    }

    // `intent.run_llm` is the single authority. Deriving this from
    // `cleanup_level` is what made the default install advertise the
    // smart_flow tier while never running Stage 2.
    let wants_flow = (intent.run_llm
        || mode
            .is_some_and(|m| !m.custom_instructions.trim().is_empty() || m.translate_to.is_some()))
        && !settings.dictation_mode.eq_ignore_ascii_case("coding")
        && !smart.is_empty();

    let formatting_ms = formatting_started.elapsed().as_millis() as u64;

    if !wants_flow {
        return PostprocessOutcome {
            smart: smart.clone(),
            final_text: crate::formatting::snippets::apply_snippets(&smart, &settings.snippets),
            rewriter_used: false,
            rewriter_error: None,
            formatting_ms,
            rewrite_ms: None,
        };
    }

    let req = RewriteRequest {
        text: smart.clone(),
        cleanup_level,
        style: match style {
            VoiceStyle::Faithful => "faithful",
            VoiceStyle::Neutral => "neutral",
            VoiceStyle::Decisive => "decisive",
            VoiceStyle::Email => "email",
            VoiceStyle::Chat => "chat",
        }
        .into(),
        dictation_mode: settings.dictation_mode.clone(),
        vocabulary: session_vocabulary(settings),
        app_process: focused_process.to_string(),
        model_id: intent.flow_model.clone(),
    };
    let rewrite_started = Instant::now();
    let task_instruction = mode.and_then(|m| {
        if let Some(language) = &m.translate_to {
            Some(
                crate::asr::languages::resolve_language_name(language).map(|language| {
                    format!(
                        "{}\nTranslate the following to {language}. Output only the translation.",
                        m.custom_instructions.trim()
                    )
                }),
            )
        } else if !m.custom_instructions.trim().is_empty() {
            Some(Ok(m.custom_instructions.clone()))
        } else {
            None
        }
    });
    let (polished, used, rewriter_error) = if let Some(instruction) = task_instruction {
        match instruction.and_then(|instruction| {
            crate::rewrite::tasks::complete_task(
                client,
                crate::rewrite::tasks::MODE_SYSTEM,
                &instruction,
                &smart,
                context,
            )
        }) {
            Ok(text) => (text, true, None),
            Err(error) => (smart.clone(), false, Some(error)),
        }
    } else {
        let outcome = crate::rewrite::client::polish_with_context_or_fallback(
            client,
            &smart,
            &req,
            &context.inputs(client.context_size),
        );
        (outcome.final_text, outcome.used, outcome.error)
    };
    let rewrite_ms = rewrite_started.elapsed().as_millis() as u64;
    // Mirror the pre-rewrite guard: raw means verbatim, so the glossary must
    // not be applied here either. An unchanged/fallback result is already
    // corrected; running replacements again could cascade A -> B -> C.
    // Keeping the check local means the invariant
    // does not silently depend on how `run_llm` is defined elsewhere.
    let final_text = if level == CleanupLevel::Raw || polished == smart {
        polished
    } else {
        let preferred = glossary
            .iter()
            .map(|(_, spelling)| (spelling.clone(), spelling.clone()))
            .collect::<Vec<_>>();
        TextCleaner::apply_glossary(&replacement_rules.restore_spellings(&polished), &preferred)
    };
    PostprocessOutcome {
        smart,
        final_text: crate::formatting::snippets::apply_snippets(&final_text, &settings.snippets),
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
    start_microphone_with_intent_at(ctx, pressed_at, crate::dory::SessionIntent::Dictate, None)
        .await
        .map(|_| ())
}

pub async fn start_microphone_with_intent_at(
    ctx: &AppContext,
    pressed_at: Option<Instant>,
    intent: crate::dory::SessionIntent,
    mode_id: Option<&str>,
) -> Result<u64, SessionError> {
    start_microphone_with_source_at(
        ctx,
        pressed_at,
        intent,
        mode_id,
        false,
        CaptureKind::Microphone,
    )
    .await
}

pub async fn start_automation(ctx: &AppContext) -> Result<u64, SessionError> {
    start_microphone_with_source_at(
        ctx,
        None,
        crate::dory::SessionIntent::Dictate,
        None,
        true,
        CaptureKind::Microphone,
    )
    .await
}

pub async fn start_meeting(ctx: &AppContext) -> Result<u64, SessionError> {
    if !ctx.settings_store.get().meeting_mode {
        return Err(SessionError::other(
            "Enable meeting mode in Advanced settings first",
        ));
    }
    start_microphone_with_source_at(
        ctx,
        None,
        crate::dory::SessionIntent::Dictate,
        None,
        false,
        CaptureKind::SystemMix,
    )
    .await
}

async fn start_microphone_with_source_at(
    ctx: &AppContext,
    pressed_at: Option<Instant>,
    intent: crate::dory::SessionIntent,
    mode_id: Option<&str>,
    automation: bool,
    capture_kind: CaptureKind,
) -> Result<u64, SessionError> {
    let _operation = ctx.session_operation.lock().await;
    let state = *ctx.state_enum.read();

    if !matches!(state, AppStateEnum::Ready | AppStateEnum::Idle) {
        return Err(SessionError::busy());
    }

    crate::overlay::restore_dictation_focus().map_err(SessionError::other)?;
    let exclusions = ctx.settings_store.get().excluded_apps;
    let process = crate::platform::active_window().1;
    if crate::settings::AppSettings::process_is_excluded(&process, &exclusions) {
        let message = format!("Recording paused in {process}");
        ctx.bus.emit(DoryEvent::Injection(InjectionFeedback {
            pasted: false,
            fallback_copy: false,
            paste_chord: String::new(),
            process_name: process,
            message: message.clone(),
            tier_status: None,
        }));
        return Err(SessionError::other(message));
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

    let base = ctx.settings_store.get();
    let (title, process) = crate::platform::active_window();
    let mode = match mode_id {
        Some(id) => base
            .modes
            .iter()
            .find(|m| m.enabled && m.id == id)
            .cloned()
            .ok_or_else(|| SessionError::other("The selected mode is disabled or missing"))?,
        None => base.resolve_mode(&process),
    };
    let settings = base.settings_for_mode(&process, &mode);
    *ctx.session_intent.write() = intent;
    *ctx.session_mode.write() = Some(mode.clone());
    let session_id = ctx.asr_handle.next_session_id();
    *ctx.current_session_id.write() = Some(session_id);

    begin_session(ctx, capture_kind, pressed_at)?;
    if settings.duck_media && capture_kind != CaptureKind::SystemMix {
        if let Err(error) = crate::platform::media::duck() {
            log::warn!("Media duck unavailable: {error}");
        }
    }

    let context_flags = mode.context.clone();
    let explicit_mode = mode_id.is_some();
    let captured = tokio::task::spawn_blocking(move || {
        let selected_text = if context_flags.selected_text {
            crate::platform::capture_selection().ok().flatten()
        } else {
            None
        };
        let clipboard = if context_flags.clipboard {
            crate::injection::injector::clipboard_text().ok().flatten()
        } else {
            None
        };
        if context_flags.selected_text || context_flags.clipboard || context_flags.window_title {
            log::info!("Captured opted-in local context for mode {}", mode.id);
        }
        SessionContext {
            automation,
            explicit_mode,
            selected_text,
            clipboard,
            window_title: context_flags.window_title.then_some(title),
        }
    })
    .await
    .map_err(|e| SessionError::other(e.to_string()))?;
    *ctx.session_context.write() = captured;
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

    let capture_res = if capture_kind == CaptureKind::SystemMix {
        ctx.audio_engine.write().start_meeting_capture(
            settings.microphone_device_id.clone(),
            settings.meeting_monitor_device_id.clone(),
            settings.input_gain,
            tx_samples,
            tx_auto_stop,
        )
    } else {
        ctx.audio_engine.write().start_capture(
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
        )
    };
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
            if let Some(error) = ctx_stop.audio_engine.read().stream_error() {
                ctx_stop.bus.emit(DoryEvent::Error(error));
            }
            ctx_stop.bus.emit(DoryEvent::AutoStop);
            let _ = stop_owned(
                &ctx_stop,
                capture_kind != CaptureKind::SystemMix,
                session_id,
            )
            .await;
        }
    });

    spawn_max_duration_watch(
        ctx,
        if capture_kind == CaptureKind::SystemMix {
            3600
        } else {
            settings.max_duration_sec
        },
        session_id,
    );
    Ok(session_id)
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
    start_external_from(ctx, language, None).await
}

pub async fn start_phone_owned(
    ctx: &AppContext,
    language: Option<String>,
    token: &str,
) -> Result<u64, SessionError> {
    start_external_from(ctx, language, Some(token)).await
}

async fn start_external_from(
    ctx: &AppContext,
    language: Option<String>,
    phone_token: Option<&str>,
) -> Result<u64, SessionError> {
    let _operation = ctx.session_operation.lock().await;
    if phone_token.is_some_and(|token| {
        !ctx.settings_store.get().api_enabled
            || !ctx.pairing.permissions(token).is_some_and(|p| p.stream)
    }) {
        return Err(SessionError::other("Phone stream authorization ended"));
    }
    let state = *ctx.state_enum.read();
    if !matches!(state, AppStateEnum::Ready | AppStateEnum::Idle) {
        return Err(SessionError::busy());
    }

    let session_id = ctx.asr_handle.next_session_id();
    *ctx.current_session_id.write() = Some(session_id);

    *ctx.session_intent.write() = crate::dory::SessionIntent::Dictate;
    *ctx.session_mode.write() = None;
    *ctx.session_context.write() = SessionContext::default();
    begin_session(ctx, CaptureKind::External, None)?;
    *ctx.session_phone_token.write() = phone_token.map(str::to_owned);

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
    if !bytes.len().is_multiple_of(2) {
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
        Ok(()) => {
            let mut timer = ctx.latency_timer.write();
            timer.peak_audio_queue_chunks = timer
                .peak_audio_queue_chunks
                .max(tx.max_capacity() - tx.capacity());
            Ok(())
        }
        Err(mpsc::error::TrySendError::Full(_)) => {
            ctx.latency_timer.write().dropped_audio_chunks += 1;
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
    stop_owned_at(ctx, inject, released_at, None, None).await
}

pub fn hotkey_owns_session(ctx: &AppContext, action: &str) -> bool {
    use crate::dory::SessionIntent;
    if ctx.session_context.read().automation
        || *ctx.capture_kind.read() != CaptureKind::Microphone
        || *ctx.state_enum.read() != AppStateEnum::Recording
    {
        return false;
    }
    let intent = *ctx.session_intent.read();
    match action {
        "command" => intent == SessionIntent::Command,
        "assistant" => intent == SessionIntent::Assistant,
        "note" => intent == SessionIntent::Note,
        "dictate" => intent == SessionIntent::Dictate && !ctx.session_context.read().explicit_mode,
        action if action.starts_with("mode:") => {
            intent == SessionIntent::Dictate
                && ctx.session_context.read().explicit_mode
                && ctx
                    .session_mode
                    .read()
                    .as_ref()
                    .is_some_and(|m| m.id == action[5..])
        }
        _ => false,
    }
}

pub async fn stop_hotkey_owned_at(
    ctx: &AppContext,
    session_id: u64,
    released_at: Instant,
) -> Result<StopOutcome, SessionError> {
    stop_owned_at(ctx, true, Some(released_at), Some(session_id), None).await
}

pub async fn stop_owned(
    ctx: &AppContext,
    inject: bool,
    session_id: u64,
) -> Result<StopOutcome, SessionError> {
    stop_owned_at(ctx, inject, None, Some(session_id), None).await
}

pub async fn stop_phone(
    ctx: &AppContext,
    inject: bool,
    session_id: u64,
    token: &str,
) -> Result<StopOutcome, SessionError> {
    stop_owned_at(ctx, inject, None, Some(session_id), Some(token)).await
}

async fn stop_owned_at(
    ctx: &AppContext,
    inject: bool,
    released_at: Option<Instant>,
    expected_session: Option<u64>,
    phone_token: Option<&str>,
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
    let session_phone_token = ctx.session_phone_token.read().clone();
    let phone_token = session_phone_token.as_deref().or(phone_token);
    let minimum = ctx.settings_store.get().min_dictation_ms;
    let too_short = released_at
        .zip(ctx.latency_timer.read().recording_started_at)
        .is_some_and(|(end, start)| {
            end.saturating_duration_since(start).as_millis() < minimum as u128
        });
    if too_short && *ctx.capture_kind.read() == CaptureKind::Microphone {
        cancel_unlocked(ctx, Some(session_id))?;
        return Ok(StopOutcome {
            raw: String::new(),
            final_text: String::new(),
            language: ctx.asr_handle.get_detected_language(),
            injected: false,
        });
    }

    ctx.latency_timer.write().speech_ended_at = Some(released_at.unwrap_or_else(Instant::now));
    ctx.audio_engine.write().stop_capture();
    crate::platform::media::restore();
    if matches!(
        *ctx.capture_kind.read(),
        CaptureKind::Microphone | CaptureKind::SystemMix
    ) {
        ctx.latency_timer.write().dropped_audio_chunks += ctx.audio_engine.read().dropped_chunks();
    }
    // Dropping the last sender is what closes the sample channel and lets the
    // drain loop finish; it must happen before we await drain-complete.
    *ctx.recording_sample_sender.write() = None;
    let capture_kind = *ctx.capture_kind.read();
    *ctx.capture_kind.write() = CaptureKind::None;

    *ctx.state_enum.write() = AppStateEnum::Processing;
    ctx.bus.emit(DoryEvent::State(AppStateEnum::Processing));

    // The session is committed to stopping now, so a pipeline failure must
    // still return the app to Ready — otherwise one bad stop strands the
    // state machine in Processing and blocks every later dictation.
    let mut cancelled = ctx
        .session_cancel
        .read()
        .as_ref()
        .map(|sender| sender.subscribe());
    let result = tokio::select! {
        biased;
        _ = async {
            if let Some(receiver) = &mut cancelled {
                while !*receiver.borrow() {
                    if receiver.changed().await.is_err() { break; }
                }
            } else {
                std::future::pending::<()>().await;
            }
        } => None,
        result = finish_stop(ctx, inject, session_id, capture_kind, phone_token) => Some(result),
    };
    let Some(result) = result else {
        // The selected pipeline future has been dropped before asking the ASR
        // actor to cancel. Its reply can no longer reach output/history.
        if let Err(error) = ctx.asr_handle.cancel_stream(session_id).await {
            log::warn!("Could not cancel processing ASR: {error}");
        }
        ctx.recording_pcm.lock().clear();
        *ctx.dictation_target_hwnd.write() = 0;
        reset_ready(ctx);
        return Ok(StopOutcome {
            raw: String::new(),
            final_text: String::new(),
            language: ctx.asr_handle.get_detected_language(),
            injected: false,
        });
    };
    if let Err(err) = &result {
        ctx.bus.emit(DoryEvent::Error(err.message.clone()));
        reset_ready(ctx);
    }
    result
}

/// The stop pipeline, run after `stop_owned_at` has committed to
/// `Processing`. Any `?` here aborts the dictation; `stop_owned_at` owns
/// the cleanup so every exit lands in a recoverable state.
async fn finish_stop(
    ctx: &AppContext,
    inject: bool,
    session_id: u64,
    capture_kind: CaptureKind,
    phone_token: Option<&str>,
) -> Result<StopOutcome, SessionError> {
    // Wait for an explicit drain-complete signal rather than sleeping. An
    // 80 ms sleep is not synchronization: under load the tail push_audio
    // could land after the sidecar had already reset its stream buffer,
    // silently dropping the last fraction of a second of speech.
    if let Err(error) = await_audio_drain(ctx).await {
        // Revoke ownership before awaiting the actor: a late push reply must
        // not mutate this recording or a later session while cancellation runs.
        *ctx.current_session_id.write() = None;
        ctx.recording_pcm.lock().clear();
        *ctx.dictation_target_hwnd.write() = 0;
        if let Err(cancel_error) = ctx.asr_handle.cancel_stream(session_id).await {
            log::warn!("Could not cancel ASR after audio drain failure: {cancel_error}");
        }
        return Err(error);
    }
    ctx.latency_timer.write().audio_drained_at = Some(Instant::now());

    let recorded_at = chrono::Utc::now();
    let (pcm, complete_audio, peak, total_samples) = {
        let mut buffer = ctx.recording_pcm.lock();
        let complete = !buffer.overflowed();
        let peak = buffer.peak();
        let total_samples = buffer.total_samples();
        (buffer.take_all(), complete, peak, total_samples)
    };
    let n = pcm.len().max(1);
    let rms = (pcm.iter().map(|s| s * s).sum::<f32>() / n as f32).sqrt();
    let capture_device = ctx.audio_engine.read().last_device_name();
    log::info!(
        "Captured {:.2}s of audio (rms={:.4}, peak={:.3}, {} samples) from '{capture_device}'",
        pcm.len() as f32 / 16000.0,
        rms,
        peak,
        pcm.len()
    );

    let audio_duration_ms = (total_samples * 1000) / 16_000;
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
    ensure_session_authority(ctx, session_id, phone_token)?;
    // Non-fatal notices, e.g. that the dictation exceeded the single-pass audio
    // limit. Surfaced rather than logged so the user knows the transcript is
    // incomplete.
    let mut asr_warning = ctx.asr_handle.take_last_warning();
    {
        let timer = ctx.latency_timer.read();
        if timer.dropped_audio_chunks > 0 || timer.failed_audio_pushes > 0 {
            let warning = format!("Audio may be incomplete: {} dropped chunks, {} failed transfers. Check the transcript.", timer.dropped_audio_chunks, timer.failed_audio_pushes);
            asr_warning =
                Some(asr_warning.map_or(warning.clone(), |prior| format!("{prior} · {warning}")));
        }
    }
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

    let settings = {
        let ctx = ctx.clone();
        tokio::task::spawn_blocking(move || crate::runtime::current_settings(&ctx))
            .await
            .map_err(|err| SessionError::other(format!("runtime planning failed: {err}")))?
    };
    ensure_session_authority(ctx, session_id, phone_token)?;
    if settings.voice_commands_enabled
        && capture_kind == CaptureKind::Microphone
        && *ctx.session_intent.read() == crate::dory::SessionIntent::Dictate
    {
        if let Some(command) = voice_command(&raw_transcript) {
            let text = if command == VoiceCommand::Undo {
                crate::commands::undo_last_ai_edit_inner(ctx).map_err(SessionError::other)?
            } else {
                String::new()
            };
            let mut pasted = false;
            if !text.is_empty() && inject {
                wait_to_deliver(ctx, session_id, phone_token, settings.paste_delay_ms).await?;
                let text_for_paste = text.clone();
                let settings = settings.clone();
                let hwnd = *ctx.dictation_target_hwnd.read();
                let delivery_ctx = ctx.clone();
                pasted = tokio::task::spawn_blocking(move || {
                    ensure_session_authority(&delivery_ctx, session_id, None)
                        .map_err(|error| error.message)?;
                    TextInjector::deliver_configured(
                        &text_for_paste,
                        &crate::settings::OutputAction::Paste,
                        hwnd,
                        settings.clipboard_restore_enabled,
                        &settings.send_key,
                        0,
                        &settings.inject_method,
                    )
                    .map(|outcome| outcome.pasted)
                })
                .await
                .map_err(|e| SessionError::other(e.to_string()))?
                .map_err(SessionError::other)?;
            }
            ctx.bus.emit(DoryEvent::Final(StreamingTranscriptPayload {
                committed_prefix: text.clone(),
                mutable_suffix: String::new(),
                full_text: text.clone(),
                language: ctx.asr_handle.get_detected_language(),
                audio_level: 0.0,
                stage: String::new(),
            }));
            ctx.bus.emit(DoryEvent::Injection(InjectionFeedback {
                pasted,
                fallback_copy: command == VoiceCommand::Undo && !pasted && !text.is_empty(),
                paste_chord: String::new(),
                process_name: String::new(),
                message: if command == VoiceCommand::Scratch {
                    "Discarded"
                } else if text.is_empty() {
                    "Nothing to undo"
                } else if pasted {
                    "Pre-AI text pasted"
                } else {
                    "Pre-AI text available in Home"
                }
                .into(),
                tier_status: None,
            }));
            let language = ctx.asr_handle.get_detected_language();
            ctx.bus.emit(DoryEvent::SessionFinished {
                session_id,
                raw: raw_transcript.clone(),
                text: text.clone(),
                language: language.clone(),
                metrics: ctx.latency_timer.read().to_metrics(audio_duration_ms),
            });
            reset_ready(ctx);
            return Ok(StopOutcome {
                raw: raw_transcript,
                final_text: text,
                language,
                injected: pasted,
            });
        }
    }
    let focused_process = if capture_kind == CaptureKind::External {
        "companion".into()
    } else {
        crate::platform::active_window().1
    };
    let session_intent = *ctx.session_intent.read();
    let mut mode = ctx
        .session_mode
        .read()
        .clone()
        .unwrap_or_else(|| settings.resolve_mode(&focused_process));
    let session_context = ctx.session_context.read().clone();
    let mut processing_text = raw_transcript.clone();
    if session_intent == crate::dory::SessionIntent::Dictate && !session_context.explicit_mode {
        if let Some((spoken_mode, rest)) = settings.spoken_mode(&processing_text) {
            mode = spoken_mode.clone();
            processing_text = rest.to_owned();
        }
    }
    *ctx.session_mode.write() = Some(mode.clone());
    let settings = settings.settings_for_mode(&focused_process, &mode);
    let mut intent = settings.resolve_intent();
    let command_input = if session_intent == crate::dory::SessionIntent::Command
        && !raw_transcript.trim().is_empty()
    {
        let hwnd = *ctx.dictation_target_hwnd.read();
        tokio::task::spawn_blocking(move || {
            if hwnd != 0
                && !crate::platform::focus_hwnd_and_confirm(hwnd, Duration::from_millis(400))
            {
                return None;
            }
            crate::platform::capture_selection().ok().flatten()
        })
        .await
        .ok()
        .flatten()
    } else {
        None
    };
    ctx.bus.emit(DoryEvent::Stage(Stage::Format));
    let cleanup_level = intent.cleanup_level.clone();
    let task_requested = session_intent == crate::dory::SessionIntent::Assistant
        || command_input.is_some()
        || !mode.custom_instructions.trim().is_empty()
        || mode.translate_to.is_some();
    let wants_flow = (session_intent == crate::dory::SessionIntent::Assistant
        || command_input.is_some()
        || ((intent.run_llm || task_requested)
            && !settings.dictation_mode.eq_ignore_ascii_case("coding")))
        && !processing_text.trim().is_empty();
    if wants_flow && intent.flow_model == "none" {
        intent.flow_model = "qwen3.5-0.8b".into();
    }
    if wants_flow {
        ctx.latency_timer.write().llm_was_warm = ctx.flow_runtime.status_ready();
        ctx.bus.emit(DoryEvent::Partial(StreamingTranscriptPayload {
            committed_prefix: String::new(),
            mutable_suffix: String::new(),
            full_text: String::new(),
            language: ctx.asr_handle.get_detected_language(),
            audio_level: 0.0,
            stage: if command_input.is_some() {
                "editing"
            } else {
                "polishing"
            }
            .into(),
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
        let deadline_ms = settings.refinement.deadline_ms;
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
        if ensure_result.is_ok() {
            // Client-side deadline, applied after ensure so a relaunch with a
            // fresh client does not lose it.
            ctx.flow_runtime.set_deadline_ms(deadline_ms);
        }
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
    let (processed, generation) = {
        let raw = processing_text.clone();
        let settings = settings.clone();
        let focused = focused_process.clone();
        let task_input = command_input.clone();
        let mode = mode.clone();
        let context = session_context.clone();
        tokio::task::spawn_blocking(move || {
            let processed = if session_intent == crate::dory::SessionIntent::Assistant {
                let started = Instant::now();
                let result = crate::rewrite::tasks::complete_task(
                    &client,
                    crate::rewrite::tasks::ASSISTANT_SYSTEM,
                    "Answer the supplied question. Use context only when relevant. If the user explicitly asks to search the web, propose exactly one line SEARCH: followed by the query. If they explicitly ask to save or remember a note, propose exactly one line NOTE: followed by the note. A button will ask the user to activate the proposal. Otherwise answer normally.",
                    &raw,
                    &context,
                );
                match result {
                    Ok(text) => PostprocessOutcome {
                        smart: raw.clone(),
                        final_text: text,
                        rewriter_used: true,
                        rewriter_error: None,
                        formatting_ms: 0,
                        rewrite_ms: Some(started.elapsed().as_millis() as u64),
                    },
                    Err(error) => PostprocessOutcome {
                        smart: raw.clone(),
                        final_text: String::new(),
                        rewriter_used: false,
                        rewriter_error: Some(error),
                        formatting_ms: 0,
                        rewrite_ms: Some(started.elapsed().as_millis() as u64),
                    },
                }
            } else if let Some(selection) = task_input {
                let started = Instant::now();
                let result =
                    crate::rewrite::tasks::command_instruction(&raw).and_then(|instruction| {
                        crate::rewrite::tasks::complete_task(
                            &client,
                            crate::rewrite::tasks::EDIT_SYSTEM,
                            &instruction,
                            &selection,
                            &context,
                        )
                    });
                match result {
                    Ok(text) => PostprocessOutcome {
                        smart: raw.clone(),
                        final_text: text,
                        rewriter_used: true,
                        rewriter_error: None,
                        formatting_ms: 0,
                        rewrite_ms: Some(started.elapsed().as_millis() as u64),
                    },
                    Err(error) => PostprocessOutcome {
                        smart: raw.clone(),
                        final_text: String::new(),
                        rewriter_used: false,
                        rewriter_error: Some(error),
                        formatting_ms: 0,
                        rewrite_ms: Some(started.elapsed().as_millis() as u64),
                    },
                }
            } else {
                postprocess_with_context(&raw, &settings, &focused, &client, Some(&mode), &context)
            };
            let generation = client.generation.read().clone();
            (processed, generation)
        })
        .await
        .map_err(|err| SessionError::other(format!("postprocess task failed: {err}")))?
    };
    if !settings.refinement.keep_warm {
        let runtime = ctx.flow_runtime.clone();
        let _ = tokio::task::spawn_blocking(move || runtime.shutdown()).await;
    }
    let smart_transcript = processed.smart;
    let mut final_transcript = processed.final_text;
    if settings.append_space
        && session_intent == crate::dory::SessionIntent::Dictate
        && !final_transcript.is_empty()
        && !final_transcript.ends_with(char::is_whitespace)
    {
        final_transcript.push(' ');
    }
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
    ensure_session_authority(ctx, session_id, phone_token)?;
    // Recheck after all awaited ASR/LLM work, immediately before insertion.
    let inject = inject
        && capture_kind != CaptureKind::SystemMix
        && phone_token.is_none_or(|token| {
            ctx.settings_store.get().api_enabled
                && ctx
                    .pairing
                    .permissions(token)
                    .is_some_and(|p| p.stream && p.injection)
        });
    let (mut app_title, mut process_name) = if capture_kind == CaptureKind::SystemMix {
        ("Meeting".to_owned(), "reflow".to_owned())
    } else {
        ("Android".to_owned(), "companion".to_owned())
    };
    let mut correction_target = None;
    let mut correction_history_id = None;

    if inject && !silent_mic && !final_transcript.is_empty() {
        *ctx.state_enum.write() = AppStateEnum::Injecting;
        ctx.bus.emit(DoryEvent::State(AppStateEnum::Injecting));
        ctx.bus.emit(DoryEvent::Stage(Stage::Inject));
        // Focus is taken (and confirmed) inside `TextInjector::inject`, which
        // is the only place that can act on it failing.
        let mode = ctx.session_mode.read().clone();
        let normal_action = mode
            .as_ref()
            .filter(|m| m.id != "dictation")
            .map(|m| &m.output)
            .unwrap_or(&settings.output_action);
        let send_key = mode
            .as_ref()
            .filter(|m| m.id != "dictation")
            .map(|m| m.send_key.as_str())
            .unwrap_or(&settings.send_key);
        let action = match session_intent {
            crate::dory::SessionIntent::Command => &crate::settings::OutputAction::Paste,
            crate::dory::SessionIntent::Note => &crate::settings::OutputAction::Hud,
            crate::dory::SessionIntent::Assistant
                if mode.as_ref().is_none_or(|m| m.id == "dictation") =>
            {
                &crate::settings::OutputAction::Hud
            }
            _ => normal_action,
        };
        if matches!(
            action,
            crate::settings::OutputAction::Paste | crate::settings::OutputAction::PasteEnter
        ) {
            wait_to_deliver(ctx, session_id, phone_token, settings.paste_delay_ms).await?;
        }
        let insertion = TextInjector::deliver_configured(
            &final_transcript,
            action,
            *ctx.dictation_target_hwnd.read(),
            settings.clipboard_restore_enabled,
            send_key,
            0,
            &settings.inject_method,
        );
        let (outcome, insertion_error) = recover_insertion(insertion);
        ctx.latency_timer.write().injection_finished_at = Some(Instant::now());
        injected = outcome.pasted;
        if outcome.pasted
            && !outcome.fallback_copy
            && insertion_error.is_none()
            && matches!(action, crate::settings::OutputAction::Paste)
            && capture_kind == CaptureKind::Microphone
            && session_intent == crate::dory::SessionIntent::Dictate
            && settings.auto_learn_dictionary
        {
            correction_target = Some(*ctx.dictation_target_hwnd.read());
        }
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
            message: insertion_error.map(|error| format!("Insertion failed: {error}. Your transcript is available in Home; copy it from there.")).unwrap_or_else(|| match action { crate::settings::OutputAction::Copy => "Copied".into(), crate::settings::OutputAction::Hud => "Done".into(), crate::settings::OutputAction::AppendFile { .. } => "Saved to file".into(), crate::settings::OutputAction::RunCommand { .. } => "Command started".into(), _ => injection_message(&InjectionMessageInput {
                silent_mic,
                capture_device: &capture_device,
                transcript_empty: final_transcript.is_empty(),
                fallback_copy: outcome.fallback_copy,
                paste_chord: &outcome.paste_chord,
                rewriter_error: rewriter_error.as_deref(),
                wants_flow,
                rewriter_used,
                asr_warning: asr_warning.as_deref(),
            }) }),
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

    let mut metrics = ctx.latency_timer.read().to_metrics(audio_duration_ms);
    if wants_flow && generation.output_tokens > 0 {
        metrics.llm_generation = Some(generation);
    }
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
    // Prefer real dictation throughput over synthetic warmup when ranking
    // precision choices on the next load. Attribute fallback to its actual rung.
    if metrics.audio_duration_ms > 0 && metrics.rtf > 0.0 && !raw_transcript.trim().is_empty() {
        let engine = ctx.asr_handle.engine_status();
        if let Some((model, _, _)) = ctx.last_asr_load.read().clone() {
            if let Some((device, precision)) = crate::profile::Device::parse(&engine.device)
                .zip(crate::profile::Precision::parse(&engine.precision))
            {
                let caps = crate::capability::capabilities();
                crate::profile::measurements::Measurements::load().record(
                    &caps,
                    &model,
                    device,
                    precision,
                    crate::profile::MeasuredPeaks {
                        rtf: Some(metrics.rtf),
                        vram_mb: (engine.vram_mb > 0.0).then_some(engine.vram_mb),
                        ..Default::default()
                    },
                );
            }
        }
    }
    *ctx.last_latency_metrics.write() = metrics;
    // Persist so `reflow --latency-json` reports real measurements from a
    // separate process instead of inventing them.
    crate::state::persist_latency_report(
        &crate::state::build_latency_report(ctx),
        &ctx.latency_report_path,
    );

    {
        let _storage_policy = ctx.settings_operation.lock();
        let storage_settings = ctx.settings_store.get();
        ensure_session_authority(ctx, session_id, phone_token)?;
        if (storage_settings.history_retention != "disabled"
            || session_intent == crate::dory::SessionIntent::Note)
            && !final_transcript.is_empty()
        {
            ctx.bus.emit(DoryEvent::Stage(Stage::History));
            let session_duration_ms = ctx
                .latency_timer
                .read()
                .recording_started_at
                .map(|t| t.elapsed().as_millis() as u64)
                .unwrap_or(0);
            let entry = HistoryEntry {
                audio_available: false,
                audio_expires_at: None,
                command_input: command_input.clone(),
                kind: match *ctx.session_intent.read() {
                    crate::dory::SessionIntent::Command => "command",
                    crate::dory::SessionIntent::Assistant => "assistant",
                    crate::dory::SessionIntent::Note => "note",
                    _ if capture_kind == CaptureKind::SystemMix => "meeting",
                    _ => "dictation",
                }
                .into(),
                source: if capture_kind == CaptureKind::SystemMix {
                    "meeting"
                } else {
                    "dictation"
                }
                .into(),
                pinned: false,
                tags: String::new(),
                id: Uuid::new_v4().to_string(),
                created_at: recorded_at.to_rfc3339(),
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
                processing_mode: if session_intent == crate::dory::SessionIntent::Assistant {
                    "assistant".into()
                } else if session_intent == crate::dory::SessionIntent::Command {
                    "command".into()
                } else {
                    cleanup_level.clone()
                },
                smart_transcript: smart_transcript.clone(),
                rewriter_used,
            };
            if let Err(error) = ctx.history_store.insert_entry(&entry) {
                ctx.bus.emit(DoryEvent::Error(format!("History could not be saved: {error}. Copy the transcript from Home before closing.")));
            } else {
                correction_history_id = Some(entry.id.clone());
                if complete_audio {
                    if let Err(error) = ctx.history_store.save_audio(&entry.id, &pcm) {
                        log::error!("Could not save dictation audio: {error}");
                        ctx.bus.emit(DoryEvent::Error(format!(
                            "Transcript saved, but audio could not be saved: {error}"
                        )));
                    }
                } else if storage_settings.audio_retention != "disabled" {
                    ctx.bus.emit(DoryEvent::Error(
                        "Transcript saved. Audio exceeded the retention buffer and was not saved."
                            .into(),
                    ));
                }
            }
        }
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
    if session_intent == crate::dory::SessionIntent::Assistant && !final_transcript.is_empty() {
        ctx.bus
            .emit(DoryEvent::AssistantResponse(final_transcript.clone()));
    }
    ctx.bus.emit(DoryEvent::SessionFinished {
        session_id,
        raw: raw_transcript.clone(),
        text: final_transcript.clone(),
        language: language.clone(),
        metrics: ctx.last_latency_metrics.read().clone(),
    });
    reset_ready(ctx);

    if let Some(hwnd) = correction_target {
        crate::correction_observer::start(
            ctx.clone(),
            hwnd,
            final_transcript.clone(),
            correction_history_id,
        );
    }

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
    let Some(session_id) = request_session_cancel(ctx, expected_session) else {
        return Ok(());
    };
    let _operation = ctx
        .session_operation
        .try_lock()
        .map_err(|_| SessionError::busy())?;
    cancel_unlocked(ctx, Some(session_id))
}

pub async fn cancel_owned_wait(
    ctx: &AppContext,
    expected_session: Option<u64>,
) -> Result<(), SessionError> {
    let Some(session_id) = request_session_cancel(ctx, expected_session) else {
        return Ok(());
    };
    let _operation = ctx.session_operation.lock().await;
    let ctx = ctx.clone();
    tokio::task::spawn_blocking(move || cancel_unlocked(&ctx, Some(session_id)))
        .await
        .map_err(|err| SessionError::other(err.to_string()))?
}

fn request_session_cancel(ctx: &AppContext, expected_session: Option<u64>) -> Option<u64> {
    let current = ctx.current_session_id.read();
    if expected_session.is_some() && *current != expected_session {
        return None;
    }
    let session_id = (*current)?;
    if let Some(sender) = ctx.session_cancel.read().as_ref() {
        sender.send_replace(true);
    }
    Some(session_id)
}

fn ensure_session_authority(
    ctx: &AppContext,
    session_id: u64,
    phone_token: Option<&str>,
) -> Result<(), SessionError> {
    if *ctx.current_session_id.read() != Some(session_id)
        || ctx
            .session_cancel
            .read()
            .as_ref()
            .is_some_and(|sender| *sender.borrow())
    {
        return Err(SessionError::other("Dictation canceled"));
    }
    if phone_token.is_some_and(|token| {
        !ctx.settings_store.get().api_enabled
            || !ctx.pairing.permissions(token).is_some_and(|p| p.stream)
    }) {
        return Err(SessionError::other("Phone stream authorization ended"));
    }
    Ok(())
}

async fn wait_to_deliver(
    ctx: &AppContext,
    session_id: u64,
    phone_token: Option<&str>,
    delay_ms: u64,
) -> Result<(), SessionError> {
    if delay_ms > 2000 {
        return Err(SessionError::other("Invalid paste delay"));
    }
    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    ensure_session_authority(ctx, session_id, phone_token)?;
    if phone_token.is_some_and(|token| !ctx.pairing.permissions(token).is_some_and(|p| p.injection))
    {
        return Err(SessionError::other("Phone injection permission ended"));
    }
    Ok(())
}

fn cancel_unlocked(ctx: &AppContext, expected_session: Option<u64>) -> Result<(), SessionError> {
    if expected_session.is_some() && *ctx.current_session_id.read() != expected_session {
        return Ok(());
    }
    let session_id = ctx.current_session_id.write().take().unwrap_or(0);
    ctx.audio_engine.write().stop_capture();
    crate::platform::media::restore();
    *ctx.recording_sample_sender.write() = None;
    *ctx.capture_kind.write() = CaptureKind::None;
    ctx.recording_pcm.lock().clear();
    // Drop the drain waiter: nothing will await it, and leaving it behind
    // would let the next session's stop path resolve against a stale signal.
    ctx.audio_drain_done.lock().take();
    *ctx.dictation_target_hwnd.write() = 0;
    let result = ctx
        .asr_handle
        .cancel_stream_blocking(session_id)
        .map_err(SessionError::other);
    reset_ready(ctx);
    result
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
    ctx.correction_watch_generation
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    *ctx.session_cancel.write() = Some(tokio::sync::watch::channel(false).0);
    *ctx.session_phone_token.write() = None;
    let mut timer = ctx.latency_timer.write();
    timer.reset();
    // Only the press instant is known here. `recording_started_at` is
    // assigned by the caller once the capture stream is actually running;
    // setting both to `now` is what pinned hotkey_to_recording_ms to zero.
    timer.hotkey_pressed_at = Some(pressed_at.unwrap_or_else(Instant::now));
    drop(timer);

    *ctx.dictation_target_hwnd.write() =
        crate::overlay::dictation_target(crate::platform::foreground_hwnd());
    let settings = ctx.settings_store.get();
    let seconds = if settings.audio_retention == "disabled" {
        60
    } else if settings.max_duration_sec == 0 {
        1800
    } else {
        settings.max_duration_sec.clamp(60, 1800)
    };
    *ctx.recording_pcm.lock() = crate::audio::AudioRingBuffer::new(seconds as usize * 16_000);
    *ctx.state_enum.write() = AppStateEnum::Recording;
    *ctx.capture_kind.write() = kind;
    *ctx.last_audio_level.write() = 0.0;
    ctx.bus.emit(DoryEvent::State(AppStateEnum::Recording));
    log::info!("Dictation session started ({:?})", kind);
    Ok(())
}

fn reset_ready(ctx: &AppContext) {
    *ctx.session_cancel.write() = None;
    *ctx.session_phone_token.write() = None;
    crate::platform::media::restore();
    *ctx.current_session_id.write() = None;
    crate::hotkey::hook::reset_mode();
    *ctx.state_enum.write() = AppStateEnum::Ready;
    *ctx.capture_kind.write() = CaptureKind::None;
    *ctx.last_audio_level.write() = 0.0;
    ctx.bus.emit(DoryEvent::State(AppStateEnum::Ready));
}

/// Block the stop path until the sample loop confirms every captured chunk
/// reached the ASR engine, or until `AUDIO_DRAIN_TIMEOUT` elapses.
async fn await_audio_drain(ctx: &AppContext) -> Result<(), SessionError> {
    let Some(rx) = ctx.audio_drain_done.lock().take() else {
        return Err(SessionError::other(
            "Audio capture has no drain confirmation. The recording was not saved; please try again.",
        ));
    };
    match tokio::time::timeout(AUDIO_DRAIN_TIMEOUT, rx).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => {
            log::warn!("Audio drain signal channel closed before completion");
            Err(SessionError::other(
                "Audio capture ended without confirming the complete recording. The recording was not saved; please try again.",
            ))
        }
        Err(_) => {
            log::error!(
                "Audio drain did not complete within {:?}; the tail of this \
                 utterance may be missing",
                AUDIO_DRAIN_TIMEOUT
            );
            Err(SessionError::other(
                "Audio capture did not finish draining. The recording was not saved; please try again.",
            ))
        }
    }
}

fn spawn_sample_loop(ctx: &AppContext, session_id: u64, mut rx_samples: mpsc::Receiver<Vec<f32>>) {
    let state_ref = Arc::clone(&ctx.state_enum);
    let session_ref = Arc::clone(&ctx.current_session_id);
    let cancel_ref = Arc::clone(&ctx.session_cancel);
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
        let owns_session = |active: &Option<u64>| {
            *active == Some(session_id)
                && !cancel_ref
                    .read()
                    .as_ref()
                    .is_some_and(|sender| *sender.borrow())
        };
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
            let rms = if chunk.is_empty() {
                0.0
            } else {
                (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt()
            };
            let level = (rms * 6.0).min(1.0);
            {
                let active = session_ref.read();
                if !owns_session(&active) {
                    break;
                }
                let mut timer = latency_ref.write();
                timer.peak_audio_queue_chunks =
                    timer.peak_audio_queue_chunks.max(rx_samples.len() + 1);
                if first_audio {
                    timer.first_audio_at = Some(Instant::now());
                    first_audio = false;
                }
                drop(timer);
                *level_ref.write() = level;
                pcm_ref.lock().extend_from_slice(&chunk);
                pending_for_asr.extend_from_slice(&chunk);
            }

            // Push to sidecar in ~0.5 s batches.
            if pending_for_asr.len() >= ASR_PUSH_THRESHOLD {
                let samples = std::mem::take(&mut pending_for_asr);
                let current_seq = sequence_id;
                sequence_id += 1;
                let result = asr_handle
                    .push_audio(session_id, current_seq, samples)
                    .await;
                // Keep ownership through every shared mutation/event, but
                // release this synchronous guard before the next async push.
                let active = session_ref.read();
                if !owns_session(&active) {
                    break;
                }
                match result {
                    Ok(Some(text)) if !text.trim().is_empty() => {
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
                    Err(err) => {
                        latency_ref.write().failed_audio_pushes += 1;
                        log::warn!("push_audio failed mid-session: {err}");
                    }
                }
            }

            if last_level_emit.elapsed() >= Duration::from_millis(40) {
                let active = session_ref.read();
                if !owns_session(&active) {
                    break;
                }
                bus.emit(DoryEvent::AudioLevel(level));
                last_level_emit = Instant::now();
            }
        }

        // Drain chunks that raced with stop_capture, including the buffered
        // tail when the capture sender closes while the loop awaits recv().
        while let Ok(chunk) = rx_samples.try_recv() {
            {
                let active = session_ref.read();
                if !owns_session(&active) {
                    break;
                }
                pcm_ref.lock().extend_from_slice(&chunk);
                pending_for_asr.extend_from_slice(&chunk);
            }
        }
        let flush_pending = !pending_for_asr.is_empty() && owns_session(&session_ref.read());
        if flush_pending {
            let samples = std::mem::take(&mut pending_for_asr);
            let current_seq = sequence_id;
            if let Err(err) = asr_handle
                .push_audio(session_id, current_seq, samples)
                .await
            {
                let active = session_ref.read();
                if owns_session(&active) {
                    log::error!("Final audio push failed; tail may be missing: {err}");
                    latency_ref.write().failed_audio_pushes += 1;
                }
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

    #[tokio::test]
    async fn canceled_insertion_delay_never_reaches_the_delivery_boundary() {
        let dir =
            std::env::temp_dir().join(format!("reflow_cancel_delay_{}", uuid::Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        let id = start_external_owned(&ctx, None).await.unwrap();
        let waiting_ctx = ctx.clone();
        let waiting =
            tokio::spawn(async move { wait_to_deliver(&waiting_ctx, id, None, 50).await });
        tokio::time::sleep(Duration::from_millis(10)).await;
        request_session_cancel(&ctx, Some(id));
        let reached_delivery = waiting.await.unwrap().is_ok();
        cancel_owned_wait(&ctx, Some(id)).await.unwrap();
        assert!(!reached_delivery, "Canceled delay must reject delivery");
        drop(ctx);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn revoked_phone_delay_never_reaches_the_delivery_boundary() {
        for disable in [false, true] {
            let dir =
                std::env::temp_dir().join(format!("reflow_revoke_delay_{}", uuid::Uuid::new_v4()));
            let ctx = AppContext::bootstrap_test(dir.clone());
            ctx.settings_store
                .merge_update(serde_json::json!({"api_enabled": true}))
                .unwrap();
            let offer = ctx.pairing.rotate_code();
            let (token, device) = ctx.pairing.pair(&offer.code, "delay phone").unwrap();
            let id = start_phone_owned(&ctx, None, &token).await.unwrap();
            let waiting_ctx = ctx.clone();
            let waiting_token = token.clone();
            let waiting = tokio::spawn(async move {
                wait_to_deliver(&waiting_ctx, id, Some(&waiting_token), 50).await
            });
            tokio::time::sleep(Duration::from_millis(10)).await;
            if disable {
                ctx.settings_store
                    .merge_update(serde_json::json!({"api_enabled": false}))
                    .unwrap();
            } else {
                ctx.pairing.revoke(&device.id).unwrap();
            }
            let reached_delivery = waiting.await.unwrap().is_ok();
            cancel_owned_wait(&ctx, Some(id)).await.unwrap();
            assert!(!reached_delivery, "Unauthorized delay must reject delivery");
            drop(ctx);
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    struct PushProbe {
        pushed_samples: Arc<AtomicUsize>,
        /// Samples the engine had received at the moment `stop_stream()` was
        /// called. This is the number that matters: anything pushed after
        /// `stop_stream()` lands in a reset buffer and is silently lost.
        samples_at_stop: Arc<AtomicUsize>,
        /// Text returned from `push_audio`, simulating an engine that emits
        /// live partials.
        partial_text: Option<String>,
        final_text: String,
        cancel_error: bool,
        push_entered: Option<Arc<tokio::sync::Notify>>,
        push_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
        push_error: bool,
        stop_entered: Option<Arc<tokio::sync::Notify>>,
        stop_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    }

    impl PushProbe {
        fn new(pushed_samples: Arc<AtomicUsize>, samples_at_stop: Arc<AtomicUsize>) -> Self {
            Self {
                pushed_samples,
                samples_at_stop,
                partial_text: None,
                final_text: "probe transcript".into(),
                cancel_error: false,
                push_entered: None,
                push_cancel: None,
                push_error: false,
                stop_entered: None,
                stop_cancel: None,
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
            if let (Some(entered), Some(cancelled)) = (&self.push_entered, &self.push_cancel) {
                entered.notify_one();
                let deadline = Instant::now() + Duration::from_secs(5);
                while !cancelled.load(Ordering::Acquire) && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            self.pushed_samples
                .fetch_add(samples_16k_mono.len(), Ordering::SeqCst);
            if self.push_error {
                Err("delayed audio transfer failed".into())
            } else {
                Ok(self.partial_text.clone())
            }
        }

        fn get_partial_transcript(&mut self) -> Result<String, String> {
            Ok(String::new())
        }

        fn stop_stream(&mut self) -> Result<String, String> {
            if let (Some(entered), Some(cancelled)) = (&self.stop_entered, &self.stop_cancel) {
                entered.notify_one();
                let deadline = Instant::now() + Duration::from_secs(2);
                while !cancelled.load(Ordering::Acquire) && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            self.samples_at_stop
                .store(self.pushed_samples.load(Ordering::SeqCst), Ordering::SeqCst);
            Ok(self.final_text.clone())
        }

        fn cancel_stream(&mut self) -> Result<(), String> {
            if self.cancel_error {
                Err("runtime disconnected".into())
            } else {
                Ok(())
            }
        }

        fn cancellation_signal(&self) -> Option<crate::asr::engine::InferenceCancellation> {
            self.push_cancel
                .as_ref()
                .or(self.stop_cancel.as_ref())
                .map(|signal| {
                    let signal = Arc::clone(signal);
                    Arc::new(move || signal.store(true, Ordering::Release))
                        as crate::asr::engine::InferenceCancellation
                })
        }

        fn get_detected_language(&self) -> String {
            "en".into()
        }

        fn get_backend_name(&self) -> String {
            "push-probe".into()
        }
    }

    #[tokio::test]
    async fn failed_audio_drain_never_publishes_or_saves_an_incomplete_recording() {
        let dir = std::env::temp_dir().join(format!("reflow_drain_failure_{}", Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        let entered = Arc::new(tokio::sync::Notify::new());
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let samples_at_stop = Arc::new(AtomicUsize::new(0));
        let mut probe = PushProbe::new(Arc::new(AtomicUsize::new(0)), Arc::clone(&samples_at_stop));
        probe.push_entered = Some(Arc::clone(&entered));
        probe.push_cancel = Some(Arc::clone(&cancelled));
        ctx.asr_handle.swap_engine(Box::new(probe)).await.unwrap();
        let mut events = ctx.bus.subscribe();
        let id = start_external_owned(&ctx, None).await.unwrap();
        push_f32_owned(&ctx, id, &vec![0.1; 8000]).unwrap();
        tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .expect("the actor must be processing the first audio chunk");
        push_f32_owned(&ctx, id, &[0.2; 100]).unwrap();

        let result = tokio::time::timeout(Duration::from_secs(8), stop_owned(&ctx, false, id))
            .await
            .expect("failed draining must return control to the user");
        let no_history = ctx.history_store.get_entries(10, 0).unwrap().is_empty();
        let mut published = false;
        while let Ok(event) = events.try_recv() {
            published |= matches!(
                event,
                DoryEvent::Final(_) | DoryEvent::SessionFinished { .. }
            );
        }
        let ready = *ctx.state_enum.read() == AppStateEnum::Ready
            && ctx.current_session_id.read().is_none();
        let next = start_external_owned(&ctx, None).await.unwrap();
        cancel_owned_wait(&ctx, Some(next)).await.unwrap();
        drop(ctx);
        let _ = std::fs::remove_dir_all(dir);

        assert!(
            result.is_err(),
            "an unconfirmed audio drain must fail the recording"
        );
        assert!(
            cancelled.load(Ordering::Acquire),
            "the stalled engine must be canceled"
        );
        assert_eq!(samples_at_stop.load(Ordering::SeqCst), 0);
        assert!(no_history, "the incomplete recording must not be saved");
        assert!(
            !published,
            "the incomplete transcript must not reach output"
        );
        assert!(
            ready,
            "a drain failure must leave recording admission available"
        );
    }

    #[tokio::test]
    async fn closed_audio_drain_signal_never_saves_a_recording() {
        let dir = std::env::temp_dir().join(format!("reflow_drain_closed_{}", Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        let id = start_external_owned(&ctx, None).await.unwrap();
        push_f32_owned(&ctx, id, &[0.1; 100]).unwrap();
        let (drain_tx, drain_rx) = tokio::sync::oneshot::channel();
        *ctx.audio_drain_done.lock() = Some(drain_rx);
        drop(drain_tx);

        let result = stop_owned(&ctx, false, id).await;
        let no_history = ctx.history_store.get_entries(10, 0).unwrap().is_empty();
        let ready = *ctx.state_enum.read() == AppStateEnum::Ready;
        drop(ctx);
        let _ = std::fs::remove_dir_all(dir);
        assert!(result.is_err(), "a lost drain task must fail the recording");
        assert!(no_history);
        assert!(ready);
    }

    #[tokio::test]
    async fn delayed_old_audio_reply_cannot_change_a_restarted_session() {
        for (push_error, flush_tail) in [(false, false), (true, false), (false, true), (true, true)]
        {
            let dir = std::env::temp_dir().join(format!("reflow_stale_audio_{}", Uuid::new_v4()));
            let ctx = AppContext::bootstrap_test(dir.clone());
            let entered = Arc::new(tokio::sync::Notify::new());
            let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let mut probe =
                PushProbe::new(Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
            probe.push_entered = Some(Arc::clone(&entered));
            probe.push_cancel = Some(Arc::clone(&cancelled));
            probe.push_error = push_error;
            probe.partial_text = Some("old transcript".into());
            ctx.asr_handle.swap_engine(Box::new(probe)).await.unwrap();
            let old = start_external_owned(&ctx, None).await.unwrap();
            let samples = if flush_tail { 100 } else { 8000 };
            push_f32_owned(&ctx, old, &vec![0.1; samples]).unwrap();
            if flush_tail {
                *ctx.recording_sample_sender.write() = None;
            }
            tokio::time::timeout(Duration::from_secs(2), entered.notified())
                .await
                .expect("the old push must be pending in the actor");
            tokio::time::sleep(Duration::from_millis(50)).await;

            // Keep this current-thread runtime from polling the old reply between
            // cancellation and restart. Both operations still use the real actor.
            cancel_owned(&ctx, Some(old)).unwrap();
            let current = ctx.asr_handle.next_session_id();
            ctx.asr_handle
                .start_stream_blocking(current, "en", &[])
                .unwrap();
            *ctx.current_session_id.write() = Some(current);
            begin_session(&ctx, CaptureKind::External, None).unwrap();
            let mut events = ctx.bus.subscribe();
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;

            let failures = ctx.latency_timer.read().failed_audio_pushes;
            let level = *ctx.last_audio_level.read();
            let pcm_empty = ctx.recording_pcm.lock().is_empty();
            let mut stale_event = false;
            while let Ok(event) = events.try_recv() {
                stale_event |= matches!(event, DoryEvent::Partial(_) | DoryEvent::AudioLevel(_));
            }
            cancel_owned_wait(&ctx, Some(current)).await.unwrap();
            drop(ctx);
            let _ = std::fs::remove_dir_all(dir);

            assert_eq!(
                failures, 0,
                "old transfer errors must not taint the new metrics"
            );
            assert_eq!(
                level, 0.0,
                "old levels must not taint the restarted session"
            );
            assert!(pcm_empty, "old samples must not survive cancellation");
            assert!(
                !stale_event,
                "old replies must not emit new-session UI events"
            );
        }
    }

    #[tokio::test]
    async fn voice_commands_finish_the_session_and_allow_another_recording() {
        for text in ["scratch that", "undo that"] {
            let dir =
                std::env::temp_dir().join(format!("reflow_voice_command_{}", uuid::Uuid::new_v4()));
            let ctx = AppContext::bootstrap_test(dir.clone());
            ctx.settings_store
                .merge_update(serde_json::json!({"voice_commands_enabled": true}))
                .unwrap();
            let mut probe =
                PushProbe::new(Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
            probe.final_text = text.into();
            ctx.asr_handle.swap_engine(Box::new(probe)).await.unwrap();
            let mut events = ctx.bus.subscribe();
            let id = start_external_owned(&ctx, None).await.unwrap();
            *ctx.capture_kind.write() = CaptureKind::Microphone;
            push_f32(&ctx, &vec![0.1; 6400]).unwrap();
            stop_owned(&ctx, false, id).await.unwrap();
            assert_eq!(*ctx.state_enum.read(), AppStateEnum::Ready, "{text}");
            assert!(ctx.current_session_id.read().is_none());
            let mut finished = false;
            while let Ok(event) = events.try_recv() {
                if matches!(event, DoryEvent::SessionFinished { session_id, .. } if session_id == id)
                {
                    finished = true;
                }
            }
            assert!(finished, "The command must finish the active session");
            let next = start_external_owned(&ctx, None).await.unwrap();
            cancel_owned_wait(&ctx, Some(next)).await.unwrap();
            drop(ctx);
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[tokio::test]
    async fn cancellation_failure_still_releases_recording_state() {
        let dir =
            std::env::temp_dir().join(format!("reflow_cancel_failure_{}", uuid::Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        let mut probe =
            PushProbe::new(Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        probe.cancel_error = true;
        ctx.asr_handle.swap_engine(Box::new(probe)).await.unwrap();
        let id = start_external_owned(&ctx, None).await.unwrap();
        assert!(cancel_owned_wait(&ctx, Some(id)).await.is_err());
        assert_eq!(*ctx.state_enum.read(), AppStateEnum::Ready);
        assert!(ctx.current_session_id.read().is_none());
        assert_eq!(*ctx.capture_kind.read(), CaptureKind::None);
        assert!(ctx.recording_sample_sender.read().is_none());
        let next = start_external_owned(&ctx, None).await.unwrap();
        let _ = cancel_owned_wait(&ctx, Some(next)).await;
        drop(ctx);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn cancellation_interrupts_processing_without_saving_its_result() {
        let dir =
            std::env::temp_dir().join(format!("reflow_cancel_processing_{}", uuid::Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        let entered = Arc::new(tokio::sync::Notify::new());
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut probe =
            PushProbe::new(Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        probe.stop_entered = Some(Arc::clone(&entered));
        probe.stop_cancel = Some(Arc::clone(&cancelled));
        ctx.asr_handle.swap_engine(Box::new(probe)).await.unwrap();
        let id = start_external_owned(&ctx, None).await.unwrap();
        push_f32(&ctx, &vec![0.1; 6400]).unwrap();
        let stopping_ctx = ctx.clone();
        let stopping = tokio::spawn(async move { stop_owned(&stopping_ctx, false, id).await });
        entered.notified().await;
        let cancel = tokio::time::timeout(
            Duration::from_millis(500),
            cancel_owned_wait(&ctx, Some(id)),
        )
        .await;
        let interrupted = cancelled.load(Ordering::Acquire);
        cancelled.store(true, Ordering::Release);
        stopping.await.unwrap().unwrap();
        assert!(cancel.is_ok(), "Cancel must not wait behind processing");
        assert!(interrupted, "Cancel must interrupt the active ASR request");
        assert!(ctx.history_store.get_entries(10, 0).unwrap().is_empty());
        assert_eq!(*ctx.state_enum.read(), AppStateEnum::Ready);
        drop(ctx);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn explicit_voice_notes_are_saved_with_dictation_history_disabled() {
        let dir = std::env::temp_dir().join(format!("reflow_voice_note_{}", uuid::Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        ctx.settings_store
            .merge_update(serde_json::json!({"history_retention": "disabled"}))
            .unwrap();
        ctx.asr_handle
            .swap_engine(Box::new(PushProbe::new(
                Arc::new(AtomicUsize::new(0)),
                Arc::new(AtomicUsize::new(0)),
            )))
            .await
            .unwrap();
        let id = start_external_owned(&ctx, None).await.unwrap();
        *ctx.session_intent.write() = crate::dory::SessionIntent::Note;
        push_f32(&ctx, &vec![0.1; 6400]).unwrap();
        let outcome = stop_owned(&ctx, false, id).await.unwrap();
        assert!(!outcome.final_text.is_empty());
        let notes = ctx.history_store.get_entries(10, 0).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].kind, "note");
        drop(ctx);
        let _ = std::fs::remove_dir_all(dir);
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
        assert!(dir.join("latency-report.json").is_file());
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

    /// An engine whose `stop_stream` fails exercises the mid-pipeline error
    /// path: the stop still surfaces the failure and returns to Ready, which
    /// is what keeps the next dictation from being rejected as session_busy.
    struct FailStopEngine;

    impl ASREngine for FailStopEngine {
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

        fn push_audio(&mut self, _samples_16k_mono: &[f32]) -> Result<Option<String>, String> {
            Ok(None)
        }

        fn get_partial_transcript(&mut self) -> Result<String, String> {
            Ok(String::new())
        }

        fn stop_stream(&mut self) -> Result<String, String> {
            Err("sidecar exploded".into())
        }

        fn cancel_stream(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn get_detected_language(&self) -> String {
            "en".into()
        }

        fn get_backend_name(&self) -> String {
            "fail-stop".into()
        }
    }

    #[tokio::test]
    async fn failed_stop_returns_to_ready_and_surfaces_error() {
        let dir = std::env::temp_dir().join(format!("reflow_failstop_{}", uuid::Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        ctx.asr_handle
            .swap_engine(Box::new(FailStopEngine))
            .await
            .unwrap();
        let mut events = ctx.bus.subscribe();
        start_external(&ctx, Some("en".into()))
            .await
            .expect("start");
        // Non-silent audio so the stop path actually calls stop_stream.
        push_f32(&ctx, &vec![0.5; 1600]).expect("push");

        let err = stop(&ctx, false)
            .await
            .expect_err("stop must surface the ASR failure");
        assert_eq!(err.message, "sidecar exploded");
        assert_eq!(*ctx.state_enum.read(), AppStateEnum::Ready);

        let saw_error = std::iter::from_fn(|| events.try_recv().ok())
            .any(|ev| matches!(ev, DoryEvent::Error(_)));
        assert!(saw_error, "failure must be emitted on the bus");

        // A subsequent session is admitted rather than hitting session_busy.
        start_external(&ctx, None)
            .await
            .expect("restart after failed stop");
        cancel(&ctx).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
