use std::path::Path;

use super::adapter::{
    linux_terminal_process, parse_hyprctl_active_window, parse_sway_focused,
    simulate_paste_with_enigo, DisplaySession, PlatformAdapter,
};

pub struct LinuxAdapter;

impl LinuxAdapter {
    fn detect_session() -> DisplaySession {
        if let Ok(value) = std::env::var("XDG_SESSION_TYPE") {
            let parsed = DisplaySession::from_env_value(&value);
            if parsed != DisplaySession::Unknown {
                return parsed;
            }
        }
        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            return DisplaySession::Wayland;
        }
        if std::env::var("DISPLAY").is_ok() {
            return DisplaySession::X11;
        }
        DisplaySession::Unknown
    }

    fn compositor_active_window() -> Option<(String, String)> {
        if let Some(win) = hyprland_active_window() {
            return Some(win);
        }
        if let Some(win) = sway_active_window() {
            return Some(win);
        }
        // KDE's queryWindowInfo is an interactive window picker, not a
        // foreground-window query. Never launch it during dictation. KDE
        // Wayland does not expose focused app details through this API.
        None
    }

    fn x11_active_window() -> Option<(String, String)> {
        #[cfg(target_os = "linux")]
        {
            use x11rb::connection::Connection;
            use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
            use x11rb::rust_connection::RustConnection;

            let (conn, screen_num) = RustConnection::connect(None).ok()?;
            let screen = &conn.setup().roots[screen_num];
            let root = screen.root;

            let intern = |name: &[u8]| {
                conn.intern_atom(false, name)
                    .ok()
                    .and_then(|cookie| cookie.reply().ok())
                    .map(|reply| reply.atom)
            };

            let net_active = intern(b"_NET_ACTIVE_WINDOW")?;
            let prop = conn
                .get_property(false, root, net_active, AtomEnum::WINDOW, 0, 1)
                .ok()?
                .reply()
                .ok()?;
            let window = prop.value32().and_then(|mut values| values.next())?;

            let mut title = String::new();
            if let (Some(net_wm_name), Some(utf8)) =
                (intern(b"_NET_WM_NAME"), intern(b"UTF8_STRING"))
            {
                if let Ok(cookie) = conn.get_property(false, window, net_wm_name, utf8, 0, 1024) {
                    if let Ok(reply) = cookie.reply() {
                        title = String::from_utf8_lossy(&reply.value).into_owned();
                    }
                }
            }
            if title.trim().is_empty() {
                if let Ok(cookie) =
                    conn.get_property(false, window, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 1024)
                {
                    if let Ok(reply) = cookie.reply() {
                        title = String::from_utf8_lossy(&reply.value).into_owned();
                    }
                }
            }
            if title.trim().is_empty() {
                title = "Unknown".into();
            }

            let mut process = "unknown".to_string();
            if let Some(net_wm_pid) = intern(b"_NET_WM_PID") {
                if let Ok(cookie) =
                    conn.get_property(false, window, net_wm_pid, AtomEnum::CARDINAL, 0, 1)
                {
                    if let Ok(reply) = cookie.reply() {
                        if let Some(pid) = reply.value32().and_then(|mut values| values.next()) {
                            process = std::fs::read_to_string(format!("/proc/{pid}/comm"))
                                .unwrap_or_else(|_| "unknown".into())
                                .trim()
                                .to_string();
                        }
                    }
                }
            }

            Some((title, process))
        }
        #[cfg(not(target_os = "linux"))]
        None
    }
}

impl PlatformAdapter for LinuxAdapter {
    fn session() -> DisplaySession {
        Self::detect_session()
    }

    fn os_display_name() -> String {
        let pretty = std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|contents| {
                contents.lines().find_map(|line| {
                    line.strip_prefix("PRETTY_NAME=")
                        .map(|value| value.trim_matches('"').to_string())
                })
            });
        pretty.unwrap_or_else(|| "Linux".into())
    }

    fn active_window() -> (String, String) {
        match Self::detect_session() {
            DisplaySession::X11 | DisplaySession::Unknown => Self::x11_active_window()
                .or_else(Self::compositor_active_window)
                .unwrap_or_else(|| ("Unknown".into(), "unknown".into())),
            DisplaySession::Wayland => Self::compositor_active_window()
                .unwrap_or_else(|| ("Unknown".into(), "unknown".into())),
            _ => ("Unknown".into(), "unknown".into()),
        }
    }

    fn simulate_paste(process: &str) -> Result<(), String> {
        let shift = linux_terminal_process(process);
        match Self::detect_session() {
            DisplaySession::Wayland => {
                return super::linux_input_portal::chord(true, shift, 'v' as i32)
            }
            // wtype/ydotool are Wayland tools; on X11 the proven fallback is
            // xdotool with cleared modifiers.
            DisplaySession::X11 => {
                if simulate_paste_with_enigo(true, shift, 'v').is_ok() {
                    return Ok(());
                }
                if xdotool_paste(shift).is_ok() {
                    return Ok(());
                }
                return Err(
                    "Could not simulate paste (enigo/xdotool). Text is on the clipboard.".into(),
                );
            }
            _ => {}
        }
        if simulate_paste_with_enigo(true, shift, 'v').is_ok() {
            return Ok(());
        }
        if wtype_paste(shift).is_ok() {
            return Ok(());
        }
        if ydotool_paste(shift).is_ok() {
            return Ok(());
        }
        Err("Could not simulate paste (enigo/wtype/ydotool). Text is on the clipboard.".into())
    }

    fn open_path(path: &Path) -> Result<(), String> {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| {
                format!(
                    "Failed to open folder with xdg-open: {e}. Install xdg-utils if it is missing."
                )
            })
    }

    fn set_launch_at_startup(enabled: bool) -> Result<(), String> {
        let autostart_dir = dirs::config_dir()
            .ok_or_else(|| "Could not locate your Linux configuration directory".to_owned())?
            .join("autostart");
        let desktop_path = autostart_dir.join("reflow.desktop");

        if !enabled {
            return match std::fs::remove_file(&desktop_path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(format!("Failed to disable autostart: {error}")),
            };
        }

        std::fs::create_dir_all(&autostart_dir)
            .map_err(|e| format!("Failed to create autostart directory: {e}"))?;

        let exe = super::linux_desktop::current_launcher_executable()?;
        let exec = super::linux_desktop::desktop_exec(&exe)?;
        let contents = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Reflow\n\
             Comment=Local dictation\n\
             Exec={exec}\n\
             Terminal=false\n\
             Categories=Utility;AudioVideo;\n\
             X-GNOME-Autostart-enabled=true\n\
             StartupNotify=false\n"
        );
        std::fs::write(&desktop_path, contents)
            .map_err(|e| format!("Failed to write autostart desktop file: {e}"))
    }
}

fn command_json(program: &str, args: &[&str]) -> Option<String> {
    super::process::output(
        program,
        args,
        std::time::Duration::from_millis(300),
        4 * 1024 * 1024,
    )
    .ok()
}

fn hyprland_active_window() -> Option<(String, String)> {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let json = command_json("hyprctl", &["activewindow", "-j"])?;
    parse_hyprctl_active_window(&json)
}

fn sway_active_window() -> Option<(String, String)> {
    std::env::var_os("SWAYSOCK")?;
    let json = command_json("swaymsg", &["-t", "get_tree"])?;
    parse_sway_focused(&json)
}

fn xdotool_paste(shift: bool) -> Result<(), String> {
    let chord = if shift { "ctrl+shift+v" } else { "ctrl+v" };
    super::process::output(
        "xdotool",
        &["key", "--clearmodifiers", chord],
        std::time::Duration::from_millis(500),
        1024,
    )?;
    Ok(())
}

fn wtype_paste(shift: bool) -> Result<(), String> {
    let args: &[&str] = if shift {
        &["-M", "ctrl", "-M", "shift", "v", "-m", "shift", "-m", "ctrl"]
    } else {
        &["-M", "ctrl", "v", "-m", "ctrl"]
    };
    super::process::output("wtype", args, std::time::Duration::from_millis(500), 1024)?;
    Ok(())
}

fn ydotool_paste(shift: bool) -> Result<(), String> {
    let args: &[&str] = if shift {
        &["key", "29:1", "42:1", "47:1", "47:0", "42:0", "29:0"]
    } else {
        &["key", "29:1", "47:1", "47:0", "29:0"]
    };
    super::process::output("ydotool", args, std::time::Duration::from_millis(500), 1024).map(|_| ())
}
