//! Desktop-only management of the localhost automation credential.
use crate::context::AppContext;
use crate::pairing::PairedDevicePublic;
use serde::Serialize;

#[derive(Serialize)]
pub struct AutomationToken {
    pub token: String,
    pub device: PairedDevicePublic,
}

#[tauri::command]
pub fn get_automation_token_status(
    ctx: tauri::State<'_, AppContext>,
) -> Option<PairedDevicePublic> {
    ctx.pairing.automation_device()
}

#[tauri::command]
pub fn create_automation_token(
    ctx: tauri::State<'_, AppContext>,
) -> Result<AutomationToken, String> {
    let _settings_operation = ctx.settings_operation.lock();
    let settings = ctx.settings_store.get();
    if !settings.api_enabled || settings.api_bind != "localhost" {
        return Err(
            "Enable the API with localhost binding before creating an automation token".into(),
        );
    }
    let (token, device) = ctx.pairing.create_automation_token()?;
    Ok(AutomationToken { token, device })
}

#[tauri::command]
pub fn revoke_automation_token(ctx: tauri::State<'_, AppContext>) -> Result<bool, String> {
    match ctx.pairing.automation_device() {
        Some(device) => ctx.pairing.revoke(&device.id),
        None => Ok(false),
    }
}
