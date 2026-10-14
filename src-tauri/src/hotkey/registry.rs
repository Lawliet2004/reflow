use super::HotkeyManager;
use crate::settings::AppSettings;
use tauri_plugin_global_shortcut::Shortcut;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyBinding {
    pub shortcut: String,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyRegistry {
    pub bindings: Vec<HotkeyBinding>,
}

impl HotkeyRegistry {
    pub fn from_settings(settings: &AppSettings) -> Result<Self, String> {
        let mut bindings: Vec<HotkeyBinding> = Vec::new();
        let mut add = |shortcut: &str, action: String| -> Result<(), String> {
            if shortcut.trim().is_empty() {
                return Err(format!("Empty hotkey for {action}"));
            }
            let shortcut = HotkeyManager::normalize_shortcut(shortcut);
            let modifiers = HotkeyManager::modifier_only_flags(&shortcut);
            if modifiers.is_some() && (!cfg!(windows) || action == "cancel") {
                return Err(
                    "This shortcut needs a non-modifier key. Modifier-only action shortcuts are only supported on Windows."
                        .into(),
                );
            }
            let parsed =
                if modifiers.is_none() {
                    Some(shortcut.parse::<Shortcut>().map_err(|error| {
                        format!("Invalid hotkey {shortcut} for {action}: {error}")
                    })?)
                } else {
                    None
                };
            if let Some(conflict) = bindings.iter().find(|binding| {
                if let Some(modifiers) = modifiers {
                    HotkeyManager::modifier_only_flags(&binding.shortcut) == Some(modifiers)
                } else {
                    binding.shortcut.parse::<Shortcut>().ok() == parsed
                }
            }) {
                return Err(format!(
                    "Hotkey {shortcut} is assigned to both {} and {action}",
                    conflict.action
                ));
            }
            bindings.push(HotkeyBinding { shortcut, action });
            Ok(())
        };
        add(&settings.hotkeys.dictation, "dictate".into())?;
        for (shortcut, action) in [
            (&settings.hotkeys.command, "command"),
            (&settings.hotkeys.assistant, "assistant"),
            (&settings.hotkeys.note, "note"),
        ] {
            if let Some(shortcut) = shortcut {
                add(shortcut, action.into())?;
            }
        }
        for mode in settings.modes.iter().filter(|m| m.enabled) {
            if let Some(shortcut) = &mode.hotkey {
                add(shortcut, format!("mode:{}", mode.id))?;
            }
        }
        if let Some(shortcut) = &settings.hotkeys.cancel {
            add(shortcut, "cancel".into())?;
        }
        Ok(Self { bindings })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conflicts_are_reported_before_registration() {
        let mut s = AppSettings::default();
        // Use a key-bearing binding so every platform reaches conflict detection.
        s.hotkeys.dictation = "Ctrl+Shift+Space".into();
        s.hotkeys.command = Some(s.hotkeys.dictation.clone());
        assert!(HotkeyRegistry::from_settings(&s)
            .unwrap_err()
            .contains("both"));
    }

    #[test]
    fn cancel_accepts_a_custom_key_and_can_be_disabled() {
        let mut settings = AppSettings::default();
        settings.hotkeys.cancel = Some("Ctrl+Alt+Backspace".into());
        let registry = HotkeyRegistry::from_settings(&settings).unwrap();
        assert!(registry.bindings.iter().any(|binding| {
            binding.action == "cancel" && binding.shortcut == "Control+Alt+Backspace"
        }));
        settings.hotkeys.cancel = None;
        assert!(!HotkeyRegistry::from_settings(&settings)
            .unwrap()
            .bindings
            .iter()
            .any(|binding| binding.action == "cancel"));
    }

    #[test]
    fn cancel_conflicts_include_aliases_and_modifier_order() {
        let mut settings = AppSettings::default();
        settings.hotkeys.dictation = "Ctrl+Shift+Space".into();
        settings.hotkeys.cancel = Some("Shift+Control+Space".into());
        assert!(HotkeyRegistry::from_settings(&settings)
            .unwrap_err()
            .contains("both dictate and cancel"));
    }

    /// update_settings must not rebind when a patch changes only mode text —
    /// registry equality is the binding-diff the gate uses.
    #[test]
    fn registries_diff_only_on_effective_binding_changes() {
        let before = AppSettings::default();
        let mut text_only = before.clone();
        text_only.modes[1].custom_instructions = "Rewrite in a friendlier tone.".into();
        text_only.modes[2].name = "Field notes".into();
        assert_eq!(
            HotkeyRegistry::from_settings(&before).unwrap(),
            HotkeyRegistry::from_settings(&text_only).unwrap(),
            "instructions/name edits produce identical binding sets"
        );
        let mut rebound = before.clone();
        rebound.modes[0].hotkey = Some("Ctrl+Alt+N".into());
        assert_ne!(
            HotkeyRegistry::from_settings(&before).unwrap(),
            HotkeyRegistry::from_settings(&rebound).unwrap()
        );
        let mut dictation = before;
        dictation.hotkeys.dictation = "F9".into();
        assert_ne!(
            HotkeyRegistry::from_settings(&dictation).unwrap(),
            HotkeyRegistry::from_settings(&AppSettings::default()).unwrap()
        );
    }

    #[test]
    fn invalid_or_empty_shortcuts_do_not_reach_registration() {
        let mut settings = AppSettings::default();
        for key in ["  ", "Ctrl+NotAKey", "Ctrl+Shift"] {
            settings.hotkeys.cancel = Some(key.into());
            assert!(HotkeyRegistry::from_settings(&settings).is_err(), "{key}");
        }
    }
}
