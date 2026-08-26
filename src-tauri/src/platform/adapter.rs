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
                "Wayland cannot always inject keys. Reflow tries enigo, then wtype, then ydotool. If paste fails, text stays on the clipboard — press the paste shortcut."
            }
            Self::Macos => "macOS text injection is not implemented in this build. Text is copied to the clipboard.",
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
        if cfg!(target_os = "linux") {
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
        _ => "Ctrl+V",
    }
}

#[cfg(target_os = "linux")]
pub fn simulate_paste_with_enigo(ctrl: bool, shift: bool, key: char) -> Result<(), String> {
    use enigo::{
        Direction::{Click, Press, Release},
        Enigo, Key, Keyboard, Settings,
    };

    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    if ctrl {
        enigo
            .key(Key::Control, Press)
            .map_err(|e| e.to_string())?;
    }
    if shift {
        enigo.key(Key::Shift, Press).map_err(|e| e.to_string())?;
    }
    enigo
        .key(Key::Unicode(key), Click)
        .map_err(|e| e.to_string())?;
    if shift {
        enigo
            .key(Key::Shift, Release)
            .map_err(|e| e.to_string())?;
    }
    if ctrl {
        enigo
            .key(Key::Control, Release)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
