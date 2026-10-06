pub mod api;
pub mod asr;
pub mod assistant_tools;
pub mod audio;
pub mod benchmark;
pub mod calibration;
pub mod capability;
pub mod commands;
pub mod context;
pub mod correction_observer;
pub mod dictionary;
pub mod dory;
pub mod expansion_commands;
pub mod file_jobs;
pub mod formatting;
pub mod history;
pub mod hotkey;
pub mod idle_policies;
pub mod injection;
pub mod model;
pub mod network_policy;
pub mod overlay;
pub mod pairing;
pub mod platform;
pub mod profile;
pub mod rewrite;
pub mod runtime;
pub mod session;
pub mod settings;
pub mod state;
pub mod transfer;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, WindowEvent,
};

use commands::{register_dictation_hotkey, spawn_toggle, AppContext};
use dory::DoryEvent;
use history::RetentionCleaner;
use state::AppStateEnum;

pub fn run() {
    let context = AppContext::bootstrap();
    let initial_settings = context.settings_store.get();
    let asr_handle = context.asr_handle.clone();
    let hotkey_error = std::sync::Arc::clone(&context.hotkey_error);

    let mut tauri_context = tauri::generate_context!();
    if crate::platform::PlatformSys::is_portable() {
        for window in &mut tauri_context.config_mut().app.windows {
            window.create = false;
        }
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // A second launch must not steal or block the global hotkey —
            // focus the existing window instead of running twice.
            log::info!("Second launch blocked; focusing the running instance.");
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .on_window_event(|window, event| {
            // Closing the main window hides it to the tray so dictation
            // (global hotkey) keeps working; Quit lives in the tray menu.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Folder {
                        path: crate::platform::PlatformSys::get_logs_dir(),
                        file_name: Some("reflow".into()),
                    }),
                ])
                .level(log::LevelFilter::Info)
                .build(),
        )
        .manage(context.clone())
        .setup(move |app| {
            if let Some(notice) = context.settings_store.recovery_notice() {
                log::warn!("{notice}");
            }
            if crate::platform::PlatformSys::is_portable() {
                for window in &app.config().app.windows {
                    let builder = tauri::WebviewWindowBuilder::from_config(app, window)?;
                    #[cfg(not(target_os = "macos"))]
                    let builder = builder.data_directory(
                        crate::platform::PlatformSys::get_app_dir()
                            .join("webview")
                            .join(&window.label),
                    );
                    builder.build()?;
                }
            }

            // The app can remain in the tray for days. Expire audio there too.
            let retention_context = context.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    let ctx = retention_context.clone();
                    match tokio::task::spawn_blocking(move || {
                        let _operation = ctx.settings_operation.lock();
                        RetentionCleaner::apply_retention(
                            &ctx.history_store,
                            &ctx.settings_store.get().history_retention,
                        )?;
                        ctx.history_store.purge_expired_audio()
                    })
                    .await
                    {
                        Ok(Ok(_)) => {}
                        result => log::error!("History/audio retention cleanup failed: {result:?}"),
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                }
            });
            if let Ok(resource_dir) = app.path().resource_dir() {
                let _ = asr_handle.set_resource_dir_blocking(resource_dir);
            }
            if let Err(err) = asr_handle.initialize_blocking() {
                log::warn!("ASR initialize failed: {err}");
            }

            // Install the refinement-runtime recovery hook here, where an
            // `AppHandle` legitimately exists. The session layer only sees a
            // callback, so it never links the GUI runtime.
            {
                let app_recovery = app.handle().clone();
                let ctx_recovery = context.clone();
                *context.runtime_recovery.write() = Some(std::sync::Arc::new(
                    move |request: crate::context::RuntimeRecoveryRequest| {
                        crate::rewrite::auto_install_if_missing(
                            &app_recovery,
                            &ctx_recovery,
                            &request.compute_backend,
                            &request.reason,
                        );
                    },
                ));
            }

            // Load the ASR model (GPU-first, CPU fallback) as soon as the
            // window is up so dictation is ready seconds later. The
            // precision is whatever the user last picked — this is what
            // makes the "remember my precision across launches" guarantee
            // work.
            if (initial_settings.asr.keep_loaded || initial_settings.preset != "custom")
                && crate::runtime::has_installed_asr(&context, &initial_settings)
            {
                let ctx_load = context.clone();
                tauri::async_runtime::spawn(async move {
                    // Ask the sidecar to compute its CUDA probe, then wait for
                    // the answer before resolving the load. The probe runs
                    // inside the sidecar's `_warm_imports` (measured 9–57s on
                    // this machine) and previously only ran on the first
                    // `load_model` — so resolving at startup always saw
                    // `cuda_available=false` (still pending) and picked the
                    // CPU path (0.6B on CPU) on a machine with a perfectly
                    // usable GPU. `probe_cuda` starts the warmup without
                    // loading anything; the status poll below observes it
                    // finishing. The budget keeps a probe that never lands
                    // from blocking the load forever.
                    if let Err(err) = ctx_load.asr_handle.probe_cuda().await {
                        log::warn!("Could not start the sidecar CUDA probe: {err}");
                    }
                    const CUDA_PROBE_BUDGET: std::time::Duration =
                        std::time::Duration::from_secs(90);
                    let started = std::time::Instant::now();
                    let mut probe_answered = false;
                    while started.elapsed() < CUDA_PROBE_BUDGET {
                        if let Ok(status) = ctx_load.asr_handle.refresh_status().await {
                            if !status.cuda_probe_pending {
                                probe_answered = true;
                                break;
                            }
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
                    }
                    if !probe_answered {
                        log::warn!(
                            "Sidecar CUDA probe did not answer within {:?}; \
                             resolving the ASR load with whatever is known",
                            CUDA_PROBE_BUDGET
                        );
                    }
                    // Resolved rather than taken verbatim from Settings: the stored
                    // choice is a ceiling, and a model that cannot fit free VRAM is
                    // stepped down so the work stays on the GPU. See
                    // `profile::select_asr_load`.
                    let resolved = commands::resolve_asr_load(&ctx_load);
                    match resolved {
                        Ok((model_id, backend, precision)) => {
                            let model_dir = ctx_load.model_manager.get_model_dir(&model_id);
                            let asr = ctx_load.asr_handle.clone();
                            let result = asr
                                .load_model_with_precision(
                                    &model_dir.to_string_lossy(),
                                    &backend,
                                    &precision,
                                )
                                .await;
                            if let Err(err) = result {
                                log::warn!("Deferred model load failed: {err}");
                            }
                        }
                        Err(err) => log::warn!("Deferred model load rejected: {err}"),
                    }
                });
            } else if !context
                .model_manager
                .is_installed(&model::manager::runtime_model_id(
                    &initial_settings.asr.model,
                    &initial_settings.asr.runtime,
                ))
            {
                log::warn!(
                    "Qwen3-ASR ({}) weights not found; install from Settings → Model.",
                    initial_settings.asr.model
                );
            }

            // Unconditionally, and regardless of whether a load was just
            // queued. This poller owns the only refresh of the ASR status
            // cache, which both the UI and the "may I record?" gate read. If it
            // is not running, a model that loads (or is loaded by a later
            // Settings action) is never observed.
            commands::spawn_model_status_watch(app.handle().clone(), context.clone());

            // A force-killed previous run leaves its llama-server child
            // orphaned, still holding VRAM. Clear those out before the
            // refinement runtime starts budgeting memory — otherwise every
            // subsequent launch inherits a tighter GPU than it really has.
            {
                let killed = crate::rewrite::server::kill_orphaned_llama_servers();
                if killed > 0 {
                    log::warn!("Killed {killed} orphaned llama-server process(es)");
                }
            }

            // Start the refinement runtime now rather than on the first
            // dictation.
            //
            // Cold-starting `llama-server` measures ~2-5s and evaluating the
            // shared prompt prefix costs seconds more on CPU, and both used to
            // land on the user mid-dictation while they waited for text. This
            // runs after the ASR load has been queued so the VRAM budgeter sees
            // the memory the ASR model is actually going to take, and it stays
            // on a background task so a slow or failing runtime never delays
            // window creation.
            {
                let intent = initial_settings.resolve_intent();
                if intent.run_llm
                    && initial_settings.preset != "fast"
                    && initial_settings.refinement.keep_warm
                {
                    let ctx_flow = context.clone();
                    let deadline_ms = initial_settings.refinement.deadline_ms;
                    tauri::async_runtime::spawn(async move {
                        // The ASR load resolves at the sidecar's ack, not at
                        // completion — its weights land on the GPU seconds to
                        // minutes later. Budgeting refinement GPU layers from
                        // free VRAM in that window over-allocates and the
                        // llama-server either OOMs or lands on the CPU.
                        commands::wait_for_asr_load_settled(&ctx_flow.asr_handle).await;
                        let flow_rt = ctx_flow.flow_runtime.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            let effective = crate::runtime::current_settings(&ctx_flow);
                            let intent = effective.resolve_intent();
                            if !intent.run_llm || !effective.refinement.keep_warm {
                                return Ok(());
                            }
                            let flow_model = intent.flow_model;
                            let backend = effective.refinement.device;
                            let override_layers = (effective.refinement.gpu_layers >= 0)
                                .then_some(effective.refinement.gpu_layers.max(0) as u32);
                            let runtime = &ctx_flow.flow_runtime;
                            runtime
                                .ensure(
                                    &flow_model,
                                    &backend,
                                    override_layers,
                                    effective.memory_policy.vram_reserve_mb,
                                    effective.refinement.context_size,
                                )
                                .map(|()| {
                                    runtime.warm_prompt_cache(&flow_model);
                                })
                        })
                        .await;
                        match result {
                            Ok(Ok(())) => {
                                flow_rt.set_deadline_ms(deadline_ms);
                                log::info!("Refinement runtime preloaded")
                            }
                            Ok(Err(err)) => log::warn!(
                                "Refinement runtime preload failed ({err}); \
                                 the first dictation will start it on demand"
                            ),
                            Err(err) => log::warn!("Refinement preload task failed: {err}"),
                        }
                    });
                }
            }

            if initial_settings.launch_at_startup {
                if let Err(err) = crate::platform::set_launch_at_startup(true) {
                    log::warn!("Failed to apply autostart: {err}");
                }
            }

            if initial_settings.start_minimized {
                if let Some(main) = app.get_webview_window("main") {
                    let _ = main.hide();
                }
            }

            overlay::position_overlay(
                app.handle(),
                &initial_settings.overlay_position,
                &initial_settings.hud_scale,
            );

            bind_dory_ui(app.handle().clone(), context.bus.clone(), context.clone());
            idle_policies::start(app.handle().clone(), context.clone());

            if initial_settings.api_enabled {
                let ctx_api = context.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(err) = crate::api::sync_server(ctx_api).await {
                        log::error!("Failed to start LAN API: {err}");
                    }
                });
            }

            let tray_menu = Menu::new(app)?;
            let item_status = MenuItem::with_id(
                app,
                "status",
                "● Reflow: Ready (Qwen3-ASR)",
                false,
                None::<&str>,
            )?;
            let item_dictate =
                MenuItem::with_id(app, "dictate", "Start / Stop Dictation", true, None::<&str>)?;
            // The quick Fast/Polished switch. Reachable without opening
            // Settings, because the whole point is to flip modes mid-workflow.
            let polish_on = initial_settings.resolve_intent().run_llm;
            let item_polish = MenuItem::with_id(
                app,
                "toggle_polish",
                if polish_on {
                    "Polishing: On  →  switch to Fast"
                } else {
                    "Polishing: Off  →  switch to Polished"
                },
                true,
                None::<&str>,
            )?;
            let item_history =
                MenuItem::with_id(app, "history", "Open History", true, None::<&str>)?;
            let item_settings =
                MenuItem::with_id(app, "settings", "Open Settings", true, None::<&str>)?;
            let item_offline =
                MenuItem::with_id(app, "offline", "Toggle airplane mode", true, None::<&str>)?;
            let item_note = MenuItem::with_id(app, "note", "New note", true, None::<&str>)?;
            let item_quit = MenuItem::with_id(app, "quit", "Quit Reflow", true, None::<&str>)?;

            tray_menu.append(&item_status)?;
            tray_menu.append(&item_dictate)?;
            tray_menu.append(&item_polish)?;
            tray_menu.append(&item_history)?;
            tray_menu.append(&item_note)?;
            tray_menu.append(&item_offline)?;
            tray_menu.append(&item_settings)?;
            tray_menu.append(&item_quit)?;

            if let Some(main) = app.get_webview_window("main") {
                if let Some(icon) = app.default_window_icon() {
                    let _ = main.set_icon(icon.clone());
                }
            }

            let mut tray_builder = TrayIconBuilder::with_id("main")
                .menu(&tray_menu)
                .tooltip("Reflow — Local Dictation");

            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }

            let _tray = tray_builder
                .on_menu_event(move |app_handle, event| match event.id.as_ref() {
                    "dictate" => spawn_toggle(app_handle.clone()),
                    "toggle_polish" => {
                        let ctx = app_handle.state::<AppContext>();
                        let enabled = ctx.settings_store.get().resolve_intent().run_llm;
                        match commands::set_polish_enabled(
                            !enabled,
                            app_handle.clone(),
                            ctx.clone(),
                        ) {
                            Ok(updated) => {
                                let now_on = updated.resolve_intent().run_llm;
                                let _ = item_polish.set_text(if now_on {
                                    "Polishing: On  →  switch to Fast"
                                } else {
                                    "Polishing: Off  →  switch to Polished"
                                });
                            }
                            Err(err) => log::error!("Could not toggle polishing: {err}"),
                        }
                    }
                    "history" | "settings" => {
                        if let Some(window) = app_handle.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        app_handle.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            match register_dictation_hotkey(app.handle(), &initial_settings.hotkey) {
                Ok(()) => {
                    log::info!("Registered dictation hotkey: {}", initial_settings.hotkey);
                }
                Err(err) => {
                    log::error!("{err}");
                    *hotkey_error.write() = Some(err);
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            api::automation::get_automation_token_status,
            api::automation::create_automation_token,
            api::automation::revoke_automation_token,
            network_policy::get_network_journal,
            transfer::export_config,
            transfer::import_config,
            transfer::import_model_file,
            expansion_commands::get_stats,
            expansion_commands::repaste_last,
            expansion_commands::get_app_version,
            expansion_commands::open_releases,
            expansion_commands::add_note,
            expansion_commands::export_notes,
            expansion_commands::update_history_metadata,
            expansion_commands::edit_history_transcript,
            commands::dismiss_assistant,
            file_jobs::transcribe_file,
            file_jobs::cancel_file_transcription,
            file_jobs::get_file_job,
            file_jobs::export_file_transcript,
            file_jobs::select_audio_file,
            commands::get_app_state,
            commands::get_settings,
            commands::update_settings,
            commands::get_audio_devices,
            commands::set_audio_device,
            commands::get_current_audio_level,
            commands::get_asr_progress,
            commands::get_calibration_status,
            commands::run_calibration,
            commands::cancel_calibration,
            commands::apply_calibration,
            commands::test_microphone,
            commands::test_recognition,
            commands::retry_clipboard_restore,
            commands::start_recording,
            commands::start_meeting,
            expansion_commands::summarize_history,
            assistant_tools::execute_assistant_tool,
            commands::stop_recording,
            commands::cancel_recording,
            commands::inject_text,
            commands::get_history,
            commands::query_history,
            commands::export_history,
            commands::get_runtime_inventory,
            commands::rollback_runtime,
            commands::repair_runtime,
            commands::search_history,
            commands::delete_history_item,
            commands::clear_today_history,
            commands::clear_all_history,
            commands::get_dictionary_terms,
            commands::save_dictionary_term,
            commands::delete_dictionary_term,
            commands::get_custom_replacements,
            commands::save_custom_replacement,
            commands::delete_custom_replacement,
            commands::get_model_status,
            commands::install_model,
            commands::remove_model,
            commands::get_downloaded_models,
            commands::reload_model,
            commands::get_latency_metrics,
            commands::get_latency_report,
            commands::reset_latency_history,
            commands::get_benchmark_report,
            commands::run_system_benchmark,
            commands::get_system_metrics,
            commands::get_capabilities,
            commands::refresh_capabilities,
            commands::get_model_manifests,
            commands::preview_profile,
            commands::get_runtime_plan,
            commands::open_logs_folder,
            commands::get_diagnostics_report,
            commands::get_platform_info,
            commands::get_api_status,
            commands::rotate_pairing_code,
            commands::quit_app,
            commands::list_api_devices,
            commands::set_api_device_permissions,
            commands::revoke_api_device,
            commands::get_flow_status,
            commands::preview_cleanup,
            commands::undo_last_ai_edit,
            commands::undo_history_ai_edit,
            commands::retry_history_transcript,
            commands::extract_history_audio,
            commands::get_intelligence_status,
            commands::get_intelligence_tiers,
            commands::install_intelligence_model,
            commands::remove_intelligence_model,
            commands::install_llama_runtime,
            commands::remove_llama_runtime,
            commands::set_intelligence_tier,
            commands::get_polish_enabled,
            commands::set_polish_enabled,
            commands::preview_tier_cleanup,
        ])
        .run(tauri_context)
        .expect("Error while running Reflow desktop application");
}

fn bind_dory_ui(app: tauri::AppHandle, bus: crate::dory::DoryBus, ctx: AppContext) {
    let warning_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            interval.tick().await;
            if let Some(error) = crate::injection::TextInjector::take_restore_error() {
                // This can arrive during a later dictation. Keep its state
                // untouched; the warning only notifies about restoration.
                let _ = warning_app.emit(
                    "app:warning",
                    format!("Clipboard restoration failed: {error}. Retry in Settings → Advanced."),
                );
            }
        }
    });
    tauri::async_runtime::spawn(async move {
        let mut rx = bus.subscribe();
        let mut previous_state = AppStateEnum::Ready;
        loop {
            match rx.recv().await {
                Ok(DoryEvent::State(state)) => {
                    let settings = ctx.settings_store.get();
                    if settings.sounds_enabled
                        && (state == AppStateEnum::Recording
                            || (previous_state == AppStateEnum::Recording
                                && state == AppStateEnum::Processing))
                    {
                        let _ = app.emit("hud:sound", serde_json::json!({"kind":if state == AppStateEnum::Recording {"start"} else {"stop"}}));
                    }
                    previous_state = state;
                    let _ = app.emit("app:state-changed", state);
                    match state {
                        AppStateEnum::Recording => {
                            let settings = ctx.settings_store.get();
                            overlay::show_overlay(
                                &app,
                                &settings.overlay_position,
                                &settings.hud_scale,
                            );
                            if let Some(tray) = app.tray_by_id("main") {
                                let _ = tray.set_tooltip(Some("Reflow — Listening"));
                            }
                        }
                        AppStateEnum::Processing => {
                            overlay::resize_overlay(&app, "listening");
                        }
                        AppStateEnum::Injecting => {
                            // Still the active pipeline, so it keeps the active
                            // size. Only the settled result grows.
                            overlay::resize_overlay(&app, "listening");
                        }
                        AppStateEnum::Ready | AppStateEnum::Idle | AppStateEnum::Error => {
                            overlay::hide_overlay_later(app.clone(), 1200);
                            if let Some(tray) = app.tray_by_id("main") {
                                let _ = tray.set_tooltip(Some("Reflow — Ready"));
                            }
                        }
                        _ => {}
                    }
                }
                Ok(DoryEvent::Partial(payload)) => {
                    if payload.stage == "polishing" {
                        overlay::resize_overlay(&app, "listening");
                    }
                    let _ = app.emit("transcript:partial", payload.clone());
                    let _ = app.emit("recording:audio-level", payload.audio_level);
                }
                Ok(DoryEvent::AudioLevel(level)) => {
                    let _ = app.emit("recording:audio-level", level);
                }
                Ok(DoryEvent::Final(payload)) => {
                    overlay::resize_overlay(&app, "preview");
                    let _ = app.emit("transcript:final", payload);
                }
                Ok(DoryEvent::SessionFinished { raw, text, .. }) => {
                    let _ = app.emit(
                        "transcript:source",
                        serde_json::json!({"raw":raw,"final_text":text}),
                    );
                }
                Ok(DoryEvent::Injection(feedback)) => {
                    overlay::resize_overlay(&app, "preview");
                    let hide_delay = if feedback.fallback_copy { 2500 } else { 1200 };
                    let _ = app.emit("injection:result", feedback);
                    overlay::hide_overlay_later(app.clone(), hide_delay);
                }
                Ok(DoryEvent::DictionaryChanged(settings)) => {
                    let _ = app.emit("settings:changed", *settings);
                }
                Ok(DoryEvent::HistoryUpdated(entry)) => {
                    let _ = app.emit("history:updated", *entry);
                }
                Ok(DoryEvent::Error(err)) => {
                    let _ = app.emit("recording:error", err);
                }
                Ok(DoryEvent::AutoStop) => {
                    let _ = app.emit("app:auto-stop", ());
                }
                Ok(DoryEvent::FileProgress(progress)) => {
                    let _ = app.emit("transcribe:progress", progress);
                }
                Ok(DoryEvent::AssistantResponse(text)) => {
                    overlay::show_response(&app);
                    let _ = app.emit("assistant:response", text);
                }
                Ok(DoryEvent::Stage(stage)) => {
                    let _ = app.emit("pipeline:stage", stage);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

pub async fn run_api_standalone(bind: Option<String>) -> Result<(), String> {
    env_logger::init();
    let ctx = AppContext::bootstrap();
    if let Err(err) = ctx.asr_handle.initialize().await {
        log::warn!("ASR initialize failed: {err}");
    }
    let mut settings = ctx.settings_store.get();
    settings.api_enabled = true;
    if let Some(bind) = bind {
        if let Some((host, port)) = bind.rsplit_once(':') {
            settings.api_bind = if host == "127.0.0.1" || host == "localhost" {
                "localhost".into()
            } else {
                "lan".into()
            };
            if let Ok(port) = port.parse() {
                settings.api_port = port;
            }
        }
    }
    let _ = ctx.settings_store.update(settings.clone());
    crate::api::sync_server(ctx.clone()).await?;
    let status = crate::api::current_status(&ctx);
    println!("Reflow LAN API listening");
    for addr in &status.listen_addrs {
        println!("  https://{addr}:{}", status.port);
    }
    if let Some(code) = &status.pairing_code {
        println!("Pairing code: {code}");
    }

    // The API's transcription endpoints need the ASR model. Without this,
    // `run_api_standalone` served /health and /status fine while every
    // /v1/transcribe and /v1/stream request wedged for its full 180s
    // transcription budget inside `_wait_model_ready` and then failed —
    // the API was reachable but useless. Load the model the same way the
    // GUI does: probe CUDA first (see the startup-load comment in `run`),
    // then resolve and load.
    if (settings.asr.keep_loaded || settings.preset != "custom")
        && crate::runtime::has_installed_asr(&ctx, &settings)
    {
        if let Err(err) = ctx.asr_handle.probe_cuda().await {
            log::warn!("Could not start the sidecar CUDA probe: {err}");
        }
        const CUDA_PROBE_BUDGET: std::time::Duration = std::time::Duration::from_secs(90);
        let started = std::time::Instant::now();
        while started.elapsed() < CUDA_PROBE_BUDGET {
            match ctx.asr_handle.refresh_status().await {
                Ok(engine) if !engine.cuda_probe_pending => break,
                _ => {}
            }
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        }
        match commands::resolve_asr_load(&ctx) {
            Ok((model_id, backend, precision)) => {
                let model_dir = ctx.model_manager.get_model_dir(&model_id);
                match ctx
                    .asr_handle
                    .load_model_with_precision(&model_dir.to_string_lossy(), &backend, &precision)
                    .await
                {
                    Ok(()) => log::info!(
                        "ASR model loaded for the headless API ({model_id} on {backend}, {precision})"
                    ),
                    Err(err) => log::warn!("Headless ASR model load failed: {err}"),
                }
            }
            Err(err) => log::warn!("Headless ASR model load rejected: {err}"),
        }
    } else {
        log::warn!(
            "ASR model ({}) is not installed; the API's transcribe endpoints \
             will reject audio until it is",
            settings.asr.model
        );
    }

    // A headless run has no UI to press "rotate pairing code" on, and the
    // code minted at startup expires after 5 minutes — after which every
    // pairing attempt fails with advice the operator cannot follow. Keep a
    // live offer and announce each rotation, so the console the operator is
    // actually looking at always shows a usable code.
    let pairing = ctx.pairing.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(4 * 60)).await;
            let offer = pairing.rotate_code();
            println!("Pairing code (refreshed): {}", offer.code);
        }
    });

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}
