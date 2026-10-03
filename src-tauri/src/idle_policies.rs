use crate::{context::AppContext, state::AppStateEnum};
use tauri::Emitter;

/// Device changes never interrupt a live capture. Power transitions wait for idle.
pub fn start(app: tauri::AppHandle, ctx: AppContext) {
    tauri::async_runtime::spawn(async move {
        let mut last_battery = None;
        let mut last_mic = String::new();
        let mut ticks = 0u8;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let Ok(_guard) = ctx.session_operation.try_lock() else {
                continue;
            };
            if !matches!(
                *ctx.state_enum.read(),
                AppStateEnum::Ready | AppStateEnum::Idle
            ) {
                continue;
            }
            let settings = ctx.settings_store.get();
            if settings.follow_default_mic && settings.microphone_device_id.is_none() {
                let name = tokio::task::spawn_blocking(|| {
                    use cpal::traits::{DeviceTrait, HostTrait};
                    cpal::default_host()
                        .default_input_device()
                        .and_then(|device| device.name().ok())
                        .unwrap_or_default()
                })
                .await
                .unwrap_or_default();
                if name != last_mic {
                    last_mic = name.clone();
                    let _ = app.emit("audio:default-mic", name);
                }
            }
            ticks = (ticks + 1) % 6;
            if ticks != 0 {
                continue;
            }
            let battery = crate::platform::media::on_battery();
            if battery == last_battery {
                continue;
            }
            last_battery = battery;
            if battery == Some(true) && settings.power_policy.unload_on_battery {
                ctx.flow_runtime.shutdown();
                if let Err(error) = ctx.asr_handle.unload_model().await {
                    let _ = app.emit("recording:error", error);
                }
            }
            if battery == Some(false)
                && settings.power_policy.unload_on_battery
                && settings.asr.keep_loaded
            {
                let context = ctx.clone();
                if let Err(error) = tokio::task::spawn_blocking(move || {
                    let (id, device, precision) = crate::commands::resolve_asr_load(&context)?;
                    context.asr_handle.load_model_with_precision_blocking(
                        &context.model_manager.get_model_dir(&id).to_string_lossy(),
                        &device,
                        &precision,
                    )
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
                {
                    let _ = app.emit("recording:error", error);
                }
            }
            let _ = app.emit("power:changed", battery);
        }
    });
}
