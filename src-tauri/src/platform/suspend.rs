//! Stop capture and discard keyboard sessions before suspend. Resume rebinds
//! shortcuts and restores only previously approved keyboard permission.
use futures_util::StreamExt;
use tauri::{Emitter, Manager};

pub fn start(app: tauri::AppHandle, ctx: crate::context::AppContext) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = monitor(app, ctx).await {
            log::warn!("Suspend monitoring unavailable: {error}");
        }
    });
}

async fn monitor(app: tauri::AppHandle, ctx: crate::context::AppContext) -> Result<(), String> {
    let connection = zbus::connection::Builder::system()
        .map_err(|error| error.to_string())?
        .method_timeout(std::time::Duration::from_secs(2))
        .build()
        .await
        .map_err(|error| error.to_string())?;
    let proxy = zbus::Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await
    .map_err(|error| error.to_string())?;
    let mut events = proxy
        .receive_signal("PrepareForSleep")
        .await
        .map_err(|error| error.to_string())?;
    while let Some(event) = events.next().await {
        let (sleeping,): (bool,) = event
            .body()
            .deserialize()
            .map_err(|error| error.to_string())?;
        if sleeping {
            super::linux_input_portal::shutdown();
            crate::hotkey::portal::clear();
            let _ = crate::session::cancel_owned_wait(&ctx, None).await;
            crate::overlay::hide_overlay(&app);
            let runtime = ctx.flow_runtime.clone();
            let _ = tokio::task::spawn_blocking(move || runtime.shutdown()).await;
        } else {
            super::linux_input_portal::restore_access();
            if let Err(error) =
                crate::commands::register_dictation_hotkey(&app, &ctx.settings_store.get().hotkey)
            {
                let _ = app.emit("hotkey:error", error);
            }
            if let Some(window) = app.get_webview_window("main") {
                let visible =
                    window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false);
                let _ = window.emit("window:visibility", visible);
            }
        }
    }
    Err("login session disconnected".into())
}
