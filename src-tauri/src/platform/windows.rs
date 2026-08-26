use std::path::Path;

use super::adapter::{DisplaySession, PlatformAdapter};

pub struct WindowsAdapter;

impl PlatformAdapter for WindowsAdapter {
    fn session() -> DisplaySession {
        DisplaySession::Windows
    }

    fn os_display_name() -> String {
        "Windows".into()
    }

    fn active_window() -> (String, String) {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::Foundation::MAX_PATH;
            use windows::Win32::System::ProcessStatus::GetProcessImageFileNameW;
            use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
            use windows::Win32::UI::WindowsAndMessaging::{
                GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
            };

            let hwnd = GetForegroundWindow();
            if hwnd.0.is_null() {
                return ("Desktop".into(), "explorer.exe".into());
            }

            let mut title_buf = [0u16; 512];
            let title_len = GetWindowTextW(hwnd, &mut title_buf);
            let title = if title_len > 0 {
                String::from_utf16_lossy(&title_buf[..title_len as usize])
            } else {
                "Unknown".into()
            };

            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let mut proc_name = "Unknown".to_string();

            if let Ok(process_handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                let mut path_buf = [0u16; MAX_PATH as usize];
                let path_len = GetProcessImageFileNameW(process_handle, &mut path_buf);
                if path_len > 0 {
                    let full_path = String::from_utf16_lossy(&path_buf[..path_len as usize]);
                    if let Some(filename) = full_path.split('\\').last() {
                        proc_name = filename.to_string();
                    }
                }
            }

            (title, proc_name)
        }
        #[cfg(not(windows))]
        ("Unknown".into(), "unknown".into())
    }

    fn simulate_paste(_process: &str) -> Result<(), String> {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::UI::Input::KeyboardAndMouse::{
                SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
                KEYEVENTF_KEYUP, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN,
                VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_V,
            };

            let key = |vk, up: bool| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: vk,
                        wScan: 0,
                        dwFlags: if up {
                            KEYEVENTF_KEYUP
                        } else {
                            KEYBD_EVENT_FLAGS(0)
                        },
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };

            // Release leftover Shift/Win/Alt from the PTT combo so Ctrl+V is
            // not turned into Win+Ctrl+V.
            let inputs = [
                key(VK_LWIN, true),
                key(VK_RWIN, true),
                key(VK_LSHIFT, true),
                key(VK_RSHIFT, true),
                key(VK_SHIFT, true),
                key(VK_LMENU, true),
                key(VK_RMENU, true),
                key(VK_MENU, true),
                key(VK_LCONTROL, true),
                key(VK_RCONTROL, true),
                key(VK_CONTROL, false),
                key(VK_V, false),
                key(VK_V, true),
                key(VK_CONTROL, true),
            ];

            let sent = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
            if sent != inputs.len() as u32 {
                return Err("Failed to send full Ctrl+V keystroke input".into());
            }
            Ok(())
        }
        #[cfg(not(windows))]
        Err("Windows paste simulation is unavailable on this OS".into())
    }

    fn open_path(path: &Path) -> Result<(), String> {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Failed to open folder: {e}"))
    }

    fn set_launch_at_startup(enabled: bool) -> Result<(), String> {
        let Some(startup_dir) = dirs::home_dir().map(|home| {
            home.join("AppData")
                .join("Roaming")
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
                .join("Startup")
        }) else {
            return Err("Could not resolve Windows Startup folder".into());
        };

        let launcher = startup_dir.join("Reflow.cmd");
        if !enabled {
            let _ = std::fs::remove_file(&launcher);
            return Ok(());
        }

        std::fs::create_dir_all(&startup_dir)
            .map_err(|e| format!("Failed to create Startup folder: {e}"))?;

        let exe = std::env::current_exe().map_err(|e| format!("current_exe failed: {e}"))?;
        let script = format!("@echo off\r\nstart \"\" \"{}\"\r\n", exe.display());
        std::fs::write(&launcher, script).map_err(|e| format!("Failed to write startup launcher: {e}"))
    }

    fn foreground_hwnd() -> isize {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
            GetForegroundWindow().0 as isize
        }
        #[cfg(not(windows))]
        0
    }

    fn focus_hwnd(hwnd: isize) -> bool {
        if hwnd == 0 {
            return false;
        }
        #[cfg(windows)]
        unsafe {
            use windows::Win32::Foundation::HWND;
            use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
            use windows::Win32::UI::WindowsAndMessaging::{
                AllowSetForegroundWindow, GetForegroundWindow, GetWindowThreadProcessId,
                SetForegroundWindow,
            };

            let target = HWND(hwnd as *mut core::ffi::c_void);
            let fg = GetForegroundWindow();
            if fg == target {
                return true;
            }
            let mut fg_pid = 0u32;
            let mut target_pid = 0u32;
            let fg_thread = GetWindowThreadProcessId(fg, Some(&mut fg_pid));
            let target_thread = GetWindowThreadProcessId(target, Some(&mut target_pid));
            let cur = GetCurrentThreadId();
            // Permit the target process to take the foreground from us.
            // Without this, Windows 10/11 silently denies SetForegroundWindow
            // when the caller is a background process (the foreground-lock
            // timeout), and our simulated Ctrl+V is delivered to the wrong
            // window.
            let _ = AllowSetForegroundWindow(target_pid);
            let _ = AttachThreadInput(cur, fg_thread, true);
            let _ = AttachThreadInput(cur, target_thread, true);
            let ok = SetForegroundWindow(target).as_bool();
            let _ = AttachThreadInput(cur, fg_thread, false);
            let _ = AttachThreadInput(cur, target_thread, false);
            let result = ok || GetForegroundWindow() == target;
            log::info!(
                "focus_hwnd({hwnd}) fg_pid={fg_pid} target_pid={target_pid} -> {result}"
            );
            result
        }
        #[cfg(not(windows))]
        {
            let _ = hwnd;
            false
        }
    }
}
