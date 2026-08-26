pub mod api;
pub mod asr;
pub mod audio;
pub mod commands;
pub mod context;
pub mod dory;
pub mod formatting;
pub mod history;
pub mod hotkey;
pub mod injection;
pub mod model;
pub mod overlay;
pub mod pairing;
pub mod rewrite;
pub mod platform;
pub mod session;
pub mod settings;
pub mod state;

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
    let _ = RetentionCleaner::apply_retention(
        &context.history_store,
        &initial_settings.history_retention,
    );
    let asr_engine = std::sync::Arc::clone(&context.asr_engine);
    let hotkey_error = std::sync::Arc::clone(&context.hotkey_error);

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
            if let Ok(resource_dir) = app.path().resource_dir() {
                asr_engine.write().set_resource_dir(resource_dir);
            }
            if let Err(err) = asr_engine.write().initialize() {
                log::warn!("ASR initialize failed: {err}");
            }

            // Load the ASR model (GPU-first, CPU fallback) as soon as the
            // window is up so dictation is ready seconds later. The
            // precision is whatever the user last picked — this is what
            // makes the "remember my precision across launches" guarantee
            // work.
            if initial_settings.keep_model_loaded
                && context.model_manager.is_installed(&initial_settings.asr_model)
            {
                let ctx_load = context.clone();
                let model_dir = ctx_load
                    .model_manager
                    .get_model_dir(&initial_settings.asr_model);
                let backend = initial_settings.compute_backend.clone();
                let precision = initial_settings.asr_precision.clone();
                std::thread::spawn(move || {
                    let result = {
                        let mut engine = ctx_load.asr_engine.write();
                        engine.load_model_with_precision(
                            &model_dir.to_string_lossy(),
                            &backend,
                            &precision,
                        )
                    };
                    if let Err(err) = result {
                        log::warn!("Deferred model load failed: {err}");
                    }
                });
                commands::spawn_model_status_watch(app.handle().clone(), context.clone());
            } else if !context.model_manager.is_installed(&initial_settings.asr_model) {
                log::warn!(
                    "Qwen3-ASR ({}) weights not found; install from Settings → Model.",
                    initial_settings.asr_model
                );
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

            overlay::position_overlay(app.handle(), &initial_settings.overlay_position);

            bind_dory_ui(app.handle().clone(), context.bus.clone(), context.clone());

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
            let item_history =
                MenuItem::with_id(app, "history", "Open History", true, None::<&str>)?;
            let item_settings =
                MenuItem::with_id(app, "settings", "Open Settings", true, None::<&str>)?;
            let item_undo =
                MenuItem::with_id(app, "undo_ai", "Undo last AI edit", true, None::<&str>)?;
            let item_quit = MenuItem::with_id(app, "quit", "Quit Reflow", true, None::<&str>)?;

            tray_menu.append(&item_status)?;
            tray_menu.append(&item_dictate)?;
            tray_menu.append(&item_history)?;
            tray_menu.append(&item_settings)?;
            tray_menu.append(&item_undo)?;
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
                    "undo_ai" => {
                        let ctx = app_handle.state::<AppContext>();
                        let _ = commands::undo_last_ai_edit_inner(ctx.inner());
                    }
                    "history" | "settings" => {
                        if let Some(window) = app_handle.get_webview_window("main") {
                            let _ = window.show();
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
            commands::get_app_state,
            commands::get_settings,
            commands::update_settings,
            commands::get_audio_devices,
            commands::set_audio_device,
            commands::get_current_audio_level,
            commands::start_recording,
            commands::stop_recording,
            commands::cancel_recording,
            commands::inject_text,
            commands::get_history,
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
            commands::reload_model,
            commands::get_latency_metrics,
            commands::get_system_metrics,
            commands::open_logs_folder,
            commands::get_diagnostics_report,
            commands::get_platform_info,
            commands::get_api_status,
            commands::rotate_pairing_code,
            commands::quit_app,
            commands::list_api_devices,
            commands::revoke_api_device,
            commands::get_flow_status,
            commands::preview_cleanup,
            commands::undo_last_ai_edit,
            commands::get_intelligence_status,
            commands::get_intelligence_tiers,
            commands::install_intelligence_model,
            commands::remove_intelligence_model,
            commands::install_llama_runtime,
            commands::remove_llama_runtime,
            commands::set_intelligence_tier,
            commands::preview_tier_cleanup,
        ])
        .run(tauri::generate_context!())
        .expect("Error while running Reflow desktop application");
}

fn bind_dory_ui(app: tauri::AppHandle, bus: crate::dory::DoryBus, ctx: AppContext) {
    tauri::async_runtime::spawn(async move {
        let mut rx = bus.subscribe();
        loop {
            match rx.recv().await {
                Ok(DoryEvent::State(state)) => {
                    let _ = app.emit("app:state-changed", state);
                    match state {
                        AppStateEnum::Recording => {
                            overlay::show_overlay(&app, &ctx.settings_store.get().overlay_position);
                            if let Some(tray) = app.tray_by_id("main") {
                                let _ = tray.set_tooltip(Some("Reflow — Listening"));
                            }
                        }
                        AppStateEnum::Processing => {
                            overlay::resize_overlay(&app, "listening");
                        }
                        AppStateEnum::Injecting => {
                            overlay::resize_overlay(&app, "preview");
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
                Ok(DoryEvent::Injection(feedback)) => {
                    overlay::resize_overlay(&app, "preview");
                    let hide_delay = if feedback.fallback_copy { 2500 } else { 1200 };
                    let _ = app.emit("injection:result", feedback);
                    overlay::hide_overlay_later(app.clone(), hide_delay);
                }
                Ok(DoryEvent::Error(err)) => {
                    let _ = app.emit("recording:error", err);
                }
                Ok(DoryEvent::AutoStop) => {
                    let _ = app.emit("app:auto-stop", ());
                }
                Ok(DoryEvent::Stage(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

pub async fn run_api_standalone(bind: Option<String>) -> Result<(), String> {
    env_logger::init();
    let ctx = AppContext::bootstrap();
    if let Err(err) = ctx.asr_engine.write().initialize() {
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
    let _ = ctx.settings_store.update(settings);
    crate::api::sync_server(ctx.clone()).await?;
    let status = crate::api::current_status(&ctx);
    println!("Reflow LAN API listening");
    for addr in &status.listen_addrs {
        println!("  http://{addr}:{}", status.port);
    }
    if let Some(code) = &status.pairing_code {
        println!("Pairing code: {code}");
    }
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}
