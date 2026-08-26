pub mod sys;
mod adapter;

#[cfg(windows)]
mod windows;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

pub use adapter::{
    linux_terminal_process, parse_hyprctl_active_window, parse_sway_focused, paste_chord_label,
    DisplaySession, PlatformAdapter, PlatformInfo,
};
pub use sys::PlatformSys;

#[cfg(windows)]
pub use windows::WindowsAdapter as CurrentAdapter;
#[cfg(target_os = "linux")]
pub use linux::LinuxAdapter as CurrentAdapter;
#[cfg(target_os = "macos")]
pub use macos::MacOsAdapter as CurrentAdapter;

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
        assert_eq!(
            paste_chord_label("firefox", DisplaySession::X11),
            "Ctrl+V"
        );
        assert_eq!(
            paste_chord_label("kitty", DisplaySession::Windows),
            "Ctrl+V"
        );
    }

    #[test]
    fn session_parse_from_env_values() {
        assert_eq!(DisplaySession::from_env_value("wayland"), DisplaySession::Wayland);
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
