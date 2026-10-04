use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

use crate::api::{self, ApiStatus};
use crate::audio::{AudioCaptureEngine, AudioDeviceInfo};
use crate::formatting::{
    format_transcript_ex, CleanupLevel, CustomReplacements, FormatRequest, ReplacementRule,
    VoiceStyle,
};
use crate::history::HistoryEntry;
use crate::hotkey::HotkeyManager;
use crate::injection::TextInjector;
use crate::overlay;
use crate::pairing::PairedDevicePublic;
use crate::platform::{self, PlatformInfo, PlatformSys};
use crate::rewrite::RewriteRequest;
use crate::session;
use crate::settings::{AppSettings, DictionaryTerm};
use crate::state::{AppStateEnum, LatencyMetrics, LatencyReport, ModelStatus, SystemMetrics};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

pub use crate::context::AppContext;

#[cfg(test)]
mod retention_tests;

pub fn spawn_start(app: AppHandle) {
    spawn_start_at(app, Instant::now());
}

/// `pressed_at` must be captured as early as possible in the input path —
/// ideally inside the OS hook callback — so `hotkey_to_recording_ms`
/// includes thread spawn, `SendInput`, IPC and tokio scheduling.
pub fn spawn_start_at(app: AppHandle, pressed_at: Instant) {
    spawn_action(app, "dictate", pressed_at);
}

pub fn spawn_stop(app: AppHandle) {
    spawn_stop_at(app, Instant::now());
}

/// `released_at` is the key-release instant from the input path. It is the
/// true start of `speech_end_to_final_ms`.
pub fn spawn_stop_at(app: AppHandle, released_at: Instant) {
    tauri::async_runtime::spawn(async move {
        let ctx = app.state::<AppContext>().inner().clone();
        if let Err(err) = session::stop_at(&ctx, true, Some(released_at)).await {
            log::error!("stop_recording failed: {}", err.message);
            *ctx.state_enum.write() = AppStateEnum::Ready;
            let _ = app.emit("recording:error", err.message);
            overlay::hide_overlay(&app);
        }
    });
}

pub fn spawn_toggle(app: AppHandle) {
    spawn_toggle_at(app, Instant::now());
}

pub fn spawn_toggle_at(app: AppHandle, event_at: Instant) {
    let ctx = app.state::<AppContext>();
    let state = *ctx.state_enum.read();
    match state {
        AppStateEnum::Recording => spawn_stop_at(app, event_at),
        AppStateEnum::Ready | AppStateEnum::Idle => spawn_start_at(app, event_at),
        _ => {}
    }
}

fn spawn_action(app: AppHandle, action: &str, pressed_at: Instant) {
    let (intent, mode_id) = match action {
        "command" => (crate::dory::SessionIntent::Command, None),
        "assistant" => (crate::dory::SessionIntent::Assistant, None),
        "note" => (crate::dory::SessionIntent::Note, None),
        action if action.starts_with("mode:") => (
            crate::dory::SessionIntent::Dictate,
            Some(action[5..].to_owned()),
        ),
        _ => (crate::dory::SessionIntent::Dictate, None),
    };
    tauri::async_runtime::spawn(async move {
        let ctx = app.state::<AppContext>().inner().clone();
        if let Err(error) = session::start_microphone_with_intent_at(
            &ctx,
            Some(pressed_at),
            intent,
            mode_id.as_deref(),
        )
        .await
        {
            let _ = app.emit("recording:error", error.message);
        }
    });
}

type HotkeyCallback = crate::hotkey::hook::platform::ComboFn;
fn hotkey_dispatcher(
    app: AppHandle,
    action: String,
) -> (HotkeyCallback, HotkeyCallback, HotkeyCallback) {
    // One queue per binding preserves press/release order across slow ASR admission.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(u8, Instant)>();
    tauri::async_runtime::spawn(async move {
        let ctx = app.state::<AppContext>().inner().clone();
        let mut owned = None;
        while let Some((event, at)) = rx.recv().await {
            if event == 1
                || (event == 2 && owned.is_some() && *ctx.current_session_id.read() == owned)
            {
                if let Some(id) = owned.take() {
                    if event == 1 {
                        let _ = session::stop_hotkey_owned_at(&ctx, id, at).await;
                    } else {
                        let _ = session::stop_owned(&ctx, true, id).await;
                    }
                }
                continue;
            }
            if owned.is_some() && *ctx.current_session_id.read() == owned {
                continue;
            }
            let (intent, mode_id) = match action.as_str() {
                "command" => (crate::dory::SessionIntent::Command, None),
                "assistant" => (crate::dory::SessionIntent::Assistant, None),
                "note" => (crate::dory::SessionIntent::Note, None),
                value if value.starts_with("mode:") => {
                    (crate::dory::SessionIntent::Dictate, Some(&value[5..]))
                }
                _ => (crate::dory::SessionIntent::Dictate, None),
            };
            match session::start_microphone_with_intent_at(&ctx, Some(at), intent, mode_id).await {
                Ok(id) => owned = Some(id),
                Err(err) => {
                    owned = None;
                    let _ = app.emit("recording:error", err.message);
                }
            }
        }
    });
    let press = tx.clone();
    let release = tx.clone();
    (
        Arc::new(move |at| {
            let _ = press.send((0, at));
        }),
        Arc::new(move |at| {
            let _ = release.send((1, at));
        }),
        Arc::new(move |at| {
            let _ = tx.send((2, at));
        }),
    )
}

pub fn register_dictation_hotkey(app: &AppHandle, shortcut_str: &str) -> Result<(), String> {
    let mut settings = app.state::<AppContext>().settings_store.get();
    settings.hotkeys.dictation = shortcut_str.into();
    let registry = crate::hotkey::registry::HotkeyRegistry::from_settings(&settings)?;
    // Parse every shortcut before replacing registrations.
    for binding in &registry.bindings {
        if HotkeyManager::modifier_only_flags(&binding.shortcut).is_none() {
            binding
                .shortcut
                .parse::<Shortcut>()
                .map_err(|e| format!("Invalid hotkey {}: {e}", binding.shortcut))?;
        }
    }
    let shortcuts = app.global_shortcut();
    shortcuts.unregister_all().map_err(|e| e.to_string())?;
    crate::hotkey::hook::clear_combo();
    let cancel_app = app.clone();
    crate::hotkey::hook::set_cancel(settings.hotkeys.cancel.as_ref().map(|_| {
        std::sync::Arc::new(move |_| {
            let ctx = cancel_app.state::<AppContext>();
            if *ctx.state_enum.read() == AppStateEnum::Recording {
                let ctx = ctx.inner().clone();
                let id = *ctx.current_session_id.read();
                tauri::async_runtime::spawn(async move {
                    let _ = session::cancel_owned_wait(&ctx, id).await;
                });
            }
        }) as crate::hotkey::hook::platform::ComboFn
    }));
    for binding in registry.bindings {
        let (press, release, toggle) = hotkey_dispatcher(app.clone(), binding.action);
        if let Some(flags) = HotkeyManager::modifier_only_flags(&binding.shortcut) {
            if !crate::hotkey::hook::set_combo(flags, press, release) {
                return Err(format!(
                    "Could not install keyboard hook for {}",
                    binding.shortcut
                ));
            }
        } else {
            let shortcut = binding
                .shortcut
                .parse::<Shortcut>()
                .map_err(|e| e.to_string())?;
            shortcuts
                .on_shortcut(shortcut, move |app, _, event| {
                    let at = Instant::now();
                    let ptt = app.state::<AppContext>().settings_store.get().push_to_talk;
                    match event.state() {
                        ShortcutState::Pressed => {
                            if ptt {
                                press(at);
                            } else {
                                toggle(at);
                            }
                        }
                        ShortcutState::Released => {
                            if ptt {
                                release(at);
                            }
                        }
                    }
                })
                .map_err(|e| format!("Could not register {}: {e}", binding.shortcut))?;
        }
    }

    let ctx = app.state::<AppContext>();
    *ctx.registered_hotkey.write() = shortcut_str.into();
    *ctx.hotkey_error.write() = None;
    Ok(())
}

pub async fn start_recording_inner(_app: AppHandle, ctx: &AppContext) -> Result<(), String> {
    session::start_microphone(ctx).await.map_err(Into::into)
}

pub async fn stop_recording_inner(_app: AppHandle, ctx: &AppContext) -> Result<String, String> {
    session::stop(ctx, true)
        .await
        .map(|outcome| outcome.final_text)
        .map_err(Into::into)
}

#[tauri::command]
pub fn get_app_state(ctx: State<'_, AppContext>) -> AppStateEnum {
    *ctx.state_enum.read()
}

#[tauri::command]
pub fn get_settings(ctx: State<'_, AppContext>) -> AppSettings {
    ctx.settings_store.get()
}

fn history_retention_cleanup(ctx: AppContext) -> impl FnOnce() -> Result<usize, String> {
    move || {
        let _operation = ctx.settings_operation.lock();
        let policy = ctx.settings_store.get().history_retention;
        crate::history::RetentionCleaner::apply_retention(&ctx.history_store, &policy)
    }
}

#[tauri::command]
pub fn update_settings(
    settings: Value,
    app: AppHandle,
    ctx: State<'_, AppContext>,
) -> Result<AppSettings, String> {
    let _settings_operation = ctx.settings_operation.lock();
    if crate::calibration::is_running() {
        return Err("Wait for calibration to finish before changing settings.".into());
    }
    let patch = if let Some(inner) = settings.get("settings") {
        if inner.is_object() {
            inner.clone()
        } else {
            settings
        }
    } else {
        settings
    };
    let gpu_retry_requested = patch
        .get("refinement")
        .and_then(Value::as_object)
        .and_then(|refinement| refinement.get("device"))
        .and_then(Value::as_str)
        .is_some_and(|device| !device.eq_ignore_ascii_case("cpu"));

    let audio_retention_requested = patch.get("audio_retention").is_some();
    let previous = ctx.settings_store.get();
    let updated = ctx.settings_store.merge_update(patch)?;
    if updated.history_encryption != previous.history_encryption {
        if let Err(error) = ctx.history_store.set_encryption(updated.history_encryption) {
            if let Err(rollback) = ctx
                .settings_store
                .merge_update(serde_json::json!({"history_encryption":previous.history_encryption}))
            {
                log::error!("Could not roll back encryption setting: {rollback}");
            }
            return Err(error);
        }
    }
    ctx.history_store
        .require_encryption(updated.history_encryption);
    if let Err(error) =
        crate::network_policy::configure(updated.offline_mode, &PlatformSys::get_app_dir())
    {
        if updated.offline_mode {
            if let Err(stop_error) = ctx.asr_handle.unload_model_blocking() {
                log::error!("Could not stop the speech runtime after policy failure: {stop_error}");
            }
        }
        let _ = app.emit(
            "recording:error",
            format!("Network policy could not be saved for the speech sidecar: {error}"),
        );
    }

    if audio_retention_requested || updated.audio_retention != previous.audio_retention {
        if let Err(error) = ctx
            .history_store
            .set_audio_retention(&updated.audio_retention)
        {
            // Keep the persisted setting consistent when storage cleanup fails.
            if let Err(rollback) = ctx.settings_store.merge_update(serde_json::json!({
                "audio_retention": previous.audio_retention
            })) {
                log::error!("Could not restore the previous audio retention setting: {rollback}");
            }
            return Err(error);
        }
    }
    if updated.asr.runtime != previous.asr.runtime {
        prepare_asr_runtime(ctx.inner(), &app)?;
    }
    let _ = app.emit("settings:changed", &updated);

    if updated.hotkey != previous.hotkey
        || serde_json::to_value(&updated.hotkeys).ok()
            != serde_json::to_value(&previous.hotkeys).ok()
        || serde_json::to_value(&updated.modes).ok() != serde_json::to_value(&previous.modes).ok()
    {
        match register_dictation_hotkey(&app, &updated.hotkey) {
            Ok(()) => {}
            Err(err) => {
                *ctx.hotkey_error.write() = Some(err.clone());
                let _ = app.emit("hotkey:error", err);
            }
        }
    }

    if updated.launch_at_startup != previous.launch_at_startup {
        if let Err(err) = platform::set_launch_at_startup(updated.launch_at_startup) {
            log::warn!("Failed to update autostart: {err}");
        }
    }

    if updated.overlay_position != previous.overlay_position {
        overlay::position_overlay(&app, &updated.overlay_position);
    }

    if updated.api_enabled != previous.api_enabled
        || updated.api_bind != previous.api_bind
        || updated.api_port != previous.api_port
    {
        let ctx_clone = ctx.inner().clone();
        tauri::async_runtime::spawn(async move {
            if let Err(err) = api::sync_server(ctx_clone).await {
                log::error!("Failed to apply LAN API settings: {err}");
            }
        });
    }

    if updated.history_retention != previous.history_retention {
        let cleanup = history_retention_cleanup(ctx.inner().clone());
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(error) = cleanup() {
                log::error!("History retention cleanup failed: {error}");
            }
        });
    }

    if !updated.resolve_intent().run_llm || updated.preset == "fast" {
        // The user turned refinement off entirely; release its memory.
        ctx.flow_runtime.shutdown();
    } else if gpu_retry_requested
        || updated.refinement.gpu_layers != previous.refinement.gpu_layers
        || updated.refinement.device != previous.refinement.device
        || updated.memory_policy.vram_reserve_mb != previous.memory_policy.vram_reserve_mb
    {
        // An explicit GPU click is also a retry signal. This deliberately
        // invalidates a cached CPU fallback so newly freed VRAM is reconsidered
        // on the next inference while successful warm runtimes stay cached.
        ctx.flow_runtime.shutdown();
    }

    // CPU and Vulkan archives share one on-disk binary path. Reconcile an
    // existing CPU/unknown install when GPU is explicitly selected; otherwise
    // the setting can say GPU while the old CPU-only binary keeps running.
    if gpu_retry_requested && !crate::rewrite::runtime_matches(&updated.refinement.device) {
        if let Some(spec) = crate::rewrite::pick_runtime_spec(&updated.refinement.device) {
            if let Err(err) = crate::rewrite::install_runtime(app, ctx.inner().clone(), spec) {
                log::warn!("Could not reconcile refinement runtime: {err}");
            }
        }
    }

    Ok(updated)
}

#[tauri::command]
pub fn get_audio_devices() -> Result<Vec<AudioDeviceInfo>, String> {
    AudioCaptureEngine::list_input_devices()
}

#[tauri::command]
pub fn set_audio_device(device_id: String, ctx: State<'_, AppContext>) -> Result<(), String> {
    let mut current = ctx.settings_store.get();
    current.microphone_device_id = if device_id == "default" {
        None
    } else {
        Some(device_id)
    };
    ctx.settings_store.update(current)?;
    Ok(())
}

#[tauri::command]
pub fn get_current_audio_level(ctx: State<'_, AppContext>) -> f32 {
    let mic = ctx.audio_engine.read().get_current_audio_level();
    if mic > 0.0 {
        mic
    } else {
        *ctx.last_audio_level.read()
    }
}

/// A bounded input-only test. Serializes with dictation; never sends audio to a model.
#[tauri::command]
pub async fn test_microphone(
    ctx: State<'_, AppContext>,
) -> Result<crate::audio::health::MicrophoneHealth, String> {
    let _operation = ctx.session_operation.lock().await;
    if !matches!(
        *ctx.state_enum.read(),
        AppStateEnum::Ready | AppStateEnum::Idle | AppStateEnum::Error
    ) {
        return Err(
            "Finish the current dictation or model operation before testing the microphone.".into(),
        );
    }
    let settings = ctx.settings_store.get();
    let (health, _samples) = crate::audio::preflight::capture_sample(&settings, 3).await?;
    Ok(health)
}

#[tauri::command]
pub async fn test_recognition(
    ctx: State<'_, AppContext>,
) -> Result<crate::audio::preflight::RecognitionTest, String> {
    let _operation = ctx.session_operation.lock().await;
    if !matches!(
        *ctx.state_enum.read(),
        AppStateEnum::Ready | AppStateEnum::Idle
    ) {
        return Err("Finish the current operation before testing recognition.".into());
    }
    let status = ctx.asr_handle.refresh_status().await?;
    if !status.loaded || status.is_loading {
        return Err("Load the speech model before testing recognition.".into());
    }
    let settings = ctx.settings_store.get();
    let (health, samples) = crate::audio::preflight::capture_sample(&settings, 3).await?;
    if health.peak < 0.001 || health.dropped_chunks > 0 {
        return Err(health.assessment);
    }
    let language = if settings.auto_detect_language {
        "auto"
    } else {
        &settings.language
    };
    let id = ctx.asr_handle.next_session_id();
    ctx.asr_handle
        .start_stream(id, language, &session::session_vocabulary(&settings))
        .await?;
    let result = async {
        ctx.asr_handle.push_audio(id, 0, samples).await?;
        ctx.asr_handle.stop_stream(id).await
    }
    .await;
    match result {
        Ok(text) if !text.trim().is_empty() => Ok(crate::audio::preflight::RecognitionTest {
            text,
            language: ctx.asr_handle.get_detected_language(),
            health,
        }),
        Ok(_) => Err("No speech was recognized. Speak clearly and try again.".into()),
        Err(error) => {
            let _ = ctx.asr_handle.cancel_stream(id).await;
            Err(error)
        }
    }
}

#[tauri::command]
pub fn retry_clipboard_restore() -> Result<bool, String> {
    TextInjector::retry_clipboard_restore()
}

#[tauri::command]
pub async fn start_recording(
    app: AppHandle,
    ctx: State<'_, AppContext>,
    intent: Option<crate::dory::SessionIntent>,
    mode_id: Option<String>,
) -> Result<(), String> {
    let ctx = ctx.inner().clone();
    let _ = app;
    session::start_microphone_with_intent_at(
        &ctx,
        None,
        intent.unwrap_or_default(),
        mode_id.as_deref(),
    )
    .await
    .map(|_| ())
    .map_err(Into::into)
}

#[tauri::command]
pub async fn start_meeting(ctx: State<'_, AppContext>) -> Result<(), String> {
    session::start_meeting(ctx.inner())
        .await
        .map(|_| ())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn stop_recording(app: AppHandle, ctx: State<'_, AppContext>) -> Result<String, String> {
    let ctx = ctx.inner().clone();
    stop_recording_inner(app, &ctx).await
}

#[tauri::command]
pub async fn cancel_recording(app: AppHandle, ctx: State<'_, AppContext>) -> Result<(), String> {
    let ctx = ctx.inner().clone();
    let id = *ctx.current_session_id.read();
    session::cancel_owned_wait(&ctx, id)
        .await
        .map_err(Into::<String>::into)?;
    overlay::hide_overlay(&app);
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some("Reflow — Ready"));
    }
    Ok(())
}

#[tauri::command]
pub fn inject_text(text: String, ctx: State<'_, AppContext>) -> Result<bool, String> {
    let settings = ctx.settings_store.get();
    // No target hwnd captured for this manual command — skip the
    // foreground-match verification and let the paste go to whatever is
    // currently focused.
    let outcome = TextInjector::inject(&text, settings.clipboard_restore_enabled, 0)?;
    Ok(outcome.pasted)
}

#[tauri::command]
pub fn query_history(
    query: crate::history::db::HistoryQuery,
    ctx: State<'_, AppContext>,
) -> Result<crate::history::db::HistoryPage, String> {
    ctx.history_store.query_entries(&query)
}

#[tauri::command]
pub async fn export_history(
    query: crate::history::db::HistoryQuery,
    ctx: State<'_, AppContext>,
) -> Result<String, String> {
    let store = Arc::clone(&ctx.history_store);
    tokio::task::spawn_blocking(move || {
        let directory = dirs::download_dir().unwrap_or_else(PlatformSys::get_app_dir);
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let path = directory.join(format!(
            "Reflow-history-{}-{}.json",
            chrono::Local::now().format("%Y%m%d-%H%M%S"),
            uuid::Uuid::new_v4()
        ));
        store.export_json(&query, &path)?;
        Ok(path.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn get_runtime_inventory(
) -> Result<Vec<crate::rewrite::runtime_inventory::RuntimeEntry>, String> {
    tokio::task::spawn_blocking(crate::rewrite::runtime_install::get_runtime_inventory)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rollback_runtime(
    ctx: State<'_, AppContext>,
) -> Result<Vec<crate::rewrite::runtime_inventory::RuntimeEntry>, String> {
    let ctx = ctx.inner().clone();
    tokio::task::spawn_blocking(move || crate::rewrite::runtime_install::rollback_runtime(&ctx))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn repair_runtime(app: AppHandle, ctx: State<'_, AppContext>) -> Result<(), String> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::rewrite::runtime_install::repair_runtime(app, ctx)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn get_history(
    limit: usize,
    offset: usize,
    ctx: State<'_, AppContext>,
) -> Result<Vec<HistoryEntry>, String> {
    ctx.history_store.get_entries(limit, offset)
}

#[tauri::command]
pub fn search_history(
    query: String,
    ctx: State<'_, AppContext>,
) -> Result<Vec<HistoryEntry>, String> {
    ctx.history_store.search_entries(&query)
}

#[tauri::command]
pub fn delete_history_item(id: String, ctx: State<'_, AppContext>) -> Result<bool, String> {
    ctx.history_store.delete_entry(&id)
}

#[tauri::command]
pub fn clear_today_history(ctx: State<'_, AppContext>) -> Result<usize, String> {
    ctx.history_store.clear_today()
}

#[tauri::command]
pub fn clear_all_history(ctx: State<'_, AppContext>) -> Result<usize, String> {
    ctx.history_store.clear_all()
}

#[tauri::command]
pub fn get_dictionary_terms(ctx: State<'_, AppContext>) -> Vec<DictionaryTerm> {
    ctx.settings_store.get().dictionary_terms
}

#[tauri::command]
pub fn save_dictionary_term(
    term: DictionaryTerm,
    ctx: State<'_, AppContext>,
) -> Result<DictionaryTerm, String> {
    let mut current = ctx.settings_store.get();
    let mut item = term;
    if item.id.is_empty() {
        item.id = Uuid::new_v4().to_string();
    }
    current.dictionary_terms.retain(|t| t.id != item.id);
    current.dictionary_terms.push(item.clone());
    ctx.settings_store.update(current)?;
    Ok(item)
}

#[tauri::command]
pub fn delete_dictionary_term(id: String, ctx: State<'_, AppContext>) -> Result<bool, String> {
    let mut current = ctx.settings_store.get();
    current.dictionary_terms.retain(|t| t.id != id);
    ctx.settings_store.update(current)?;
    Ok(true)
}

#[tauri::command]
pub fn get_custom_replacements(ctx: State<'_, AppContext>) -> Vec<ReplacementRule> {
    ctx.settings_store.get().custom_replacements
}

#[tauri::command]
pub fn save_custom_replacement(
    replacement: ReplacementRule,
    ctx: State<'_, AppContext>,
) -> Result<ReplacementRule, String> {
    let mut current = ctx.settings_store.get();
    let mut item = replacement;
    if item.id.is_empty() {
        item.id = Uuid::new_v4().to_string();
    }
    current.custom_replacements.retain(|r| r.id != item.id);
    current.custom_replacements.push(item.clone());
    ctx.settings_store.update(current)?;
    Ok(item)
}

#[tauri::command]
pub fn delete_custom_replacement(id: String, ctx: State<'_, AppContext>) -> Result<bool, String> {
    let mut current = ctx.settings_store.get();
    current.custom_replacements.retain(|r| r.id != id);
    ctx.settings_store.update(current)?;
    Ok(true)
}

#[tauri::command]
pub fn get_model_status(ctx: State<'_, AppContext>) -> ModelStatus {
    let settings = ctx.settings_store.get();
    let active =
        crate::model::manager::runtime_model_id(&settings.asr.model, &settings.asr.runtime);
    let engine_status = ctx.asr_handle.engine_status();
    let mut status = ctx.model_manager.get_status(&engine_status, &active);
    status.asr_selection_notice = ctx.asr_selection_notice.read().clone();
    status
}

/// Resolve which ASR model to actually load, and record why if it is not the
/// one the user picked.
///
/// The user's Settings choice is a ceiling, not a command: a model that does not
/// fit free VRAM used to fall back to the CPU, which measured 17.5s for 7.3s of
/// audio against 2.9s for a smaller model on the GPU. Choosing here — in the one
/// place both the startup load and `reload_model` go through — keeps that
/// decision from drifting between the two.
pub fn resolve_asr_load(ctx: &AppContext) -> Result<(String, String, String), String> {
    let settings = ctx.settings_store.get();
    let requested = settings.asr.model.clone();
    let manager = ctx.model_manager.clone();
    let installed = move |id: &str| manager.is_installed(id);
    let caps = crate::capability::capabilities_uncached();
    // The performance preset now reaches the real load. It used to be read only
    // by `preview_profile`, so choosing "Fast" or "Accurate" in Settings changed
    // what the preview said and nothing else.
    let preset = crate::profile::Preset::parse(&settings.preset);

    let store = crate::profile::measurements::Measurements::load();
    let measured = crate::profile::measurements::lookup_fn(&store, &caps);
    let automatic = preset != crate::profile::Preset::Custom;
    if let Some(choice) = crate::calibration::cached_choice(ctx, &caps, &settings) {
        let fits = crate::profile::asr_manifest(&choice.model).is_some_and(|manifest| {
            if choice.device == "cpu" {
                caps.ram.available_mb > manifest.estimated_cpu_ram_mb() + 1024.0
            } else {
                crate::profile::Precision::parse(&choice.precision).is_some_and(|precision| {
                    caps.free_vram_mb() > manifest.estimated_vram_mb(precision) + 384.0
                })
            }
        });
        if fits {
            *ctx.asr_selection_notice.write() = Some(format!("Using the fastest qualified calibration for {}. Other languages require another sample.", settings.language));
            *ctx.last_asr_load.write() = Some((
                choice.model.clone(),
                choice.device.clone(),
                choice.precision.clone(),
            ));
            return Ok((choice.model, choice.device, choice.precision));
        }
    }
    let selection = crate::profile::select_asr_load_for_runtime(
        &settings.asr.runtime,
        preset,
        &requested,
        if automatic {
            "auto"
        } else {
            &settings.asr.device
        },
        if automatic {
            "auto"
        } else {
            &settings.asr.precision
        },
        &installed,
        &caps,
        &measured,
    );

    // A resolver error means the requested configuration cannot be loaded at
    // all — e.g. a Custom precision the model does not support. Proceeding
    // anyway was the silent CPU-fallback bug; refuse with the detail instead.
    if let Some(err) = &selection.error {
        *ctx.asr_selection_notice.write() = Some(err.detail.clone());
        log::warn!("ASR configuration rejected: {}", err.detail);
        return Err(err.detail.clone());
    }

    match &selection.downgrade {
        Some(reason) => log::warn!("ASR model selection: {reason}"),
        None => log::info!(
            "ASR model selection: {} on {} ({}), preset {}",
            selection.model_id,
            selection.device.as_str(),
            selection.precision.as_str(),
            preset.as_str()
        ),
    }
    *ctx.asr_selection_notice.write() = selection.downgrade.clone();
    *ctx.last_asr_load.write() = Some((
        selection.model_id.to_string(),
        selection.device.as_str().to_string(),
        selection.precision.as_str().to_string(),
    ));

    Ok((
        selection.model_id.to_string(),
        selection.device.as_str().to_string(),
        selection.precision.as_str().to_string(),
    ))
}

fn prepare_asr_runtime(ctx: &AppContext, app: &AppHandle) -> Result<(), String> {
    let requested = ctx.settings_store.get().asr.runtime;
    let mut active = ctx.asr_runtime.lock();
    if *active == requested {
        return Ok(());
    }
    let mut engine: Box<dyn crate::asr::ASREngine> = if requested == "native" {
        Box::new(crate::asr::native::NativeAsrEngine::default())
    } else {
        Box::new(crate::asr::Qwen3AsrSidecar::new())
    };
    if let Ok(dir) = app.path().resource_dir() {
        engine.set_resource_dir(dir);
    }
    ctx.asr_handle.swap_engine_blocking(engine)?;
    *active = requested;
    ctx.asr_handle.initialize_blocking()
}

#[tauri::command]
pub fn get_calibration_status() -> crate::calibration::CalibrationStatus {
    crate::calibration::status()
}
#[tauri::command]
pub async fn run_calibration(
    app: AppHandle,
    ctx: State<'_, AppContext>,
    reference: String,
    language: String,
    seconds: u64,
) -> Result<crate::calibration::CalibrationStatus, String> {
    crate::calibration::run(
        ctx.inner().clone(),
        app.path().resource_dir().ok(),
        reference,
        language,
        seconds,
    )
    .await
}
#[tauri::command]
pub fn cancel_calibration() {
    crate::calibration::cancel();
}
#[tauri::command]
pub fn apply_calibration(
    app: AppHandle,
    ctx: State<'_, AppContext>,
    id: String,
) -> Result<bool, String> {
    let applied = crate::calibration::apply(ctx.inner(), &id)?;
    let _ = app.emit("settings:changed", ctx.settings_store.get());
    Ok(applied)
}

#[tauri::command]
pub fn install_model(
    app: AppHandle,
    model_size: Option<String>,
    ctx: State<'_, AppContext>,
) -> Result<(), String> {
    crate::network_policy::require_online(ctx.settings_store.get().offline_mode)?;
    let _operation = ctx
        .session_operation
        .try_lock()
        .map_err(|_| "Finish the current operation before installing a model.")?;
    let settings = ctx.settings_store.get();
    let active = crate::model::manager::runtime_model_id(
        &model_size.unwrap_or(settings.asr.model),
        &settings.asr.runtime,
    );
    prepare_asr_runtime(ctx.inner(), &app)?;
    let model_dir = ctx.model_manager.get_model_dir(&active);
    let repo = ctx.model_manager.repo_for(&active);
    if active == "phonon-2" {
        let (_, backend, precision) = resolve_asr_load(ctx.inner())?;
        ctx.asr_handle.install_model_dir_with_options_blocking(
            &model_dir.to_string_lossy(),
            repo,
            &backend,
            &precision,
        )?;
    } else {
        ctx.asr_handle
            .install_model_dir_blocking(&model_dir.to_string_lossy(), repo)?;
    }
    spawn_model_status_watch(app, ctx.inner().clone());
    Ok(())
}

#[tauri::command]
pub fn remove_model(model_size: Option<String>, ctx: State<'_, AppContext>) -> Result<(), String> {
    let _operation = ctx
        .session_operation
        .try_lock()
        .map_err(|_| "Finish the current operation before removing a model.")?;
    let settings = ctx.settings_store.get();
    let active = crate::model::manager::runtime_model_id(
        &model_size.unwrap_or(settings.asr.model),
        &settings.asr.runtime,
    );
    let _ = ctx.asr_handle.unload_model_blocking();
    ctx.model_manager.remove_model(&active)
}

#[tauri::command]
pub fn reload_model(app: AppHandle, ctx: State<'_, AppContext>) -> Result<(), String> {
    let _operation = ctx.session_operation.try_lock().map_err(|_| {
        "Wait for the active dictation to finish before reloading models.".to_string()
    })?;
    if !matches!(
        *ctx.state_enum.read(),
        AppStateEnum::Ready | AppStateEnum::Idle
    ) {
        return Err("Wait for the active dictation to finish before reloading models.".into());
    }
    ctx.flow_runtime.shutdown();
    ctx.asr_handle.unload_model_blocking()?;
    prepare_asr_runtime(ctx.inner(), &app)?;
    // Same resolution as the startup load, so a manual reload cannot end up
    // running a different model than a launch would.
    let (model_id, device, precision) = resolve_asr_load(ctx.inner())?;
    let model_dir = ctx.model_manager.get_model_dir(&model_id);
    ctx.asr_handle.load_model_with_precision_blocking(
        &model_dir.to_string_lossy(),
        &device,
        &precision,
    )?;
    spawn_model_status_watch(app, ctx.inner().clone());
    Ok(())
}

/// Emits `model:status` to the UI whenever the sidecar model state changes
/// (loading → ready on GPU, download progress, errors).
///
/// Runs for the life of the app and is idempotent: the first call starts the
/// poller, later calls are no-ops.
///
/// It must not stop. `get_model_status` — and, more importantly, the gate in
/// `session::start_microphone_at` that refuses to record unless the engine
/// reports `loaded` — both read the `AsrHandle` status cache, and this poller
/// is the only thing that ever refreshes it. The previous version gave up after
/// a bounded number of ticks, so when the sidecar was still warming at that
/// point the cache froze on "not loaded" forever: the model finished loading,
/// the UI never noticed, and the hotkey stayed dead for the rest of the
/// session.
pub fn spawn_model_status_watch(app: AppHandle, ctx: AppContext) {
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    // While the model is not ready, poll briskly so the UI converges quickly.
    const POLL_PENDING: Duration = Duration::from_millis(700);
    // Once ready, keep polling so an unload, crash or manual reload is still
    // noticed — but slowly, so a long transcription (which occupies the ASR
    // actor) cannot accumulate a deep backlog of queued status commands.
    const POLL_READY: Duration = Duration::from_secs(10);

    if ctx
        .model_status_watch_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        // A poller is already running; it will pick up the new state.
        return;
    }

    tauri::async_runtime::spawn(async move {
        let mut last: Option<ModelStatus> = None;
        // The load whose measured peaks have already been persisted — a new
        // tuple means a fresh load happened and its warmup RTF/VRAM should be
        // recorded once, not on every poll.
        let mut recorded_load: Option<(String, String, String, u64)> = None;
        loop {
            let (status, engine_status) = {
                let settings = ctx.settings_store.get();
                let active = crate::model::manager::runtime_model_id(
                    &settings.asr.model,
                    &settings.asr.runtime,
                );
                // Model loading happens on the Python sidecar's background
                // thread. The actor cache only changes when a command reaches
                // the engine, so merely reading it here leaves the UI stuck on
                // the initial `loading` snapshot forever. Poll the sidecar and
                // let RefreshStatus update the shared cache before publishing.
                let engine_status = ctx.asr_handle.refresh_status().await.unwrap_or_else(|err| {
                    // Expected while the sidecar warms its native imports and
                    // cannot answer probes at all. Not a fault on its own.
                    log::debug!("Could not refresh ASR model status: {err}");
                    ctx.asr_handle.engine_status()
                });
                (
                    ctx.model_manager.get_status(&engine_status, &active),
                    engine_status,
                )
            };
            let status = {
                let mut status = status;
                status.asr_selection_notice = ctx.asr_selection_notice.read().clone();
                status
            };
            let ready = status.loaded && !status.is_downloading && !status.is_loading;
            let changed = last
                .as_ref()
                .map(|prev| {
                    prev.loaded != status.loaded
                        || prev.is_downloading != status.is_downloading
                        || prev.is_loading != status.is_loading
                        || prev.download_progress_pct != status.download_progress_pct
                        || prev.backend != status.backend
                        || prev.error != status.error
                })
                .unwrap_or(true);
            if changed {
                let _ = app.emit("model:status", status.clone());
                log::info!(
                    "Model status: loaded={} loading={} downloading={} backend={}",
                    status.loaded,
                    status.is_loading,
                    status.is_downloading,
                    status.backend
                );
            }
            // A settled load has measured peaks the resolver can use next
            // time — persist them against the triple that was actually loaded.
            // `load_seconds` disambiguates repeat loads of the same config.
            if ready {
                let resolved = ctx.last_asr_load.read().clone();
                let load_ms = (engine_status.load_seconds * 1000.0) as u64;
                if let Some((model_id, device, precision)) = resolved {
                    let stamp = (model_id.clone(), device.clone(), precision.clone(), load_ms);
                    if recorded_load.as_ref() != Some(&stamp)
                        && (engine_status.warmup_rtf.is_some() || engine_status.vram_mb > 0.0)
                    {
                        let caps = crate::capability::capabilities();
                        // An unparsable triple must be skipped, not guessed —
                        // a mislabeled row (e.g. a GPU rung filed as CPU) is
                        // worse than no measurement at all.
                        let parsed = crate::profile::Device::parse(&engine_status.device)
                            .zip(crate::profile::Precision::parse(&engine_status.precision));
                        if let Some((device_enum, precision_enum)) = parsed {
                            crate::profile::measurements::Measurements::load().record(
                                &caps,
                                &model_id,
                                device_enum,
                                precision_enum,
                                crate::profile::MeasuredPeaks {
                                    vram_mb: (engine_status.vram_mb > 0.0)
                                        .then_some(engine_status.vram_mb),
                                    load_seconds: (engine_status.load_seconds > 0.0)
                                        .then_some(engine_status.load_seconds),
                                    rtf: engine_status.warmup_rtf,
                                    ..Default::default()
                                },
                            );
                            recorded_load = Some(stamp);
                        }
                    }
                }
            }
            last = Some(status);
            tokio::time::sleep(if ready { POLL_READY } else { POLL_PENDING }).await;
        }
    });
}

/// Wait until an in-flight ASR model load has settled — loaded or
/// definitively failed — so resource measurements taken afterwards include
/// the memory it actually allocated.
///
/// `load_model_with_precision` resolves at the sidecar's *ack*, not at
/// completion: the weights land on the GPU on the sidecar's loader thread
/// seconds-to-minutes later (its warm imports alone can run past a minute).
/// The refinement runtime budgets GPU layers from live free VRAM, so calling
/// `FlowRuntime::ensure` in that window over-allocates: llama-server grabs
/// layers the ASR model is about to need, then OOMs or falls back to CPU.
///
/// The settle signal is `EngineStatus.is_loading` going false. Status is
/// refreshed here rather than read from cache, so the wait does not depend
/// on the UI's status poller. While the sidecar is mute inside warm imports
/// the refresh errors and the cache still reads `is_loading`, which is also
/// the truth. The wait is bounded: a sidecar that never settles must not
/// park the caller forever, and proceeding then is no worse than not
/// waiting at all.
pub async fn wait_for_asr_load_settled(asr: &crate::asr::AsrHandle) {
    // Same bound as the sidecar's own `_wait_model_ready` and the benchmark
    // harness's `wait_loaded`.
    const SETTLE_BUDGET: std::time::Duration = std::time::Duration::from_secs(240);
    const POLL: std::time::Duration = std::time::Duration::from_millis(700);
    let started = Instant::now();
    loop {
        let is_loading = match asr.refresh_status().await {
            Ok(status) => status.is_loading,
            Err(_) => asr.engine_status().is_loading,
        };
        if !is_loading {
            return;
        }
        if started.elapsed() >= SETTLE_BUDGET {
            log::warn!("ASR load still unsettled after {SETTLE_BUDGET:?}; proceeding anyway");
            return;
        }
        tokio::time::sleep(POLL).await;
    }
}

#[tauri::command]
pub fn get_latency_metrics(ctx: State<'_, AppContext>) -> LatencyMetrics {
    ctx.last_latency_metrics.read().clone()
}

#[tauri::command]
pub fn get_asr_progress(ctx: State<'_, AppContext>) -> crate::asr::engine::InferenceProgress {
    let progress = ctx.asr_handle.inference_progress();
    if *ctx.state_enum.read() == AppStateEnum::Processing
        && progress.session_id == *ctx.current_session_id.read()
    {
        progress
    } else {
        crate::asr::engine::InferenceProgress::default()
    }
}

/// Everything the developer diagnostics waterfall needs in one round trip:
/// the last dictation, the rolling p50/p95, and the recent samples behind it.
#[tauri::command]
pub fn get_latency_report(ctx: State<'_, AppContext>) -> LatencyReport {
    crate::state::build_latency_report(&ctx)
}

/// Discard the rolling latency window. Used when changing configuration, so
/// percentiles are not mixed across two different profiles.
#[tauri::command]
pub fn reset_latency_history(ctx: State<'_, AppContext>) -> LatencyReport {
    ctx.latency_history.write().clear();
    crate::state::build_latency_report(&ctx)
}

#[tauri::command]
pub fn get_system_metrics() -> SystemMetrics {
    PlatformSys::get_system_metrics()
}

/// The full hardware picture: separate total/used/free VRAM, GPU presence as
/// distinct from CUDA availability, Vulkan, CPU topology and per-runtime RAM.
#[tauri::command]
pub fn get_capabilities() -> crate::capability::Capabilities {
    crate::capability::capabilities()
}

/// Force a fresh probe. Used before a load decision, where a cached free-VRAM
/// reading would be actively misleading.
#[tauri::command]
pub fn refresh_capabilities() -> crate::capability::Capabilities {
    crate::capability::capabilities_uncached()
}

/// A snapshot of what has actually been measured on this machine.
///
/// Fields this process cannot measure — WER needs a labeled corpus, decode
/// speed needs a timed run (the `benchmark::runtime` harness) — are `None`
/// rather than filled with plausible-looking defaults. The old path through
/// `run_synthetic_benchmark` invented them, which presented `0% WER` and a
/// hardcoded `42.5 tok/s` as if they were real results.
fn current_benchmark_report(ctx: &AppContext) -> crate::benchmark::FullBenchmarkReport {
    let status = ctx.asr_handle.engine_status();
    let caps = crate::capability::capabilities();
    let asr = status.loaded.then(|| {
        // The resolved triple is authoritative; the status strings are the
        // fallback for a model loaded before this field existed.
        let (model_id, device, precision) = ctx.last_asr_load.read().clone().unwrap_or_else(|| {
            (
                status.backend.clone(),
                status.device.clone(),
                status.precision.clone(),
            )
        });
        crate::benchmark::AsrBenchmarkMetrics {
            model_id,
            device,
            precision,
            load_ms: (status.load_seconds * 1000.0) as u64,
            warmup_rtf: status.warmup_rtf.map(|r| r as f64),
            average_inference_ms: None,
            corpus_wer: None,
            corpus_cer: None,
            spill_detected: status.spill_detected,
        }
    });
    let refinement = ctx.flow_runtime.active_model().map(|model_id| {
        crate::benchmark::RefinementBenchmarkMetrics {
            model_id,
            device: ctx
                .flow_runtime
                .active_mode()
                .map(|m| m.backend_label())
                .unwrap_or_else(|| "unknown".into()),
            load_ms: None,
            tokens_per_second: None,
            safety_pass_rate: None,
            average_latency_ms: None,
        }
    });
    let hardware_summary = format!(
        "{} / {}",
        caps.primary_gpu()
            .map(|g| g.name.as_str())
            .unwrap_or("no discrete GPU"),
        caps.cpu.model
    );
    crate::benchmark::FullBenchmarkReport {
        timestamp: chrono::Utc::now().to_rfc3339(),
        asr,
        refinement,
        hardware_summary,
    }
}

#[tauri::command]
pub fn get_benchmark_report(ctx: State<'_, AppContext>) -> crate::benchmark::FullBenchmarkReport {
    crate::benchmark::load_benchmark_report()
        .unwrap_or_else(|| current_benchmark_report(ctx.inner()))
}

#[tauri::command]
pub fn run_system_benchmark(
    ctx: State<'_, AppContext>,
) -> Result<crate::benchmark::FullBenchmarkReport, String> {
    let report = current_benchmark_report(ctx.inner());
    crate::benchmark::persist_benchmark_report(&report);
    Ok(report)
}

/// Every declared model and runtime, for the settings and download UI.
#[tauri::command]
pub fn get_model_manifests() -> Vec<crate::profile::ModelManifest> {
    crate::profile::all_manifests().cloned().collect()
}

/// Resolve a configuration for the current hardware without applying it.
///
/// This is what the settings page calls to preview a preset, so the user sees
/// the resolved configuration and the reasons behind it before committing.
#[tauri::command]
pub fn preview_profile(
    preset: String,
    overrides: Option<crate::profile::ProfileOverrides>,
) -> crate::profile::ResolvedProfile {
    let caps = crate::capability::capabilities_uncached();
    let store = crate::profile::measurements::Measurements::load();
    let measured = crate::profile::measurements::lookup_fn(&store, &caps);
    crate::profile::resolve_profile(
        &caps,
        crate::profile::Preset::parse(&preset),
        &overrides.unwrap_or_default(),
        &measured,
    )
}

#[tauri::command]
pub fn open_logs_folder() -> Result<(), String> {
    let logs_dir = PlatformSys::get_logs_dir();
    let _ = std::fs::create_dir_all(&logs_dir);
    platform::open_path(&logs_dir)
}

#[tauri::command]
pub fn get_runtime_plan(ctx: State<'_, AppContext>) -> crate::runtime::RuntimePlan {
    crate::runtime::current_plan(ctx.inner())
}

#[tauri::command]
pub fn get_diagnostics_report() -> String {
    PlatformSys::generate_diagnostics_report()
}

#[tauri::command]
pub fn get_platform_info(ctx: State<'_, AppContext>) -> PlatformInfo {
    platform::platform_info(ctx.hotkey_error.read().clone())
}

#[tauri::command]
pub fn get_api_status(ctx: State<'_, AppContext>) -> ApiStatus {
    api::current_status(ctx.inner())
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub fn rotate_pairing_code(ctx: State<'_, AppContext>) -> ApiStatus {
    let _ = ctx.pairing.rotate_code();
    api::current_status(ctx.inner())
}

#[tauri::command]
pub fn list_api_devices(ctx: State<'_, AppContext>) -> Vec<PairedDevicePublic> {
    ctx.pairing.list_public()
}

#[tauri::command]
pub fn set_api_device_permissions(
    id: String,
    permissions: crate::pairing::DevicePermissions,
    ctx: State<'_, AppContext>,
) -> Result<bool, String> {
    ctx.pairing.set_permissions(&id, permissions)
}

#[tauri::command]
pub fn revoke_api_device(id: String, ctx: State<'_, AppContext>) -> Result<bool, String> {
    ctx.pairing.revoke(&id)
}

#[derive(serde::Serialize)]
pub struct FlowStatus {
    pub active_tier: String,
    pub active_model: String,
    /// `true` when the GGUF weights for the active flow model are present on
    /// disk. This is the source of truth for the "Installed" badge in the UI.
    /// The `llama-server` runtime is a separate, optional binary tracked by
    /// `runtime_installed`.
    pub installed: bool,
    /// `true` when the `llama-server` binary that the GGUF needs in order to
    /// actually run is on disk. The frontend should show a clear "runtime
    /// missing" hint when this is `false` but `installed` is `true`.
    pub runtime_installed: bool,
    pub ready: bool,
    pub backend: String,
    pub is_loading: bool,
    pub is_downloading: bool,
    pub download_progress_pct: u32,
    /// Effective execution mode of the running `llama-server` child.
    /// `"cpu"` or `"gpu"`; `None` when the runtime is shut down.
    pub mode: Option<String>,
    /// The actual `--n-gpu-layers` value the running child was launched with.
    pub n_gpu_layers: Option<u32>,
    /// Approximate VRAM in use, in MB, polled from `nvidia-smi`.
    pub vram_used_mb: f32,
    /// Last `ensure()` error, or `None` if the runtime is healthy. The
    /// frontend surfaces this as a one-line hint next to the model badge
    /// with a "Reinstall runtime" action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

fn tier_to_flow_model(tier: &str) -> &'static str {
    crate::settings::flow_model_for_tier(tier)
}

fn flow_model_to_tier(model: &str) -> &'static str {
    match model {
        "qwen3.5-2b" => "deep_context",
        "none" => "raw_verbatim",
        _ => "smart_flow",
    }
}

fn build_flow_status(ctx: &AppContext) -> FlowStatus {
    let settings = ctx.settings_store.get();
    let active_model = settings.flow_model.clone();
    let active_tier = if settings.intelligence_tier.is_empty() {
        flow_model_to_tier(&active_model).to_string()
    } else {
        settings.intelligence_tier.clone()
    };
    let gguf = crate::rewrite::flow_gguf_path(&active_model);
    let bin = crate::rewrite::llama_server_bin();
    let gguf_exists = gguf.exists();
    let bin_exists = bin.exists();
    let ready = ctx.flow_runtime.status_ready();
    // Surface the actual mode the running llama-server was started with,
    // not just whether a GPU is present on the system.
    let active_mode = ctx.flow_runtime.active_mode();
    let backend = active_mode
        .as_ref()
        .map(|m| m.backend_label())
        .unwrap_or_default();
    let mode = active_mode.as_ref().map(|m| match m {
        crate::rewrite::LlamaMode::Cpu => "cpu",
        crate::rewrite::LlamaMode::Gpu(_) => "gpu",
    });
    let n_gpu_layers = ctx.flow_runtime.active_n_gpu_layers();
    let vram_used_mb = if active_mode.as_ref().map(|m| m.is_gpu()).unwrap_or(false) {
        crate::platform::PlatformSys::detect_gpu().1
    } else {
        0.0
    };
    let any_active = ctx
        .active_intelligence_downloads
        .lock()
        .contains(&active_tier);
    let runtime_active = ctx
        .active_runtime_downloads
        .lock()
        .contains(crate::rewrite::runtime_install::RUNTIME_LOCK_KEY);
    FlowStatus {
        active_tier,
        active_model,
        installed: gguf_exists,
        runtime_installed: bin_exists,
        ready,
        backend,
        is_loading: ctx.flow_runtime.is_starting() || runtime_active,
        is_downloading: any_active || runtime_active,
        download_progress_pct: 0,
        mode: mode.map(|s| s.to_string()),
        n_gpu_layers,
        vram_used_mb,
        last_error: ctx.flow_runtime.last_error(),
    }
}

#[tauri::command]
pub fn get_flow_status(ctx: State<'_, AppContext>) -> FlowStatus {
    build_flow_status(ctx.inner())
}

#[tauri::command]
pub fn get_intelligence_status(ctx: State<'_, AppContext>) -> FlowStatus {
    build_flow_status(ctx.inner())
}

#[derive(serde::Serialize)]
pub struct IntelligenceTierState {
    pub tier: String,
    pub model_id: String,
    /// `true` when both the GGUF weights for this tier AND the
    /// `llama-server` runtime are on disk. This is the source of truth
    /// for the "Installed" badge in the UI. The frontend can also
    /// distinguish the two states by looking at `get_intelligence_status`
    /// → `runtime_installed`.
    pub installed: bool,
    pub weights_installed: bool,
    pub downloading: bool,
}

#[tauri::command]
pub fn get_intelligence_tiers(ctx: State<'_, AppContext>) -> Vec<IntelligenceTierState> {
    let active = ctx.active_intelligence_downloads.lock().clone();
    let runtime_present = crate::rewrite::llama_server_bin().exists();
    [
        ("smart_flow", "qwen3.5-0.8b"),
        ("deep_context", "qwen3.5-2b"),
    ]
    .iter()
    .map(|(tier, model_id)| {
        let gguf_path = crate::rewrite::flow_gguf_path(model_id);
        IntelligenceTierState {
            tier: (*tier).to_string(),
            model_id: (*model_id).to_string(),
            installed: gguf_path.exists() && runtime_present,
            weights_installed: gguf_path.exists(),
            downloading: active.contains(*tier),
        }
    })
    .collect()
}

#[tauri::command]
pub fn set_intelligence_tier(
    tier: String,
    ctx: State<'_, AppContext>,
) -> Result<AppSettings, String> {
    let normalized = tier.trim().to_ascii_lowercase();
    let valid = matches!(
        normalized.as_str(),
        "raw_verbatim" | "smart_flow" | "deep_context"
    );
    if !valid {
        return Err(format!("Unknown intelligence tier: {tier}"));
    }
    let flow_model = tier_to_flow_model(&normalized).to_string();
    let current = ctx.settings_store.get();
    // The three settings fields are independent now: do not clobber the
    // user's existing `cleanup_level` when they switch tier. The effective
    // cleanup level is resolved at read time via `AppSettings::resolve_intent`.
    let new_settings = AppSettings {
        intelligence_tier: normalized.clone(),
        flow_model: flow_model.clone(),
        ..current
    };
    let updated = ctx.settings_store.update(new_settings)?;
    if normalized == "raw_verbatim" {
        ctx.flow_runtime.shutdown();
    }
    Ok(updated)
}

/// `true` when Stage 2 polishing is currently on.
#[tauri::command]
pub fn get_polish_enabled(ctx: State<'_, AppContext>) -> bool {
    let settings = ctx.settings_store.get();
    settings.preset != "fast" && settings.resolve_intent().run_llm
}

/// Turn Stage 2 polishing on or off, remembering the tier to come back to.
///
/// This is the "fast now, polished later" switch. It persists like any other
/// setting, so the choice survives a restart, and `session::stop_at` re-reads
/// settings on every dictation, so it takes effect on the very next utterance
/// with no reload.
///
/// Turning it on also starts the runtime immediately instead of leaving the cost
/// for the first dictation: a cold `llama-server` is ~2-5s and the shared prompt
/// prefix costs seconds more on CPU, and paying that while the user waits for
/// their first transcript is exactly the latency this is meant to avoid.
#[tauri::command]
pub fn set_polish_enabled(
    enabled: bool,
    app: AppHandle,
    ctx: State<'_, AppContext>,
) -> Result<AppSettings, String> {
    let current = ctx.settings_store.get();
    let current_tier = current.resolve_intent().tier;

    let (tier, last_polish_tier) = if enabled {
        // Restore whichever polishing tier was last in use.
        let restore = if current
            .last_polish_tier
            .trim()
            .eq_ignore_ascii_case("raw_verbatim")
            || current.last_polish_tier.trim().is_empty()
        {
            "smart_flow".to_string()
        } else {
            current.last_polish_tier.trim().to_ascii_lowercase()
        };
        (restore.clone(), restore)
    } else {
        // Remember where to come back to before erasing it.
        let remember = if current_tier == "raw_verbatim" {
            current.last_polish_tier.clone()
        } else {
            current_tier
        };
        ("raw_verbatim".to_string(), remember)
    };

    let updated = ctx.settings_store.update(AppSettings {
        intelligence_tier: tier,
        last_polish_tier,
        preset: if enabled && current.preset == "fast" {
            "auto".into()
        } else {
            current.preset.clone()
        },
        cleanup_level: if enabled && current.cleanup_level == "raw" {
            "medium".into()
        } else {
            current.cleanup_level.clone()
        },
        ..current
    })?;

    let intent = updated.resolve_intent();
    if intent.run_llm && updated.refinement.keep_warm {
        let ctx_flow = ctx.inner().clone();
        tauri::async_runtime::spawn(async move {
            // If an ASR load is still in flight, its VRAM is not in the free
            // figure yet — wait for it to settle before budgeting GPU layers.
            // Returns immediately when nothing is loading.
            wait_for_asr_load_settled(&ctx_flow.asr_handle).await;
            let _ = tokio::task::spawn_blocking(move || {
                let effective = crate::runtime::current_settings(&ctx_flow);
                let intent = effective.resolve_intent();
                if !intent.run_llm || !effective.refinement.keep_warm {
                    return;
                }
                let flow_model = intent.flow_model;
                let backend = effective.refinement.device;
                let override_layers = (effective.refinement.gpu_layers >= 0)
                    .then_some(effective.refinement.gpu_layers.max(0) as u32);
                let runtime = &ctx_flow.flow_runtime;
                match runtime.ensure(
                    &flow_model,
                    &backend,
                    override_layers,
                    effective.memory_policy.vram_reserve_mb,
                    effective.refinement.context_size,
                ) {
                    Ok(()) => {
                        runtime.set_deadline_ms(effective.refinement.deadline_ms);
                        runtime.warm_prompt_cache(&flow_model)
                    }
                    Err(err) => log::warn!("Could not start the polish runtime: {err}"),
                }
            })
            .await;
        });
    } else {
        // Free the VRAM and the process straight away; the user asked for speed.
        ctx.flow_runtime.shutdown();
    }

    let _ = app.emit("settings:changed", updated.clone());
    log::info!(
        "Polish {} (tier={}, restore={})",
        if enabled { "enabled" } else { "disabled" },
        updated.intelligence_tier,
        updated.last_polish_tier
    );
    Ok(updated)
}

#[tauri::command]
pub fn install_intelligence_model(
    tier: String,
    app: AppHandle,
    ctx: State<'_, AppContext>,
) -> Result<(), String> {
    crate::network_policy::require_online(ctx.settings_store.get().offline_mode)?;
    use std::time::Instant;
    let normalized = tier.trim().to_ascii_lowercase();
    let spec = match normalized.as_str() {
        "smart_flow" => crate::rewrite::server::flow_model_spec("qwen3.5-0.8b"),
        "deep_context" => crate::rewrite::server::flow_model_spec("qwen3.5-2b"),
        _ => return Err(format!("Tier '{tier}' has no model to install")),
    };
    let manifest = crate::profile::manifest::refinement_manifest(spec.id)
        .ok_or("Refinement manifest missing")?;
    let final_dest = crate::rewrite::flow_gguf_path(spec.id);
    let dest = final_dest.with_extension("gguf.part");
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let url = format!(
        "https://huggingface.co/{}/resolve/{}/{}",
        manifest.repo, manifest.revision, manifest.filename
    );

    // Re-entry guard: refuse to start a second concurrent download for the same tier.
    // The button can be clicked again while a download is in progress (e.g. after
    // the app regains focus). Without this, two threads race on the same file and
    // the second one truncates the partial work back to 0 bytes.
    let mut active = ctx.active_intelligence_downloads.lock();
    if active.contains(&normalized) {
        return Err(format!(
            "A download for '{normalized}' is already in progress"
        ));
    }
    active.insert(normalized.clone());
    drop(active);

    let app_handle = app.clone();
    let filename = spec.filename.to_string();
    let tier_label = normalized.clone();
    let ctx_for_thread = ctx.inner().clone();
    std::thread::spawn(move || {
        let _ = app_handle.emit(
            "intelligence:download-progress",
            serde_json::json!({
                "tier": tier_label,
                "progress_pct": 0,
                "speed_mbps": 0.0,
                "phase": "starting",
            }),
        );
        let client = match reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60 * 30))
            .redirect(crate::network_policy::redirect_policy())
            .build()
        {
            Ok(c) => c,
            Err(err) => {
                emit_error(&app_handle, &tier_label, 0, err.to_string());
                clear_active(&ctx_for_thread, &tier_label);
                return;
            }
        };

        // Resume from the existing file size if a partial file is on disk.
        let resume_from: u64 = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
        let mut request = client.get(&url);
        if resume_from > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
        }

        let started = Instant::now();
        let mut last_emit = Instant::now();
        let mut response = match crate::network_policy::send_download(request) {
            Ok(r) => r,
            Err(err) => {
                emit_error(
                    &app_handle,
                    &tier_label,
                    pct_from(resume_from, spec.approx_bytes),
                    err.to_string(),
                );
                clear_active(&ctx_for_thread, &tier_label);
                return;
            }
        };
        let status = response.status();
        // If the server doesn't honor Range, it replies 200 and we restart from zero.
        let already_have: u64 = if status == reqwest::StatusCode::PARTIAL_CONTENT {
            resume_from
        } else if status.is_success() {
            // Server ignored Range; discard any partial file and start over.
            let _ = std::fs::remove_file(&dest);
            0
        } else {
            emit_error(
                &app_handle,
                &tier_label,
                pct_from(resume_from, spec.approx_bytes),
                format!("HTTP {status}"),
            );
            clear_active(&ctx_for_thread, &tier_label);
            return;
        };
        // content_length is the number of bytes remaining when resuming, or the full size otherwise.
        let remaining = response
            .content_length()
            .unwrap_or(spec.approx_bytes.saturating_sub(already_have));
        let total = already_have + remaining;

        // Open the destination without truncating so we append to the existing partial file.
        use std::io::{Read, Seek, SeekFrom, Write};
        let mut dest_file = match std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&dest)
        {
            Ok(f) => f,
            Err(err) => {
                emit_error(
                    &app_handle,
                    &tier_label,
                    pct_from(already_have, total),
                    err.to_string(),
                );
                clear_active(&ctx_for_thread, &tier_label);
                return;
            }
        };
        if let Err(err) = dest_file.seek(SeekFrom::Start(already_have)) {
            emit_error(
                &app_handle,
                &tier_label,
                pct_from(already_have, total),
                err.to_string(),
            );
            clear_active(&ctx_for_thread, &tier_label);
            return;
        }

        // Emit the current state so the UI doesn't display 0% if we're resuming.
        if already_have > 0 {
            let elapsed = started.elapsed().as_secs_f32().max(0.001);
            let speed_mbps = (already_have as f32 / 1_048_576.0) / elapsed;
            let _ = app_handle.emit(
                "intelligence:download-progress",
                serde_json::json!({
                    "tier": tier_label,
                    "progress_pct": pct_from(already_have, total),
                    "speed_mbps": speed_mbps,
                    "phase": "downloading",
                }),
            );
            last_emit = Instant::now();
        }

        let mut downloaded: u64 = already_have;
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            match response.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if let Err(err) = dest_file.write_all(&buffer[..n]) {
                        emit_error(
                            &app_handle,
                            &tier_label,
                            pct_from(downloaded, total),
                            err.to_string(),
                        );
                        // Keep the partial file so the next attempt can resume it.
                        clear_active(&ctx_for_thread, &tier_label);
                        return;
                    }
                    downloaded += n as u64;
                    if last_emit.elapsed() >= std::time::Duration::from_millis(250) {
                        let elapsed = started.elapsed().as_secs_f32().max(0.001);
                        let speed_mbps =
                            ((downloaded - already_have) as f32 / 1_048_576.0) / elapsed;
                        let pct = pct_from(downloaded, total);
                        let _ = app_handle.emit(
                            "intelligence:download-progress",
                            serde_json::json!({
                                "tier": tier_label,
                                "progress_pct": pct,
                                "speed_mbps": speed_mbps,
                                "phase": "downloading",
                            }),
                        );
                        last_emit = Instant::now();
                    }
                }
                Err(err) => {
                    emit_error(
                        &app_handle,
                        &tier_label,
                        pct_from(downloaded, total),
                        err.to_string(),
                    );
                    // Keep the partial file so the next attempt can resume it.
                    clear_active(&ctx_for_thread, &tier_label);
                    return;
                }
            }
        }
        let verification = dest_file
            .flush()
            .map_err(|e| e.to_string())
            .and_then(|_| crate::rewrite::runtime_install::verify_sha256(&dest, manifest.sha256));
        drop(dest_file);
        if let Err(err) = verification {
            let _ = std::fs::remove_file(&dest);
            emit_error(
                &app_handle,
                &tier_label,
                100,
                format!("Model SHA-256 verification failed: {err}"),
            );
            clear_active(&ctx_for_thread, &tier_label);
            return;
        }
        let finalize = if final_dest.exists() {
            std::fs::remove_file(&final_dest)
        } else {
            Ok(())
        }
        .and_then(|_| std::fs::rename(&dest, &final_dest));
        if let Err(err) = finalize {
            emit_error(&app_handle, &tier_label, 100, err.to_string());
            clear_active(&ctx_for_thread, &tier_label);
            return;
        }
        let _ = app_handle.emit(
            "intelligence:download-progress",
            serde_json::json!({
                "tier": tier_label,
                "progress_pct": 100,
                "speed_mbps": 0.0,
                "phase": "complete",
                "filename": filename,
            }),
        );
        clear_active(&ctx_for_thread, &tier_label);
    });
    Ok(())
}

fn pct_from(downloaded: u64, total: u64) -> u32 {
    (downloaded * 100 / total.max(1)) as u32
}

fn emit_error(app: &AppHandle, tier: &str, progress_pct: u32, error: String) {
    let _ = app.emit(
        "intelligence:download-progress",
        serde_json::json!({
            "tier": tier,
            "progress_pct": progress_pct,
            "speed_mbps": 0.0,
            "phase": "error",
            "error": error,
        }),
    );
}

fn clear_active(ctx: &AppContext, tier: &str) {
    let mut active = ctx.active_intelligence_downloads.lock();
    active.remove(tier);
}

#[tauri::command]
pub fn remove_intelligence_model(tier: String, ctx: State<'_, AppContext>) -> Result<(), String> {
    let normalized = tier.trim().to_ascii_lowercase();
    let flow_id = match normalized.as_str() {
        "smart_flow" => "qwen3.5-0.8b",
        "deep_context" => "qwen3.5-2b",
        _ => return Err(format!("Tier '{tier}' has no model to remove")),
    };
    let path = crate::rewrite::flow_gguf_path(flow_id);
    let active = ctx.settings_store.get();
    if active.flow_model == flow_id {
        ctx.flow_runtime.shutdown();
    }
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("Failed to remove model: {e}"))?;
    }
    Ok(())
}

/// Install (or re-install) the `llama-server` runtime binary.
///
/// Picks a build from the pinned `ggml-org/llama.cpp` release based on
/// the user's `compute_backend` setting and the GPU presence on the
/// machine (Vulkan on a GPU box, CPU otherwise). The download and
/// extraction happens in a background thread; progress is delivered via
/// the `runtime:download-progress` Tauri event.
#[tauri::command]
pub fn install_llama_runtime(
    compute_backend: String,
    app: AppHandle,
    ctx: State<'_, AppContext>,
) -> Result<(), String> {
    crate::network_policy::require_online(ctx.settings_store.get().offline_mode)?;
    let spec = crate::rewrite::pick_runtime_spec(&compute_backend)
        .ok_or_else(|| "This platform has no prebuilt llama-server runtime".to_string())?;
    crate::rewrite::install_runtime(app, ctx.inner().clone(), spec)
}

/// Remove the on-disk `llama-server` runtime, if any. Idempotent and
/// safe to call when no runtime is installed. Shuts the runtime down
/// first so the next dictation cleanly fails over to the "no runtime"
/// error path.
#[tauri::command]
pub fn remove_llama_runtime(ctx: State<'_, AppContext>) -> Result<(), String> {
    if ctx.asr_runtime.lock().as_str() == "native" {
        ctx.asr_handle.unload_model_blocking()?;
    }
    ctx.flow_runtime.shutdown();
    let bin = crate::rewrite::llama_server_bin();
    if bin.exists() {
        // Refuse to delete a path the user pointed at via env var —
        // they probably want to keep their custom build.
        if std::env::var(crate::rewrite::runtime_install::ENV_OVERRIDE_BIN).is_ok() {
            return Ok(());
        }
        std::fs::remove_file(&bin)
            .map_err(|e| format!("Could not remove llama-server binary: {e}"))?;
        let marker = crate::rewrite::runtime_kind_path();
        if marker.exists() {
            std::fs::remove_file(marker)
                .map_err(|e| format!("Could not remove runtime kind marker: {e}"))?;
        }
    }
    Ok(())
}

/// Result of the format + optional LLM-polish pipeline. Shared by
/// `preview_tier_cleanup` and `retry_history_transcript`.
struct CleanupRun {
    smart: String,
    final_text: String,
    rewriter_used: bool,
    model_id: String,
    tier_label: String,
    latency_ms: u64,
    rewriter_error: Option<String>,
}

/// `focused` is the process name replacements/polish should target;
/// `""`/`"preview"` when there is no real foreground app.
fn run_cleanup_pipeline(
    text: &str,
    tier: Option<String>,
    style: Option<String>,
    focused: &str,
    app: &AppHandle,
    ctx: &AppContext,
) -> Result<CleanupRun, String> {
    let tier_label = tier
        .as_deref()
        .map(|t| t.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "smart_flow".into());
    let model_id = match tier_label.as_str() {
        "deep_context" => "qwen3.5-2b",
        "raw_verbatim" => "none",
        _ => "qwen3.5-0.8b",
    };
    let mut settings = ctx.settings_store.get();
    settings.intelligence_tier = tier_label.clone();
    settings.flow_model = model_id.to_string();
    settings.style = style.unwrap_or_else(|| "neutral".into());
    let caps = crate::capability::capabilities();
    let measurements = crate::profile::measurements::Measurements::load();
    let measured = crate::profile::measurements::lookup_fn(&measurements, &caps);
    let plan = crate::runtime::plan(
        &settings,
        &caps,
        &|id| ctx.model_manager.is_installed(id),
        &measured,
        ctx.asr_handle.engine_status().loaded,
        crate::runtime::refinement_is_resident(ctx, &settings),
    );
    settings = crate::runtime::effective_settings(&settings, &plan);
    let replacement_rules = CustomReplacements::new(settings.custom_replacements.clone());
    let level = CleanupLevel::parse(&settings.resolved_cleanup_level());
    let mut smart = format_transcript_ex(
        text,
        FormatRequest {
            cleanup_level: level,
            dictation_mode: &settings.dictation_mode,
            style: VoiceStyle::parse(&settings.style),
            filler_removal_enabled: settings.filler_removal_enabled,
            spoken_punctuation_enabled: settings.spoken_punctuation_enabled,
            custom_replacements: &replacement_rules,
            focused_process: if focused.is_empty() {
                None
            } else {
                Some(focused)
            },
        },
    );
    if level != CleanupLevel::Raw {
        let glossary: Vec<(String, String)> = settings
            .dictionary_terms
            .iter()
            .map(|t| (t.term.clone(), t.preferred_spelling.clone()))
            .collect();
        smart = crate::formatting::TextCleaner::apply_glossary(&smart, &glossary);
    }
    let started = Instant::now();
    let wants_flow = settings.resolve_intent().run_llm
        && !settings.dictation_mode.eq_ignore_ascii_case("coding");
    let mut out = smart.clone();
    let mut used = false;
    let mut rewriter_error: Option<String> = None;
    if wants_flow {
        let runtime = std::sync::Arc::clone(&ctx.flow_runtime);
        let compute_backend = settings.refinement.device.clone();
        let override_layers = if settings.refinement.gpu_layers < 0 {
            None
        } else {
            Some(settings.refinement.gpu_layers.max(0) as u32)
        };
        let vram_reserve_mb = settings.memory_policy.vram_reserve_mb;
        // Direct call: this command is synchronous, so there is no async
        // context to preserve. The previous `block_in_place` panicked outright
        // when Tauri dispatched this onto a current-thread runtime.
        let ensure_result = runtime.ensure(
            model_id,
            &compute_backend,
            override_layers,
            vram_reserve_mb,
            settings.refinement.context_size,
        );
        if let Err(err) = &ensure_result {
            crate::rewrite::auto_install_if_missing(app, ctx, &compute_backend, err);
        }
        if ensure_result.is_ok() {
            runtime.set_deadline_ms(settings.refinement.deadline_ms);
        }
        let client = ctx.flow_runtime.client.read().clone();
        let req = RewriteRequest {
            text: smart.clone(),
            cleanup_level: settings.resolved_cleanup_level(),
            style: settings.style.clone(),
            dictation_mode: settings.dictation_mode.clone(),
            vocabulary: crate::session::session_vocabulary(&settings),
            app_process: focused.to_string(),
            model_id: model_id.to_string(),
        };
        let outcome = crate::rewrite::polish_or_fallback(&client, &smart, &req);
        out = outcome.final_text;
        used = outcome.used;
        rewriter_error = outcome.error;
    }
    out = crate::formatting::snippets::apply_snippets(&out, &settings.snippets);
    Ok(CleanupRun {
        smart,
        final_text: out,
        rewriter_used: used,
        model_id: model_id.into(),
        tier_label,
        latency_ms: started.elapsed().as_millis() as u64,
        rewriter_error,
    })
}

#[tauri::command]
pub fn preview_tier_cleanup(
    text: String,
    tier: Option<String>,
    style: Option<String>,
    mode: Option<crate::settings::Mode>,
    app: AppHandle,
    ctx: State<'_, AppContext>,
) -> Result<serde_json::Value, String> {
    if let Some(mode) = mode {
        if mode.custom_instructions.chars().count() > 2000 {
            return Err("Mode instructions are limited to 2000 characters".into());
        }
        let settings = ctx.settings_store.get().settings_for_mode("preview", &mode);
        let intent = settings.resolve_intent();
        let model = if intent.flow_model == "none" && !mode.custom_instructions.is_empty() {
            "qwen3.5-0.8b"
        } else {
            &intent.flow_model
        };
        if !mode.dictation_mode.eq_ignore_ascii_case("coding") && model != "none" {
            ctx.flow_runtime.ensure(
                model,
                &settings.refinement.device,
                (settings.refinement.gpu_layers >= 0)
                    .then_some(settings.refinement.gpu_layers as u32),
                settings.memory_policy.vram_reserve_mb,
                settings.refinement.context_size,
            )?;
            ctx.flow_runtime
                .set_deadline_ms(settings.refinement.deadline_ms);
        }
        let client = ctx.flow_runtime.client.read().clone();
        let result = session::postprocess_with_context(
            &text,
            &settings,
            "preview",
            &client,
            Some(&mode),
            &session::SessionContext::default(),
        );
        if let Some(error) = result.rewriter_error {
            return Err(error);
        }
        return Ok(
            serde_json::json!({"text": result.final_text, "latency_ms": result.formatting_ms + result.rewrite_ms.unwrap_or(0), "tier_used": intent.tier, "model_used": model, "rewriter_used": result.rewriter_used}),
        );
    }
    let run = run_cleanup_pipeline(&text, tier, style, "preview", &app, ctx.inner())?;
    let mut payload = serde_json::json!({
        "text": run.final_text,
        "latency_ms": run.latency_ms,
        "tier_used": run.tier_label,
        "model_used": run.model_id,
        "rewriter_used": run.rewriter_used,
    });
    if let Some(err) = run.rewriter_error {
        payload["rewriter_error"] = serde_json::Value::String(err);
    }
    Ok(payload)
}

#[derive(Debug, serde::Deserialize, Default)]
#[serde(default)]
pub struct RetryOptions {
    pub tier: Option<String>,
    pub mode: Option<String>,
    pub language: Option<String>,
}

/// Recognize the saved recording again, then clean the new transcript.
#[tauri::command]
pub async fn retry_history_transcript(
    id: String,
    options: Option<RetryOptions>,
    app: AppHandle,
    ctx: State<'_, AppContext>,
) -> Result<HistoryEntry, String> {
    let _operation = ctx
        .session_operation
        .try_lock()
        .map_err(|_| "Another dictation operation is in progress. Try again when it finishes.")?;
    let Some(entry) = ctx.history_store.get_entry(&id)? else {
        return Err("That transcript is no longer in history.".into());
    };
    let options = options.unwrap_or_default();
    if let Some(language) = &options.language {
        if language != "auto" {
            crate::asr::languages::resolve_language_name(language)?;
        }
    }
    if options
        .tier
        .as_deref()
        .is_some_and(|t| !matches!(t, "raw_verbatim" | "smart_flow" | "deep_context"))
    {
        return Err("Unknown retry tier".into());
    }
    let mode = options
        .mode
        .as_ref()
        .map(|id| {
            ctx.settings_store
                .get()
                .modes
                .into_iter()
                .find(|m| m.id == *id)
                .ok_or("Unknown retry mode")
        })
        .transpose()?;
    let raw = crate::session::transcribe_saved_audio_with_language(
        ctx.inner(),
        &id,
        options.language.as_deref(),
    )
    .await?;
    let language = ctx.asr_handle.get_detected_language();
    let settings = ctx.settings_store.get();
    let ctx_clone = ctx.inner().clone();
    let raw_for_cleanup = raw.clone();
    let run = tokio::task::spawn_blocking(move || {
        if let Some(mut mode) = mode {
            if let Some(tier) = &options.tier {
                mode.intelligence_tier = tier.clone();
            }
            let mut effective = settings.settings_for_mode(&entry.application_process, &mode);
            if let Some(tier) = &options.tier {
                effective.intelligence_tier = tier.clone();
            }
            let intent = effective.resolve_intent();
            let model = if intent.flow_model == "none"
                && (!mode.custom_instructions.is_empty() || mode.translate_to.is_some())
            {
                "qwen3.5-0.8b"
            } else {
                &intent.flow_model
            };
            if !mode.dictation_mode.eq_ignore_ascii_case("coding") && model != "none" {
                ctx_clone.flow_runtime.ensure(
                    model,
                    &effective.refinement.device,
                    (effective.refinement.gpu_layers >= 0)
                        .then_some(effective.refinement.gpu_layers as u32),
                    effective.memory_policy.vram_reserve_mb,
                    effective.refinement.context_size,
                )?;
                ctx_clone
                    .flow_runtime
                    .set_deadline_ms(effective.refinement.deadline_ms);
            }
            let client = ctx_clone.flow_runtime.client.read().clone();
            let result = session::postprocess_with_context(
                &raw_for_cleanup,
                &effective,
                &entry.application_process,
                &client,
                Some(&mode),
                &Default::default(),
            );
            return Ok(CleanupRun {
                smart: result.smart,
                final_text: result.final_text,
                rewriter_used: result.rewriter_used,
                model_id: model.to_owned(),
                tier_label: intent.tier,
                latency_ms: result.formatting_ms + result.rewrite_ms.unwrap_or(0),
                rewriter_error: result.rewriter_error,
            });
        }
        run_cleanup_pipeline(
            &raw_for_cleanup,
            Some(options.tier.unwrap_or(settings.intelligence_tier)),
            Some(settings.style),
            &entry.application_process,
            &app,
            &ctx_clone,
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    // A retry that fell back silently would overwrite a polished transcript
    // with unpolished text and call it a success — surface the error instead.
    if let Some(err) = run.rewriter_error {
        return Err(format!("Polish did not run: {err}"));
    }
    ctx.history_store.replace_transcript(
        &id,
        &raw,
        &run.smart,
        &run.final_text,
        run.rewriter_used,
        &language,
    )?;
    ctx.history_store
        .get_entry(&id)?
        .ok_or_else(|| "That transcript is no longer in history.".into())
}

#[tauri::command]
pub async fn extract_history_audio(
    id: String,
    ctx: State<'_, AppContext>,
) -> Result<String, String> {
    let store = ctx.history_store.clone();
    tokio::task::spawn_blocking(move || {
        let directory = dirs::download_dir().unwrap_or_else(PlatformSys::get_app_dir);
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let path = directory.join(format!(
            "Reflow-audio-{}-{}.wav",
            chrono::Local::now().format("%Y%m%d-%H%M%S"),
            uuid::Uuid::new_v4()
        ));
        store.export_audio(&id, &path)?;
        Ok(path.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Pre-AI text for an entry: the pre-LLM "smart" transcript when a rewriter
/// ran and it differs from the final, otherwise the raw ASR. Second call after
/// an undo falls through to raw, giving a two-step undo for free.
/// Mirrors `undoAiText` in src/historyDisplay.ts.
fn pre_ai_text(entry: &HistoryEntry) -> String {
    if entry.rewriter_used {
        let smart = entry.smart_transcript.trim();
        if !smart.is_empty() && smart != entry.final_transcript.trim() {
            return entry.smart_transcript.clone();
        }
    }
    entry.raw_transcript.clone()
}

pub fn undo_last_ai_edit_inner(ctx: &AppContext) -> Result<String, String> {
    let entries = ctx.history_store.get_entries(1, 0)?;
    let Some(entry) = entries.first() else {
        return Ok(String::new());
    };
    if !entry.rewriter_used && entry.raw_transcript == entry.final_transcript {
        return Ok(String::new());
    }
    let text = pre_ai_text(entry);
    if text.is_empty() {
        return Ok(String::new());
    }
    // Return recoverable source text. Appending a second paste is not an undo,
    // and targeting an arbitrary foreground app from a history action is unsafe.
    Ok(text)
}

/// Revert a stored entry's final transcript to its pre-AI text. Returns the
/// updated entry; `None` when the id is gone.
#[tauri::command]
pub fn undo_history_ai_edit(
    id: String,
    ctx: State<'_, AppContext>,
) -> Result<Option<HistoryEntry>, String> {
    let Some(entry) = ctx.history_store.get_entry(&id)? else {
        return Ok(None);
    };
    let text = pre_ai_text(&entry);
    if !text.is_empty() && text != entry.final_transcript {
        ctx.history_store.update_transcript(
            &id,
            &entry.smart_transcript,
            &text,
            entry.rewriter_used,
        )?;
    }
    ctx.history_store.get_entry(&id)
}

#[tauri::command]
pub fn undo_last_ai_edit(ctx: State<'_, AppContext>) -> Result<String, String> {
    undo_last_ai_edit_inner(ctx.inner())
}

#[tauri::command]
pub fn preview_cleanup(text: String, ctx: State<'_, AppContext>) -> String {
    let settings = ctx.settings_store.get();
    let rules = CustomReplacements::new(settings.custom_replacements.clone());
    format_transcript_ex(
        &text,
        FormatRequest {
            cleanup_level: CleanupLevel::parse(&settings.resolved_cleanup_level()),
            dictation_mode: &settings.dictation_mode,
            style: VoiceStyle::parse(&settings.style),
            filler_removal_enabled: settings.filler_removal_enabled,
            spoken_punctuation_enabled: settings.spoken_punctuation_enabled,
            custom_replacements: &rules,
            focused_process: None,
        },
    )
}

#[tauri::command]
pub fn dismiss_assistant(app: tauri::AppHandle) {
    overlay::hide_overlay(&app);
}
