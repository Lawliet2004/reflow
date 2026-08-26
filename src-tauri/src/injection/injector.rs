use std::thread;
use std::time::Duration;

use arboard::Clipboard;
use serde::{Deserialize, Serialize};

use crate::platform::{self, paste_chord_label};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectionOutcome {
    pub app_title: String,
    pub process_name: String,
    pub pasted: bool,
    pub fallback_copy: bool,
    pub paste_chord: String,
}

pub struct TextInjector;

impl TextInjector {
    pub fn get_active_app() -> (String, String) {
        platform::active_window()
    }

    /// `target_hwnd`: the HWND captured when the user pressed the hotkey
    /// (i.e. the window we *want* to paste into). After `simulate_paste`
    /// we re-read the foreground window; if it does not match the target,
    /// the synthesized Ctrl+V almost certainly went to a different window
    /// (Reflow's overlay, the system tray, etc.) and the transcript is
    /// not in the text box. In that case we return `fallback_copy: true`
    /// and **leave the transcript on the clipboard** so the user's manual
    /// Ctrl+V still pastes the right thing. The overlay shows
    /// "Copied — press Ctrl+V" so the user knows what to do.
    pub fn inject(
        text: &str,
        restore_clipboard: bool,
        target_hwnd: isize,
    ) -> Result<InjectionOutcome, String> {
        let (app_title, process_name) = platform::active_window();
        let paste_chord = paste_chord_label(&process_name, platform::session()).to_string();

        if text.is_empty() {
            return Ok(InjectionOutcome {
                app_title,
                process_name,
                pasted: true,
                fallback_copy: false,
                paste_chord,
            });
        }

        let mut clipboard = Clipboard::new()
            .map_err(|e| format!("Failed to open clipboard: {e}"))?;
        let previous_text = clipboard.get_text().ok();

        clipboard
            .set_text(text)
            .map_err(|e| format!("Failed to write to clipboard: {e}"))?;

        thread::sleep(Duration::from_millis(30));

        match platform::simulate_paste(&process_name) {
            Ok(()) => {
                // Target apps often read the clipboard asynchronously; restoring
                // too soon pastes the user's previous clipboard instead.
                thread::sleep(Duration::from_millis(180));
                // Verify the synthesized Ctrl+V actually went to the
                // intended target BEFORE we restore the previous clipboard.
                // When no target was captured (companion / Android path),
                // skip the check.
                let foreground_now = platform::foreground_hwnd();
                let delivered = target_hwnd == 0 || foreground_now == target_hwnd;
                if !delivered {
                    log::warn!(
                        "Foreground changed during paste (target_hwnd={target_hwnd}, now={foreground_now}); transcript is left on the clipboard for {paste_chord}"
                    );
                    return Ok(InjectionOutcome {
                        app_title,
                        process_name,
                        pasted: false,
                        fallback_copy: true,
                        paste_chord,
                    });
                }
                if restore_clipboard {
                    if let Some(prev) = previous_text {
                        let _ = clipboard.set_text(prev);
                    }
                }
                Ok(InjectionOutcome {
                    app_title,
                    process_name,
                    pasted: true,
                    fallback_copy: false,
                    paste_chord,
                })
            }
            Err(err) => {
                log::warn!(
                    "Paste simulation failed ({err}); leaving transcript on the clipboard for {paste_chord}"
                );
                Ok(InjectionOutcome {
                    app_title,
                    process_name,
                    pasted: false,
                    fallback_copy: true,
                    paste_chord,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{linux_terminal_process, paste_chord_label, DisplaySession};

    #[test]
    fn empty_inject_is_noop_success() {
        let outcome = TextInjector::inject("", true, 0).expect("empty inject should succeed");
        assert!(outcome.pasted);
        assert!(!outcome.fallback_copy);
    }

    #[test]
    fn chord_helpers_are_stable() {
        assert!(linux_terminal_process("alacritty"));
        assert_eq!(
            paste_chord_label("alacritty", DisplaySession::Wayland),
            "Ctrl+Shift+V"
        );
    }
}
