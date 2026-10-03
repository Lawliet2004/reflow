use super::HotkeyManager;
use crate::settings::AppSettings;

#[derive(Debug, Clone)]
pub struct HotkeyBinding {
    pub shortcut: String,
    pub action: String,
}

#[derive(Debug, Clone)]
pub struct HotkeyRegistry {
    pub bindings: Vec<HotkeyBinding>,
}

impl HotkeyRegistry {
    pub fn from_settings(settings: &AppSettings) -> Result<Self, String> {
        let mut bindings: Vec<HotkeyBinding> = Vec::new();
        let mut add = |shortcut: &str, action: String| -> Result<(), String> {
            let shortcut = HotkeyManager::normalize_shortcut(shortcut);
            if shortcut.trim().is_empty() {
                return Err(format!("Empty hotkey for {action}"));
            }
            if let Some(conflict) = bindings.iter().find(|b| b.shortcut == shortcut) {
                return Err(format!(
                    "Hotkey {shortcut} is assigned to both {} and {action}",
                    conflict.action
                ));
            }
            if HotkeyManager::modifier_only_flags(&shortcut).is_some() && !cfg!(windows) {
                return Err(
                    "Modifier-only hotkeys are only supported on Windows. Add a non-modifier key."
                        .into(),
                );
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
        Ok(Self { bindings })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conflicts_are_reported_before_registration() {
        let mut s = AppSettings::default();
        s.hotkeys.command = Some(s.hotkeys.dictation.clone());
        assert!(HotkeyRegistry::from_settings(&s)
            .unwrap_err()
            .contains("both"));
    }
}
