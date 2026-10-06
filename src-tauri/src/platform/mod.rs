mod adapter;
mod keys;
pub use crate::injection::injector::capture_selection;
pub use keys::{send_key, simulate_copy, type_text};
pub mod sys;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

pub use adapter::{
    linux_terminal_process, parse_hyprctl_active_window, parse_sway_focused, paste_chord_label,
    DisplaySession, PlatformAdapter, PlatformInfo,
};
pub use sys::PlatformSys;

#[cfg(target_os = "linux")]
pub use linux::LinuxAdapter as CurrentAdapter;
#[cfg(target_os = "macos")]
pub use macos::MacOsAdapter as CurrentAdapter;
#[cfg(windows)]
pub use windows::WindowsAdapter as CurrentAdapter;

/// Fallback adapter when compiling for an unexpected OS.
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub struct CurrentAdapter;

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
impl PlatformAdapter for CurrentAdapter {
    fn session() -> DisplaySession {
        DisplaySession::Unknown
    }
}

pub fn session() -> DisplaySession {
    CurrentAdapter::session()
}

pub fn default_hotkey() -> &'static str {
    CurrentAdapter::default_hotkey()
}

pub fn os_display_name() -> String {
    CurrentAdapter::os_display_name()
}

pub fn active_window() -> (String, String) {
    CurrentAdapter::active_window()
}

pub fn simulate_paste(process: &str) -> Result<(), String> {
    CurrentAdapter::simulate_paste(process)
}

pub fn foreground_hwnd() -> isize {
    CurrentAdapter::foreground_hwnd()
}

pub fn focus_hwnd(hwnd: isize) -> bool {
    CurrentAdapter::focus_hwnd(hwnd)
}

/// Request focus for `hwnd` and wait until the OS actually reports it as the
/// foreground window, up to `timeout`.
///
/// `SetForegroundWindow` is asynchronous: it returns before the foreground has
/// changed. Synthesizing Ctrl+V straight afterwards is a race, and losing it
/// sends the keystroke to whichever window is still focused — so the paste
/// silently lands somewhere else (or nowhere) while the code believes it
/// succeeded. Polling for the transition converts that race into a bounded
/// wait, and lets the caller fall back to "left on the clipboard" honestly when
/// focus genuinely cannot be taken.
pub fn focus_hwnd_and_confirm(hwnd: isize, timeout: std::time::Duration) -> bool {
    if hwnd == 0 {
        return false;
    }
    if foreground_hwnd() == hwnd {
        return true;
    }
    focus_hwnd(hwnd);
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if foreground_hwnd() == hwnd {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    foreground_hwnd() == hwnd
}

pub fn open_path(path: &std::path::Path) -> Result<(), String> {
    CurrentAdapter::open_path(path)
}

pub fn set_launch_at_startup(enabled: bool) -> Result<(), String> {
    CurrentAdapter::set_launch_at_startup(enabled)
}

pub fn platform_info(hotkey_error: Option<String>) -> PlatformInfo {
    let session = session();
    PlatformInfo {
        os: os_display_name(),
        session: session.as_str().to_string(),
        default_hotkey: default_hotkey().to_string(),
        data_dir: PlatformSys::get_app_dir().display().to_string(),
        logs_dir: PlatformSys::get_logs_dir().display().to_string(),
        hotkey_error,
        injection_notes: session.injection_notes().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_process_detection() {
        assert!(linux_terminal_process("kitty"));
        assert!(linux_terminal_process("/usr/bin/gnome-terminal-server"));
        assert!(linux_terminal_process("wezterm-gui"));
        assert!(linux_terminal_process("kgx"));
        assert!(!linux_terminal_process("code"));
        assert!(!linux_terminal_process("firefox"));
        assert!(!linux_terminal_process("chrome"));
    }

    #[test]
    fn paste_chord_linux_terminal_uses_shift() {
        assert_eq!(
            paste_chord_label("kitty", DisplaySession::X11),
            "Ctrl+Shift+V"
        );
        assert_eq!(paste_chord_label("firefox", DisplaySession::X11), "Ctrl+V");
        assert_eq!(
            paste_chord_label("kitty", DisplaySession::Windows),
            "Ctrl+V"
        );
    }

    #[test]
    fn session_parse_from_env_values() {
        assert_eq!(
            DisplaySession::from_env_value("wayland"),
            DisplaySession::Wayland
        );
        assert_eq!(DisplaySession::from_env_value("x11"), DisplaySession::X11);
        assert_eq!(DisplaySession::from_env_value("X11"), DisplaySession::X11);
        assert_eq!(DisplaySession::from_env_value(""), DisplaySession::Unknown);
    }

    #[test]
    fn default_hotkey_is_platform_specific() {
        let hotkey = default_hotkey();
        #[cfg(target_os = "linux")]
        {
            assert!(hotkey.contains("Space"));
            assert_eq!(hotkey, "Ctrl+Shift+Space");
        }
        #[cfg(windows)]
        assert_eq!(hotkey, "Shift+Win");
        #[cfg(target_os = "macos")]
        assert_eq!(hotkey, "Ctrl+Shift+Space");
    }

    #[test]
    fn fresh_and_missing_legacy_hotkeys_use_the_platform_default() {
        let settings = crate::settings::AppSettings::default();
        assert_eq!(settings.hotkey, default_hotkey());
        assert_eq!(settings.hotkeys.dictation, default_hotkey());
        crate::hotkey::registry::HotkeyRegistry::from_settings(&settings)
            .expect("fresh platform hotkeys must be supported");

        let mut legacy = serde_json::json!({"settings_version": 4});
        crate::settings::config::migrate_document(&mut legacy);
        assert_eq!(legacy["hotkeys"]["dictation"], default_hotkey());

        let mut saved = serde_json::json!({"settings_version": 4, "hotkey": "Shift+Win"});
        crate::settings::config::migrate_document(&mut saved);
        assert_eq!(saved["hotkeys"]["dictation"], "Shift+Win");
    }

    #[test]
    fn parses_hyprctl_json() {
        let json = r#"{"class":"kitty","title":"nvim"}"#;
        assert_eq!(
            parse_hyprctl_active_window(json),
            Some(("nvim".into(), "kitty".into()))
        );
    }

    #[test]
    fn parses_sway_tree() {
        let json = r#"{
            "nodes": [
                {"focused": false, "name": "a", "app_id": "x"},
                {"focused": true, "name": "vim", "app_id": "foot", "floating_nodes": []}
            ],
            "floating_nodes": []
        }"#;
        assert_eq!(
            parse_sway_focused(json),
            Some(("vim".into(), "foot".into()))
        );
    }
}

pub mod media;

pub fn open_browser(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
    if !matches!(parsed.scheme(), "https" | "http") || url.len() > 8192 {
        return Err("Unsupported browser URL".into());
    }
    #[cfg(windows)]
    let result = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", url])
            .creation_flags(0x08000000)
            .spawn()
    };
    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    return Err("Opening browser URLs is unavailable".into());
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    result
        .map(|mut child| {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        })
        .map_err(|e| e.to_string())
}
