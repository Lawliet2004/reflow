use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DisplaySession {
    Windows,
    Macos,
    X11,
    Wayland,
    Unknown,
}

impl DisplaySession {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::X11 => "x11",
            Self::Wayland => "wayland",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_env_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "wayland" => Self::Wayland,
            "x11" => Self::X11,
            "windows" => Self::Windows,
            "macos" | "darwin" => Self::Macos,
            _ => Self::Unknown,
        }
    }

    pub fn injection_notes(self) -> &'static str {
        match self {
            Self::Windows => "Native clipboard paste via SendInput (Ctrl+V).",
            Self::X11 => "X11 clipboard paste. Terminals use Ctrl+Shift+V.",
            Self::Wayland => {
                "Allow Reflow keyboard access in the desktop dialog for automatic paste on Wayland. Until permission is granted, transcripts stay on the clipboard for manual paste."
            }
            Self::Macos => "macOS pastes with ⌘V. Reflow needs Accessibility access (System Settings → Privacy & Security → Accessibility).",
            Self::Unknown => "Display session could not be detected. Paste may require a manual clipboard shortcut.",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformInfo {
    pub os: String,
    pub session: String,
    pub default_hotkey: String,
    pub data_dir: String,
    pub logs_dir: String,
    pub hotkey_error: Option<String>,
    pub injection_notes: String,
}

pub trait PlatformAdapter {
    fn session() -> DisplaySession;

    fn default_hotkey() -> &'static str {
        if cfg!(target_os = "linux") || Self::session() == DisplaySession::Macos {
            "Ctrl+Shift+Space"
        } else {
            "Shift+Win"
        }
    }

    fn os_display_name() -> String {
        format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
    }

    fn active_window() -> (String, String) {
        ("Unknown".into(), "unknown".into())
    }

    fn simulate_paste(process: &str) -> Result<(), String>;

    fn foreground_hwnd() -> isize {
        0
    }

    fn focus_hwnd(_hwnd: isize) -> bool {
        false
    }

    fn open_path(path: &Path) -> Result<(), String> {
        let path = path.to_path_buf();
        if cfg!(windows) {
            std::process::Command::new("explorer")
                .arg(&path)
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("Failed to open folder: {e}"))
        } else if cfg!(target_os = "macos") {
            std::process::Command::new("open")
                .arg(&path)
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("Failed to open folder: {e}"))
        } else {
            std::process::Command::new("xdg-open")
                .arg(&path)
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("Failed to open folder: {e}"))
        }
    }

    fn set_launch_at_startup(_enabled: bool) -> Result<(), String> {
        Ok(())
    }
}

pub fn parse_hyprctl_active_window(json: &str) -> Option<(String, String)> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let title = value
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();
    let class = value
        .get("class")
        .and_then(|v| v.as_str())
        .or_else(|| value.get("initialClass").and_then(|v| v.as_str()))
        .unwrap_or("unknown")
        .to_string();
    Some((title, class))
}

pub fn parse_sway_focused(json: &str) -> Option<(String, String)> {
    fn walk(node: &serde_json::Value) -> Option<(String, String)> {
        if node.get("focused").and_then(|v| v.as_bool()) == Some(true) {
            let title = node
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string();
            let app = node
                .get("app_id")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    node.get("window_properties")
                        .and_then(|p| p.get("class"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("unknown")
                .to_string();
            return Some((title, app));
        }
        if let Some(nodes) = node.get("nodes").and_then(|v| v.as_array()) {
            for child in nodes {
                if let Some(found) = walk(child) {
                    return Some(found);
                }
            }
        }
        if let Some(nodes) = node.get("floating_nodes").and_then(|v| v.as_array()) {
            for child in nodes {
                if let Some(found) = walk(child) {
                    return Some(found);
                }
            }
        }
        None
    }
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    walk(&value)
}

pub fn linux_terminal_process(process: &str) -> bool {
    let name = process
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(process)
        .trim()
        .to_ascii_lowercase();

    const TERMINALS: &[&str] = &[
        "alacritty",
        "cool-retro-term",
        "deepin-terminal",
        "foot",
        "footclient",
        "ghostty",
        "gnome-terminal",
        "gnome-terminal-server",
        "hyper",
        "kitty",
        "kgx",
        "konsole",
        "lxterminal",
        "mate-terminal",
        "ptyxis",
        "qterminal",
        "rio",
        "rxvt",
        "st",
        "tabby",
        "terminator",
        "terminology",
        "tilix",
        "urxvt",
        "uxterm",
        "wezterm",
        "wezterm-gui",
        "xfce4-terminal",
        "xterm",
    ];

    TERMINALS.iter().any(|term| name == *term)
}

pub fn paste_chord_label(process: &str, session: DisplaySession) -> &'static str {
    match session {
        DisplaySession::X11 | DisplaySession::Wayland if linux_terminal_process(process) => {
            "Ctrl+Shift+V"
        }
        DisplaySession::Macos => "⌘V",
        _ => "Ctrl+V",
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn linux_keyboard_settings(session: DisplaySession) -> enigo::Settings {
    let mut settings = enigo::Settings::default();
    if session == DisplaySession::Wayland {
        // Enigo 0.3 otherwise opens both connections and treats an X11 event
        // as success even when the focused application is a native Wayland
        // client. It has no backend-disable flag; an invalid explicit display
        // prevents X11 fallback without changing this process's environment.
        settings.x11_display = Some("reflow-native-wayland-only".into());
    }
    settings
}

#[cfg(target_os = "linux")]
pub fn simulate_paste_with_enigo(ctrl: bool, shift: bool, key: char) -> Result<(), String> {
    use enigo::{
        Direction::{Click, Press, Release},
        Enigo, Key, Keyboard,
    };

    let mut enigo =
        Enigo::new(&linux_keyboard_settings(super::session())).map_err(|e| e.to_string())?;
    if ctrl {
        enigo.key(Key::Control, Press).map_err(|e| e.to_string())?;
    }
    if shift {
        enigo.key(Key::Shift, Press).map_err(|e| e.to_string())?;
    }
    enigo
        .key(Key::Unicode(key), Click)
        .map_err(|e| e.to_string())?;
    if shift {
        enigo.key(Key::Shift, Release).map_err(|e| e.to_string())?;
    }
    if ctrl {
        enigo
            .key(Key::Control, Release)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ClipboardMac;

    impl PlatformAdapter for ClipboardMac {
        fn session() -> DisplaySession {
            DisplaySession::Macos
        }

        fn simulate_paste(_process: &str) -> Result<(), String> {
            Err("clipboard preview".into())
        }
    }

    #[test]
    fn macos_preview_default_hotkey_has_a_registrable_key() {
        let hotkey = ClipboardMac::default_hotkey();
        assert_eq!(hotkey, "Ctrl+Shift+Space");
        let normalized = crate::hotkey::HotkeyManager::normalize_shortcut(hotkey);
        assert!(crate::hotkey::HotkeyManager::modifier_only_flags(&normalized).is_none());
        normalized
            .parse::<tauri_plugin_global_shortcut::Shortcut>()
            .expect("the macOS default must parse as a global shortcut");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn wayland_keyboard_does_not_silently_connect_to_xwayland() {
        let settings = linux_keyboard_settings(DisplaySession::Wayland);
        assert!(x11rb::connect(settings.x11_display.as_deref()).is_err());
        assert_eq!(settings.wayland_display, None);
        assert_eq!(
            linux_keyboard_settings(DisplaySession::X11).x11_display,
            None
        );
    }
}
