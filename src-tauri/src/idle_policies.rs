use crate::{context::AppContext, state::AppStateEnum};
use tauri::Emitter;

fn resumes_ac(previous: Option<bool>, current: Option<bool>) -> bool {
    previous == Some(true) && current == Some(false)
}

/// Device changes never interrupt a live capture. Power transitions wait for idle.
pub fn start(app: tauri::AppHandle, ctx: AppContext) {
    tauri::async_runtime::spawn(async move {
        let mut last_battery = None;
        let mut last_mic = String::new();
        let mut ticks = 0u8;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
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
                    #[cfg(target_os = "linux")]
                    if let Some(name) = crate::audio::pipewire::default_source_description() {
                        return name;
                    }
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
            // Desktop audio probes can take seconds. They only refresh the UI,
            // so never hold recording admission while waiting for them.
            // Acquire/recheck only for the power transition that unloads models.
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
            let battery = crate::platform::media::on_battery();
            if battery.is_none() || battery == last_battery {
                continue;
            }
            let previous = last_battery;
            last_battery = battery;
            if battery == Some(true) && settings.power_policy.unload_on_battery {
                ctx.flow_runtime.shutdown();
                if let Err(error) = ctx.asr_handle.unload_model().await {
                    let _ = app.emit("recording:error", error);
                }
            }
            if resumes_ac(previous, battery)
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

#[cfg(test)]
mod tests {
    use super::resumes_ac;

    #[test]
    fn discovering_ac_power_does_not_reload_an_already_loaded_model() {
        assert!(!resumes_ac(None, Some(false)));
        assert!(!resumes_ac(Some(false), Some(false)));
        assert!(!resumes_ac(Some(false), None));
    }

    #[test]
    fn only_a_known_battery_to_ac_transition_resumes_the_model() {
        assert!(resumes_ac(Some(true), Some(false)));
        assert!(!resumes_ac(Some(false), Some(true)));
        assert!(!resumes_ac(Some(true), None));
    }
}
