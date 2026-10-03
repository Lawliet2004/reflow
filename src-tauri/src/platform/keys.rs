#[cfg(not(windows))]
use enigo::{Direction, Key};
use enigo::{Enigo, Keyboard, Settings};

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
        let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
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
        let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
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
    Enigo::new(&Settings::default())
        .map_err(|e| e.to_string())?
        .text(text)
        .map_err(|e| e.to_string())
}
