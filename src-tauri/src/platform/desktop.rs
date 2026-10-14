//! Live integration state, separate from saved preferences and ASR readiness.
use std::sync::OnceLock;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::DisplaySession;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutState {
    Pending,
    Ready,
    Unassigned,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyboardState {
    NotRequired,
    NotRequested,
    Pending,
    Ready,
    Denied,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShortcutStatus {
    pub state: ShortcutState,
    pub assigned_trigger: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeyboardStatus {
    pub state: KeyboardState,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesktopIntegrationStatus {
    pub session: String,
    pub shortcut: ShortcutStatus,
    pub keyboard: KeyboardStatus,
    pub hud_supported: bool,
    pub foreground_app_available: bool,
    pub assigned_shortcuts: std::collections::BTreeMap<String, String>,
    pub shortcut_editable: bool,
}

impl DesktopIntegrationStatus {
    pub fn for_session(session: DisplaySession) -> Self {
        let wayland = session == DisplaySession::Wayland;
        Self {
            session: session.as_str().into(),
            shortcut: ShortcutStatus {
                state: ShortcutState::Pending,
                assigned_trigger: None,
                message: None,
            },
            keyboard: KeyboardStatus {
                state: if wayland {
                    KeyboardState::NotRequested
                } else {
                    KeyboardState::NotRequired
                },
                message: None,
            },
            hud_supported: !wayland,
            // XWayland's last active window is not the foreground Wayland app.
            foreground_app_available: !wayland,
            assigned_shortcuts: std::collections::BTreeMap::new(),
            shortcut_editable: !wayland || {
                #[cfg(target_os = "linux")]
                {
                    super::kde_shortcuts::supported_desktop()
                }
                #[cfg(not(target_os = "linux"))]
                {
                    false
                }
            },
        }
    }
}

static APP: OnceLock<AppHandle> = OnceLock::new();
static STATUS: OnceLock<Mutex<DesktopIntegrationStatus>> = OnceLock::new();

fn status() -> &'static Mutex<DesktopIntegrationStatus> {
    STATUS.get_or_init(|| Mutex::new(DesktopIntegrationStatus::for_session(super::session())))
}

pub fn configure(app: AppHandle) {
    let _ = APP.set(app);
}

pub fn snapshot() -> DesktopIntegrationStatus {
    status().lock().clone()
}

fn update(change: impl FnOnce(&mut DesktopIntegrationStatus)) {
    let mut current = status().lock();
    let before = current.clone();
    change(&mut current);
    if before != *current {
        // Serialize/emit under the lock to retain update order across workers.
        if let Some(app) = APP.get() {
            let _ = app.emit("desktop:integration-changed", current.clone());
        }
    }
}

pub fn shortcut(state: ShortcutState, trigger: Option<String>, message: Option<String>) {
    update(|status| {
        if matches!(state, ShortcutState::Pending | ShortcutState::Unavailable) {
            status.assigned_shortcuts.clear();
        }
        status.shortcut = ShortcutStatus {
            state,
            assigned_trigger: trigger,
            message,
        }
    });
}

pub fn keyboard(state: KeyboardState, message: Option<String>) {
    update(|status| status.keyboard = KeyboardStatus { state, message });
}

pub fn hud(supported: bool) {
    update(|status| status.hud_supported = supported);
}

pub fn assignments(shortcuts: std::collections::BTreeMap<String, String>) {
    update(|status| status.assigned_shortcuts = shortcuts);
}

/// Called only after the native configuration API confirms the actual key.
/// A subsequent portal event supplies the desktop's localized description.
pub fn assigned(action: &str, trigger: Option<String>) {
    update(|status| {
        if let Some(trigger) = &trigger {
            status
                .assigned_shortcuts
                .insert(action.into(), trigger.clone());
        } else {
            status.assigned_shortcuts.remove(action);
        }
        if action == "dictate" {
            status.shortcut = ShortcutStatus {
                state: if trigger.is_some() {
                    ShortcutState::Ready
                } else {
                    ShortcutState::Unassigned
                },
                message: trigger
                    .is_none()
                    .then(|| "Choose a dictation shortcut in Settings → General.".into()),
                assigned_trigger: trigger,
            };
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wayland_starts_without_claiming_permissions_or_foreground_information() {
        let status = DesktopIntegrationStatus::for_session(DisplaySession::Wayland);
        assert_eq!(status.keyboard.state, KeyboardState::NotRequested);
        assert!(!status.hud_supported);
        assert!(!status.foreground_app_available);
        assert_eq!(
            serde_json::to_value(status).unwrap()["shortcut"]["state"],
            "pending"
        );
    }

    #[test]
    fn native_keyboard_backends_do_not_require_wayland_permission() {
        for session in [
            DisplaySession::Windows,
            DisplaySession::X11,
            DisplaySession::Macos,
        ] {
            assert_eq!(
                DesktopIntegrationStatus::for_session(session)
                    .keyboard
                    .state,
                KeyboardState::NotRequired
            );
        }
    }
}
