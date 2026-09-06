use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use arboard::Clipboard;
use serde::{Deserialize, Serialize};

use crate::platform::{self, paste_chord_label};

/// How long to wait for the target window to actually become foreground after
/// focus is requested. Long enough to cover the Windows foreground-lock
/// handshake, short enough that a genuinely unfocusable target degrades to the
/// clipboard fallback quickly.
const FOCUS_CONFIRM_TIMEOUT: Duration = Duration::from_millis(400);

/// How long the transcript stays on the clipboard before the user's previous
/// contents are put back. Electron and browser targets read the clipboard
/// asynchronously, well after the keystroke is delivered.
const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(900);

static INJECTION_GENERATION: AtomicU64 = AtomicU64::new(0);
static CLIPBOARD_OPERATION: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

fn may_restore_clipboard(
    generation: u64,
    current_generation: u64,
    current_text: Option<&str>,
    inserted_text: &str,
) -> bool {
    generation == current_generation && current_text == Some(inserted_text)
}

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

        let _clipboard_operation = CLIPBOARD_OPERATION.lock();
        let mut clipboard =
            Clipboard::new().map_err(|e| format!("Failed to open clipboard: {e}"))?;
        let previous_text = clipboard.get_text().ok();

        let generation = INJECTION_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        clipboard
            .set_text(text)
            .map_err(|e| format!("Failed to write to clipboard: {e}"))?;

        thread::sleep(Duration::from_millis(30));

        // Make sure the intended window really has the focus before
        // synthesizing the chord. `SetForegroundWindow` is asynchronous, so
        // pasting immediately after asking for focus is a race whose loser
        // sends Ctrl+V to whatever happened to be focused instead.
        if target_hwnd != 0 && !platform::focus_hwnd_and_confirm(target_hwnd, FOCUS_CONFIRM_TIMEOUT)
        {
            log::warn!(
                "Could not bring target window {target_hwnd} to the foreground; \
                 leaving the transcript on the clipboard for {paste_chord}"
            );
            return Ok(InjectionOutcome {
                app_title,
                process_name,
                pasted: false,
                fallback_copy: true,
                paste_chord,
            });
        }

        match platform::simulate_paste(&process_name) {
            Ok(()) => {
                // Give the target a moment to act on the keystroke before
                // checking where it landed.
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
                        // Restore late, and off the hot path. Electron-based
                        // targets (the chat apps this gets used in most) read
                        // the clipboard well after the keystroke is delivered;
                        // restoring at ~180ms raced them, so the user's previous
                        // clipboard got pasted instead of the transcript.
                        // Deferring also keeps the restore out of the dictation
                        // latency budget.
                        let inserted_text = text.to_string();
                        thread::spawn(move || {
                            thread::sleep(CLIPBOARD_RESTORE_DELAY);
                            let _clipboard_operation = CLIPBOARD_OPERATION.lock();
                            match Clipboard::new() {
                                Ok(mut clipboard) => {
                                    let current_text = clipboard.get_text().ok();
                                    if may_restore_clipboard(
                                        generation,
                                        INJECTION_GENERATION.load(Ordering::SeqCst),
                                        current_text.as_deref(),
                                        &inserted_text,
                                    ) {
                                        let _ = clipboard.set_text(prev);
                                    }
                                }
                                Err(err) => {
                                    log::warn!("Could not restore the clipboard: {err}")
                                }
                            }
                        });
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
    fn clipboard_restore_preserves_new_copies_and_later_injections() {
        assert!(may_restore_clipboard(1, 1, Some("dictation"), "dictation"));
        assert!(!may_restore_clipboard(
            1,
            1,
            Some("new user copy"),
            "dictation"
        ));
        assert!(!may_restore_clipboard(1, 2, Some("dictation"), "dictation"));
        assert!(!may_restore_clipboard(1, 1, None, "dictation"));
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
