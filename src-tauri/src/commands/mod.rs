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

pub fn spawn_start(app: AppHandle) {
    spawn_start_at(app, Instant::now());
}

/// `pressed_at` must be captured as early as possible in the input path —
/// ideally inside the OS hook callback — so `hotkey_to_recording_ms`
/// includes thread spawn, `SendInput`, IPC and tokio scheduling.
pub fn spawn_start_at(app: AppHandle, pressed_at: Instant) {
    tauri::async_runtime::spawn(async move {
        let ctx = app.state::<AppContext>().inner().clone();
        if let Err(err) = session::start_microphone_at(&ctx, Some(pressed_at)).await {
            log::error!("start_recording failed: {}", err.message);
            // A model still loading, a busy microphone, or a transient device
            // error is recoverable. Leaving the state as Error made the next
            // hotkey press fail as `session_busy` even though no session ever
            // started. Always return admission failures to Ready so retrying
            // works after the underlying condition clears.
            *ctx.state_enum.write() = AppStateEnum::Ready;
            let _ = app.emit("recording:error", err.message);
            overlay::hide_overlay(&app);
        }
    });
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

pub fn register_dictation_hotkey(app: &AppHandle, shortcut_str: &str) -> Result<(), String> {
    let normalized = HotkeyManager::normalize_shortcut(shortcut_str);

    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();

    // Modifier-only combos (Shift+Win, Ctrl+Alt, …) can't be registered via
    // RegisterHotKey — route them through the low-level keyboard hook.
    if let Some(flags) = HotkeyManager::modifier_only_flags(&normalized) {
        if !cfg!(windows) {
            let msg = "Modifier-only hotkeys (like Shift+Win) are only supported on Windows.";
            *app.state::<AppContext>().hotkey_error.write() = Some(msg.into());
            return Err(msg.into());
        }
        let app_start = app.clone();
        let app_stop = app.clone();
        let ok = crate::hotkey::hook::set_combo(
            flags,
            std::sync::Arc::new(move |pressed_at| spawn_start_at(app_start.clone(), pressed_at)),
            std::sync::Arc::new(move |released_at| spawn_stop_at(app_stop.clone(), released_at)),
        );
        return if ok {
            log::info!("Registered push-to-talk combo via keyboard hook: {shortcut_str}");
            let ctx = app.state::<AppContext>();
            *ctx.registered_hotkey.write() = shortcut_str.to_string();
            *ctx.hotkey_error.write() = None;
            Ok(())
        } else {
            let msg = format!("Could not install keyboard hook for {shortcut_str}.");
            log::error!("{msg}");
            *app.state::<AppContext>().hotkey_error.write() = Some(msg.clone());
            Err(msg)
        };
    }

    crate::hotkey::hook::clear_combo();

    let shortcut: Shortcut = normalized
        .parse()
        .map_err(|err| format!("Invalid shortcut '{normalized}': {err}"))?;

    let app_for_handler = app.clone();
    shortcuts
        .on_shortcut(shortcut, move |app_handle, _shortcut, event| {
            // Timestamp before any state or settings reads, for the same
            // reason the low-level hook does: everything after this line is
            // latency the user pays for.
            let event_at = Instant::now();
            let ctx = app_handle.state::<AppContext>();
            let settings = ctx.settings_store.get();
            let state = *ctx.state_enum.read();
            match event.state() {
                ShortcutState::Pressed => {
                    if settings.push_to_talk {
                        if state != AppStateEnum::Recording
                            && matches!(state, AppStateEnum::Ready | AppStateEnum::Idle)
                        {
                            spawn_start_at(app_handle.clone(), event_at);
                        }
                    } else if matches!(state, AppStateEnum::Ready | AppStateEnum::Idle | AppStateEnum::Recording)
                    {
                        spawn_toggle_at(app_handle.clone(), event_at);
                    }
                }
                ShortcutState::Released => {
                    if settings.push_to_talk && state == AppStateEnum::Recording {
                        spawn_stop_at(app_handle.clone(), event_at);
                    }
                }
            }
        })
        .map_err(|err| {
            format!(
                "Could not register shortcut {normalized}. It may already be used by another app or blocked by the compositor ({err})."
            )
        })?;

    let ctx = app_for_handler.state::<AppContext>();
    *ctx.registered_hotkey.write() = normalized;
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

#[tauri::command]
pub fn update_settings(
    settings: Value,
    app: AppHandle,
    ctx: State<'_, AppContext>,
) -> Result<AppSettings, String> {
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

    let previous = ctx.settings_store.get();
    let updated = ctx.settings_store.merge_update(patch)?;
    if updated.asr.runtime != previous.asr.runtime {
        prepare_asr_runtime(ctx.inner(), &app)?;
    }
    let _ = app.emit("settings:changed", &updated);

    if updated.hotkey != previous.hotkey {
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
        let store = ctx.history_store.clone();
        let policy = updated.history_retention.clone();
        tauri::async_runtime::spawn(async move {
            let _ = crate::history::RetentionCleaner::apply_retention(&store, &policy);
        });
    }

    if !updated.resolve_intent().run_llm {
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
pub fn get_audio_devices() -> Vec<AudioDeviceInfo> {
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

#[tauri::command]
pub async fn start_recording(app: AppHandle, ctx: State<'_, AppContext>) -> Result<(), String> {
    let ctx = ctx.inner().clone();
    start_recording_inner(app, &ctx).await
}

#[tauri::command]
pub async fn stop_recording(app: AppHandle, ctx: State<'_, AppContext>) -> Result<String, String> {
    let ctx = ctx.inner().clone();
    stop_recording_inner(app, &ctx).await
}

#[tauri::command]
pub fn cancel_recording(app: AppHandle, ctx: State<'_, AppContext>) -> Result<(), String> {
    session::cancel(ctx.inner()).map_err(Into::<String>::into)?;
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
pub fn resolve_asr_load(ctx: &AppContext) -> (String, String, String) {
    let settings = ctx.settings_store.get();
    let requested = settings.asr.model.clone();
    let manager = ctx.model_manager.clone();
    let installed = move |id: &str| manager.is_installed(id);
    let caps = crate::capability::capabilities_uncached();
    // The performance preset now reaches the real load. It used to be read only
    // by `preview_profile`, so choosing "Fast" or "Accurate" in Settings changed
    // what the preview said and nothing else.
    let preset = crate::profile::Preset::parse(&settings.preset);

    let selection = crate::profile::select_asr_load_for_runtime(
        &settings.asr.runtime,
        preset,
        &requested,
        &settings.asr.device,
        &settings.asr.precision,
        &installed,
        &caps,
        &crate::profile::no_measurements,
    );

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

    (
        selection.model_id.to_string(),
        selection.device.as_str().to_string(),
        selection.precision.as_str().to_string(),
    )
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
pub fn install_model(
    app: AppHandle,
    model_size: Option<String>,
    ctx: State<'_, AppContext>,
) -> Result<(), String> {
    let settings = ctx.settings_store.get();
    let active = crate::model::manager::runtime_model_id(
        &model_size.unwrap_or(settings.asr.model),
        &settings.asr.runtime,
    );
    prepare_asr_runtime(ctx.inner(), &app)?;
    let model_dir = ctx.model_manager.get_model_dir(&active);
    let repo = ctx.model_manager.repo_for(&active);
    ctx.asr_handle
        .install_model_dir_blocking(&model_dir.to_string_lossy(), repo)?;
    spawn_model_status_watch(app, ctx.inner().clone());
    Ok(())
}

#[tauri::command]
pub fn remove_model(model_size: Option<String>, ctx: State<'_, AppContext>) -> Result<(), String> {
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
    prepare_asr_runtime(ctx.inner(), &app)?;
    // Same resolution as the startup load, so a manual reload cannot end up
    // running a different model than a launch would.
    let (model_id, device, precision) = resolve_asr_load(ctx.inner());
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
        loop {
            let status = {
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
                ctx.model_manager.get_status(&engine_status, &active)
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
            last = Some(status);
            tokio::time::sleep(if ready { POLL_READY } else { POLL_PENDING }).await;
        }
    });
}

#[tauri::command]
pub fn get_latency_metrics(ctx: State<'_, AppContext>) -> LatencyMetrics {
    ctx.last_latency_metrics.read().clone()
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

#[tauri::command]
pub fn get_benchmark_report() -> crate::benchmark::FullBenchmarkReport {
    crate::benchmark::load_benchmark_report().unwrap_or_else(|| {
        let report = crate::benchmark::run_synthetic_benchmark(&[], 0, 0.0);
        crate::benchmark::persist_benchmark_report(&report);
        report
    })
}

#[tauri::command]
pub fn run_system_benchmark(
    ctx: State<'_, AppContext>,
) -> Result<crate::benchmark::FullBenchmarkReport, String> {
    let asr_status = ctx.asr_handle.engine_status();
    let load_ms = (asr_status.load_seconds * 1000.0) as u64;
    let warmup_rtf = asr_status.warmup_rtf.unwrap_or(0.25);

    let report = crate::benchmark::run_synthetic_benchmark(&[], load_ms, warmup_rtf as f64);
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
    crate::profile::resolve_profile(
        &caps,
        crate::profile::Preset::parse(&preset),
        &overrides.unwrap_or_default(),
        &crate::profile::no_measurements,
    )
}

#[tauri::command]
pub fn open_logs_folder() -> Result<(), String> {
    let logs_dir = PlatformSys::get_logs_dir();
    let _ = std::fs::create_dir_all(&logs_dir);
    platform::open_path(&logs_dir)
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
    ctx.settings_store.get().resolve_intent().run_llm
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
        ..current
    })?;

    let intent = updated.resolve_intent();
    if intent.run_llm {
        let ctx_flow = ctx.inner().clone();
        let flow_model = intent.flow_model.clone();
        let backend = updated.refinement.device.clone();
        let override_layers = if updated.refinement.gpu_layers < 0 {
            None
        } else {
            Some(updated.refinement.gpu_layers.max(0) as u32)
        };
        let reserve = updated.memory_policy.vram_reserve_mb;
        let context_size = updated.refinement.context_size;
        tauri::async_runtime::spawn(async move {
            let _ = tokio::task::spawn_blocking(move || {
                let runtime = &ctx_flow.flow_runtime;
                match runtime.ensure(
                    &flow_model,
                    &backend,
                    override_layers,
                    reserve,
                    context_size,
                ) {
                    Ok(()) => runtime.warm_prompt_cache(&flow_model),
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
        let mut response = match request.send() {
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

#[tauri::command]
pub fn preview_tier_cleanup(
    text: String,
    tier: Option<String>,
    style: Option<String>,
    app: AppHandle,
    ctx: State<'_, AppContext>,
) -> Result<serde_json::Value, String> {
    use std::time::Instant;
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
    let focused = "preview";
    let replacement_rules = CustomReplacements::new(settings.custom_replacements.clone());
    let level = CleanupLevel::parse(&settings.resolved_cleanup_level());
    let mut smart = format_transcript_ex(
        &text,
        FormatRequest {
            cleanup_level: level,
            dictation_mode: &settings.dictation_mode,
            style: VoiceStyle::parse(&settings.style),
            filler_removal_enabled: settings.filler_removal_enabled,
            spoken_punctuation_enabled: settings.spoken_punctuation_enabled,
            custom_replacements: &replacement_rules,
            focused_process: Some(focused),
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
    let wants_flow = model_id != "none"
        && matches!(level, CleanupLevel::Medium | CleanupLevel::High)
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
            crate::rewrite::auto_install_if_missing(&app, ctx.inner(), &compute_backend, err);
        }
        let _ = ensure_result;
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
    let _ = settings;
    let latency_ms = started.elapsed().as_millis() as u64;
    let mut payload = serde_json::json!({
        "text": out,
        "latency_ms": latency_ms,
        "tier_used": tier_label,
        "model_used": model_id,
        "rewriter_used": used,
    });
    if let Some(err) = rewriter_error {
        payload["rewriter_error"] = serde_json::Value::String(err);
    }
    Ok(payload)
}

pub fn undo_last_ai_edit_inner(ctx: &AppContext) -> Result<String, String> {
    let entries = ctx.history_store.get_entries(1, 0)?;
    let Some(entry) = entries.first() else {
        return Ok(String::new());
    };
    if !entry.rewriter_used && entry.raw_transcript == entry.final_transcript {
        return Ok(String::new());
    }
    let text = if entry.rewriter_used {
        let smart = entry.smart_transcript.trim();
        if !smart.is_empty() && smart != entry.final_transcript.trim() {
            entry.smart_transcript.clone()
        } else {
            entry.raw_transcript.clone()
        }
    } else {
        entry.raw_transcript.clone()
    };
    if text.is_empty() {
        return Ok(String::new());
    }
    let restore = ctx.settings_store.get().clipboard_restore_enabled;
    // History re-inject: user just clicked a button, foreground is whatever
    // they were looking at. Skip foreground-match verification.
    TextInjector::inject(&text, restore, 0)?;
    Ok(text)
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
