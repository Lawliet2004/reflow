#[cfg(not(windows))]
use enigo::{Direction, Key};
use enigo::{Enigo, Keyboard, Settings};

fn keyboard_settings() -> Settings {
    #[cfg(target_os = "linux")]
    {
        super::adapter::linux_keyboard_settings(super::session())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Settings::default()
    }
}

pub fn paste_chord(shortcut: &str) -> Result<(), String> {
    if !matches!(shortcut, "ctrl_v" | "ctrl_shift_v") {
        return Err("Unknown paste shortcut".into());
    }
    let shift = shortcut == "ctrl_shift_v";
    #[cfg(target_os = "linux")]
    if super::session() == super::DisplaySession::Wayland {
        return super::linux_input_portal::chord(true, shift, 'v' as i32);
    }
    // Always attempt modifier releases, including partial press failures.
    let mut keyboard = Enigo::new(&keyboard_settings()).map_err(|error| error.to_string())?;
    let press = keyboard
        .key(enigo::Key::Control, enigo::Direction::Press)
        .and_then(|_| {
            if shift {
                keyboard.key(enigo::Key::Shift, enigo::Direction::Press)
            } else {
                Ok(())
            }
        })
        .and_then(|_| keyboard.key(enigo::Key::Unicode('v'), enigo::Direction::Click));
    let shift_release = if shift {
        keyboard.key(enigo::Key::Shift, enigo::Direction::Release)
    } else {
        Ok(())
    };
    let ctrl_release = keyboard.key(enigo::Key::Control, enigo::Direction::Release);
    press
        .and(shift_release)
        .and(ctrl_release)
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
fn chord(
    modifier: Option<windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY>,
    key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY,
) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    let input = |vk, up| INPUT {
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
    let mut inputs = Vec::new();
    if let Some(modifier) = modifier {
        inputs.push(input(modifier, false));
    }
    inputs.push(input(key, false));
    inputs.push(input(key, true));
    if let Some(modifier) = modifier {
        inputs.push(input(modifier, true));
    }
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent == inputs.len() as u32 {
        Ok(())
    } else {
        Err("Failed to send the full key chord".into())
    }
}

pub fn simulate_copy() -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_C, VK_CONTROL};
        chord(Some(VK_CONTROL), VK_C)
    }
    #[cfg(target_os = "linux")]
    {
        if super::session() == super::DisplaySession::Wayland {
            return super::linux_input_portal::chord(true, false, 'c' as i32);
        }
        let mut enigo = Enigo::new(&keyboard_settings()).map_err(|e| e.to_string())?;
        enigo
            .key(Key::Control, Direction::Press)
            .map_err(|e| e.to_string())?;
        let result = enigo
            .key(Key::Unicode('c'), Direction::Click)
            .map_err(|e| e.to_string());
        let release = enigo
            .key(Key::Control, Direction::Release)
            .map_err(|e| e.to_string());
        result.and(release)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Err("Selected-text capture is unavailable on this OS".into())
    }
}

pub fn send_key(send_key: &str) -> Result<(), String> {
    if !matches!(send_key, "enter" | "shift_enter" | "ctrl_enter") {
        return Err("Unknown send key".into());
    }
    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_CONTROL, VK_RETURN, VK_SHIFT};
        chord(
            match send_key {
                "shift_enter" => Some(VK_SHIFT),
                "ctrl_enter" => Some(VK_CONTROL),
                _ => None,
            },
            VK_RETURN,
        )
    }
    #[cfg(not(windows))]
    {
        #[cfg(target_os = "linux")]
        if super::session() == super::DisplaySession::Wayland {
            return super::linux_input_portal::chord(
                send_key == "ctrl_enter",
                send_key == "shift_enter",
                0xff0d,
            );
        }
        let mut enigo = Enigo::new(&keyboard_settings()).map_err(|e| e.to_string())?;
        let modifier = match send_key {
            "shift_enter" => Some(Key::Shift),
            "ctrl_enter" => Some(Key::Control),
            _ => None,
        };
        if let Some(key) = modifier {
            enigo
                .key(key, Direction::Press)
                .map_err(|e| e.to_string())?;
        }
        let result = enigo
            .key(Key::Return, Direction::Click)
            .map_err(|e| e.to_string());
        if let Some(key) = modifier {
            enigo
                .key(key, Direction::Release)
                .map_err(|e| e.to_string())?;
        }
        result
    }
}

pub fn type_text(text: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if super::session() == super::DisplaySession::Wayland {
        return super::linux_input_portal::type_text(text);
    }
    Enigo::new(&keyboard_settings())
        .map_err(|e| e.to_string())?
        .text(text)
        .map_err(|e| e.to_string())
}
