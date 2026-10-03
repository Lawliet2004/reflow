use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::clipboard::{ClipboardAccess, ClipboardSnapshot};
use super::restore_chain::RestoreChain;

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
static CLIPBOARD_CHAIN: OnceLock<parking_lot::Mutex<RestoreChain<ClipboardSnapshot>>> =
    OnceLock::new();
static RESTORE_ERROR: parking_lot::Mutex<Option<String>> = parking_lot::Mutex::new(None);

fn clipboard_chain() -> &'static parking_lot::Mutex<RestoreChain<ClipboardSnapshot>> {
    CLIPBOARD_CHAIN.get_or_init(|| parking_lot::Mutex::new(RestoreChain::default()))
}

fn bounded_restore_error(message: &str) -> String {
    message.chars().take(2_048).collect()
}

fn record_restore_error(message: &str) {
    log::warn!("Could not restore the clipboard: {message}");
    *RESTORE_ERROR.lock() = Some(bounded_restore_error(message));
}

fn restore_generation(generation: u64) -> Result<bool, String> {
    let _clipboard_operation = CLIPBOARD_OPERATION.lock();
    let mut clipboard = ClipboardAccess::open()?;
    let current_text = clipboard.text();
    let previous = clipboard_chain().lock().candidate_if_unchanged(
        generation,
        current_text.as_deref(),
        clipboard.sequence(),
    );
    let Some(previous) = previous else {
        return Ok(false);
    };
    match clipboard.restore(&previous) {
        Ok(()) => {
            clipboard_chain().lock().complete_restore(generation);
            Ok(true)
        }
        Err(err) => {
            let current_text = clipboard.text();
            clipboard_chain().lock().note_failed_restore(
                generation,
                current_text,
                clipboard.sequence(),
            );
            Err(err)
        }
    }
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

fn changed_selection(
    previous_text: Option<&str>,
    previous_sequence: Option<u32>,
    text: Option<String>,
    sequence: Option<u32>,
) -> Option<String> {
    let changed = match (previous_sequence, sequence) {
        (Some(a), Some(b)) => a != b,
        _ => previous_text != text.as_deref(),
    };
    text.filter(|s| changed && !s.trim().is_empty())
}

pub fn clipboard_text() -> Result<Option<String>, String> {
    let _operation = CLIPBOARD_OPERATION.lock();
    Ok(ClipboardAccess::open()?.text())
}

/// Hold our operation lock, but release the OS clipboard lock before Ctrl+C.
pub fn capture_selection() -> Result<Option<String>, String> {
    let _operation = CLIPBOARD_OPERATION.lock();
    #[cfg(windows)]
    let clipboard = ClipboardAccess::open()?;
    #[cfg(not(windows))]
    let mut clipboard = ClipboardAccess::open()?;
    let snapshot = clipboard.snapshot()?;
    let before = clipboard.text();
    let sequence = clipboard.sequence();
    drop(clipboard);
    platform::simulate_copy()?;
    let started = std::time::Instant::now();
    let mut last_error = None;
    while started.elapsed() < Duration::from_millis(400) {
        thread::sleep(Duration::from_millis(20));
        match ClipboardAccess::open() {
            Ok(mut clipboard) => {
                let text = clipboard.text();
                let now = clipboard.sequence();
                let result = changed_selection(before.as_deref(), sequence, text.clone(), now);
                if result.is_some() || (sequence.is_some() && sequence != now) || before != text {
                    clipboard.restore(&snapshot)?;
                    // A pending injection restore must not overwrite this snapshot.
                    clipboard_chain().lock().discard();
                    return Ok(result);
                }
            }
            Err(err) => last_error = Some(err),
        }
    }
    if let Some(error) = last_error {
        return Err(error);
    }
    Ok(None)
}

impl TextInjector {
    pub fn deliver(
        text: &str,
        action: &crate::settings::OutputAction,
        target_hwnd: isize,
    ) -> Result<InjectionOutcome, String> {
        Self::deliver_with_options(text, action, target_hwnd, true, "enter")
    }

    pub fn deliver_with_options(
        text: &str,
        action: &crate::settings::OutputAction,
        target_hwnd: isize,
        restore: bool,
        send_key: &str,
    ) -> Result<InjectionOutcome, String> {
        Self::deliver_configured(text, action, target_hwnd, restore, send_key, 30, "paste")
    }

    pub fn deliver_configured(
        text: &str,
        action: &crate::settings::OutputAction,
        target_hwnd: isize,
        restore: bool,
        send_key: &str,
        delay_ms: u64,
        method: &str,
    ) -> Result<InjectionOutcome, String> {
        use crate::settings::OutputAction;
        use std::io::Write;
        if matches!(action, OutputAction::Paste | OutputAction::PasteEnter) {
            let mut outcome =
                Self::inject_configured(text, restore, target_hwnd, delay_ms, method)?;
            if matches!(action, OutputAction::PasteEnter) && outcome.pasted && !text.is_empty() {
                thread::sleep(Duration::from_millis(120));
                // Never send Enter to a window that stole focus after the paste.
                if (target_hwnd != 0 && platform::foreground_hwnd() != target_hwnd)
                    || platform::send_key(send_key).is_err()
                {
                    Self::deliver(text, &OutputAction::Copy, 0)?;
                    outcome.pasted = false;
                    outcome.fallback_copy = true;
                }
            }
            return Ok(outcome);
        }
        let (app_title, process_name) = platform::active_window();
        let result = InjectionOutcome {
            app_title,
            process_name,
            pasted: false,
            fallback_copy: false,
            paste_chord: String::new(),
        };
        match action {
            OutputAction::Copy => {
                let _operation = CLIPBOARD_OPERATION.lock();
                let mut clipboard = ClipboardAccess::open()?;
                clipboard.set_text(text)?;
                INJECTION_GENERATION.fetch_add(1, Ordering::SeqCst);
                clipboard_chain().lock().discard();
            }
            OutputAction::Hud => {}
            OutputAction::AppendFile { path } => {
                if path.trim().is_empty() {
                    return Err("Choose an output file".into());
                }
                let path = std::path::Path::new(path);
                if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|e| e.to_string())?;
                writeln!(file, "{text}").map_err(|e| e.to_string())?;
            }
            OutputAction::RunCommand { template } => run_output_command(template, text)?,
            OutputAction::Paste | OutputAction::PasteEnter => unreachable!(),
        }
        Ok(result)
    }
    /// Latest delayed-restoration failure, without clipboard content. Polling
    /// consumes the bounded diagnostic; the original snapshot remains retained.
    pub fn take_restore_error() -> Option<String> {
        RESTORE_ERROR.lock().take()
    }

    /// Explicit recovery action; refuses to overwrite a new user copy.
    pub fn retry_clipboard_restore() -> Result<bool, String> {
        let generation = clipboard_chain().lock().generation();
        let Some(generation) = generation else {
            return Ok(false);
        };
        match restore_generation(generation) {
            Ok(restored) => Ok(restored),
            Err(err) => {
                record_restore_error(&err);
                Err(err)
            }
        }
    }

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
        Self::inject_configured(text, restore_clipboard, target_hwnd, 30, "paste")
    }

    pub fn inject_configured(
        text: &str,
        restore_clipboard: bool,
        target_hwnd: isize,
        delay_ms: u64,
        method: &str,
    ) -> Result<InjectionOutcome, String> {
        if delay_ms > 2000 || !matches!(method, "paste" | "type") {
            return Err("Invalid paste delay or insertion method".into());
        }
        let (app_title, process_name) = platform::active_window();
        if method == "type" && !text.is_empty() {
            thread::sleep(Duration::from_millis(delay_ms));
            if target_hwnd != 0
                && !platform::focus_hwnd_and_confirm(target_hwnd, FOCUS_CONFIRM_TIMEOUT)
            {
                return Err("Could not focus the target window for typing".into());
            }
            platform::type_text(text)?;
            return Ok(InjectionOutcome {
                app_title,
                process_name,
                pasted: true,
                fallback_copy: false,
                paste_chord: String::new(),
            });
        }
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
        let mut clipboard = ClipboardAccess::open()?;
        let previous = if restore_clipboard {
            let observed = clipboard.snapshot()?;
            let current_text = clipboard.text();
            Some(clipboard_chain().lock().original_for_injection(
                observed,
                current_text.as_deref(),
                clipboard.sequence(),
            ))
        } else {
            None
        };
        if let Err(err) = clipboard.set_text(text) {
            // Native APIs can fail after EmptyClipboard. Keep the original
            // snapshot even when the initial transcript write only completed
            // partially, and record our resulting clipboard sequence.
            if let Some(previous) = previous {
                let generation = INJECTION_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
                let current_text = clipboard.text();
                let sequence = clipboard.sequence();
                let mut chain = clipboard_chain().lock();
                chain.mark_injected(previous, String::new(), sequence, generation);
                chain.note_failed_restore(generation, current_text, sequence);
                record_restore_error(&err);
            }
            return Err(err);
        }
        let sequence = clipboard.sequence();
        let generation = INJECTION_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(previous) = previous {
            clipboard_chain().lock().mark_injected(
                previous,
                text.to_string(),
                sequence,
                generation,
            );
        } else {
            clipboard_chain().lock().discard();
        }
        // Paste targets must be able to open the clipboard before the chord.
        drop(clipboard);

        thread::sleep(Duration::from_millis(delay_ms));

        // Make sure the intended window really has the focus before
        // synthesizing the chord. `SetForegroundWindow` is asynchronous, so
        // pasting immediately after asking for focus is a race whose loser
        // sends Ctrl+V to whatever happened to be focused instead.
        if target_hwnd != 0 && !platform::focus_hwnd_and_confirm(target_hwnd, FOCUS_CONFIRM_TIMEOUT)
        {
            clipboard_chain().lock().discard();
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
                    clipboard_chain().lock().discard();
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
                    // Restore late, and off the hot path. Electron-based
                    // targets (the chat apps this gets used in most) read
                    // the clipboard well after the keystroke is delivered;
                    // restoring at ~180ms raced them, so the user's previous
                    // clipboard got pasted instead of the transcript.
                    // Deferring also keeps the restore out of the dictation
                    // latency budget.
                    thread::spawn(move || {
                        thread::sleep(CLIPBOARD_RESTORE_DELAY);
                        if let Err(err) = restore_generation(generation) {
                            record_restore_error(&err);
                        }
                    });
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
                clipboard_chain().lock().discard();
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

fn run_output_command(template: &str, text: &str) -> Result<(), String> {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    if template.trim().is_empty() {
        return Err("Command template must not be empty".into());
    }
    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        // Delayed expansion treats the environment value as data, not shell syntax.
        let mut command = Command::new("cmd");
        command.args([
            "/D",
            "/V:ON",
            "/C",
            &template.replace("{text}", "\"!REFLOW_TEXT!\""),
        ]);
        command.env("REFLOW_TEXT", text).creation_flags(0x08000000);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("sh");
        let quoted = format!("'{}'", text.replace('\'', "'\\''"));
        command.args(["-c", &template.replace("{text}", &quoted)]);
        command
    };
    let mut child = command
        .stdin(if template.contains("{text}") {
            Stdio::null()
        } else {
            Stdio::piped()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let text = text.to_owned();
    std::thread::spawn(move || {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        if let Some(mut stderr) = child.stderr.take() {
            let mut message = String::new();
            let _ = stderr.by_ref().take(4096).read_to_string(&mut message);
            let _ = std::io::copy(&mut stderr, &mut std::io::sink());
            if !message.trim().is_empty() {
                log::warn!("Output command stderr: {message}");
            }
        }
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{linux_terminal_process, paste_chord_label, DisplaySession};

    #[test]
    fn selection_change_requires_new_nonempty_clipboard() {
        assert!(changed_selection(Some("old"), Some(1), Some("old".into()), Some(1)).is_none());
        assert_eq!(
            changed_selection(Some("old"), Some(1), Some("old".into()), Some(2)).as_deref(),
            Some("old")
        );
        assert!(changed_selection(Some("old"), None, Some("old".into()), None).is_none());
        assert!(changed_selection(Some("old"), Some(1), Some("".into()), Some(2)).is_none());
    }

    #[test]
    fn hud_does_not_need_clipboard_and_file_output_appends() {
        use crate::settings::OutputAction;
        assert!(
            !TextInjector::deliver("answer", &OutputAction::Hud, 0)
                .unwrap()
                .pasted
        );
        let path = std::env::temp_dir().join(format!("reflow-output-{}.txt", uuid::Uuid::new_v4()));
        let action = OutputAction::AppendFile {
            path: path.to_string_lossy().into(),
        };
        TextInjector::deliver("one", &action, 0).unwrap();
        TextInjector::deliver("two", &action, 0).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        std::fs::remove_file(path).unwrap();
    }

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

    #[test]
    fn restoration_error_is_bounded_without_breaking_unicode() {
        assert_eq!(bounded_restore_error("clipboard busy"), "clipboard busy");
        let message = "失敗".repeat(4_000);
        let bounded = bounded_restore_error(&message);
        assert_eq!(bounded.chars().count(), 2_048);
        assert!(message.starts_with(&bounded));
    }

    #[test]
    fn restore_error_poll_consumes_only_the_latest_error() {
        record_restore_error("old failure");
        record_restore_error("latest failure");
        assert_eq!(
            TextInjector::take_restore_error().as_deref(),
            Some("latest failure")
        );
        assert_eq!(TextInjector::take_restore_error(), None);
    }
}
