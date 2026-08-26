use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HotkeyAction {
    StartRecording,
    StopRecording,
    ToggleRecording,
    CancelRecording,
}

pub struct HotkeyManager;

impl HotkeyManager {
    /// Normalizes human-friendly hotkey strings into Tauri shortcut representations
    pub fn normalize_shortcut(shortcut: &str) -> String {
        let trimmed = shortcut.trim();
        if trimmed.is_empty() {
            return crate::platform::default_hotkey().replace("Ctrl", "Control");
        }

        trimmed
            .replace("CommandOrControl", "Control")
            .replace("CTRL", "Control")
            .replace("Ctrl", "Control")
            .replace("ctrl", "Control")
            .replace("Win", "Super")
            .replace(" ", "")
    }

    /// If the shortcut consists only of modifier keys (e.g. "Shift+Super"),
    /// return hook flags (shift=1, ctrl=2, alt=4, win=8). Requires ≥2
    /// modifiers — single modifiers would hijack normal typing.
    pub fn modifier_only_flags(shortcut: &str) -> Option<u8> {
        let mut flags = 0u8;
        for part in shortcut.split('+') {
            match part.trim().to_lowercase().as_str() {
                "shift" => flags |= 1,
                "ctrl" | "control" => flags |= 2,
                "alt" => flags |= 4,
                "win" | "super" | "meta" | "cmd" => flags |= 8,
                _ => return None,
            }
        }
        if flags.count_ones() >= 2 {
            Some(flags)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_ctrl_shift_space() {
        assert_eq!(
            HotkeyManager::normalize_shortcut("Ctrl+Shift+Space"),
            "Control+Shift+Space"
        );
    }

    #[test]
    fn empty_uses_platform_default() {
        let normalized = HotkeyManager::normalize_shortcut("  ");
        #[cfg(windows)]
        assert_eq!(normalized, "Shift+Win");
        #[cfg(target_os = "linux")]
        assert!(normalized.contains("Space"));
    }

    #[test]
    fn shift_win_is_modifier_only() {
        let normalized = HotkeyManager::normalize_shortcut("Shift+Win");
        assert_eq!(normalized, "Shift+Super");
        assert_eq!(HotkeyManager::modifier_only_flags(&normalized), Some(1 | 8));
    }

    #[test]
    fn ctrl_win_is_a_different_combo_from_shift_win() {
        let shift_win = HotkeyManager::modifier_only_flags("Shift+Win").unwrap();
        let ctrl_win = HotkeyManager::modifier_only_flags("Ctrl+Win").unwrap();
        assert_ne!(shift_win, ctrl_win);
        // Extra Ctrl must not satisfy a Shift+Win combo.
        assert_ne!(ctrl_win, shift_win);
        assert_eq!(ctrl_win & shift_win, 8); // only Win in common
    }
}
