//! Windows UI Automation stays on its own MTA thread. No clipboard reads,
//! screenshots, global key logging, or changes to accessibility settings.
//! https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-threading
use crate::{
    context::AppContext,
    correction_observer::{record_observed_edit, TrackedDictation, MAX_FIELD_CHARS},
    settings::AppSettings,
};
use std::{
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};
use windows::{
    core::Interface,
    Win32::{
        Foundation::HWND,
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_MULTITHREADED,
        },
        UI::{
            Accessibility::{
                CUIAutomation8, IUIAutomation, IUIAutomation2, IUIAutomationElement,
                IUIAutomationTextPattern, IUIAutomationValuePattern, UIA_DocumentControlTypeId,
                UIA_EditControlTypeId, UIA_IsReadOnlyAttributeId, UIA_TextPatternId,
                UIA_ValuePatternId,
            },
            WindowsAndMessaging::GetWindowThreadProcessId,
        },
    },
};

struct ComApartment;
impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

pub fn start(ctx: AppContext, hwnd: isize, inserted: String, history_id: Option<String>) {
    if hwnd == 0 || !ctx.settings_store.get().auto_learn_dictionary {
        return;
    }
    let generation = ctx
        .correction_watch_generation
        .fetch_add(1, Ordering::SeqCst)
        + 1;
    let _ = thread::Builder::new()
        .name("dictation-corrections".into())
        .spawn(move || {
            // A broken/unsupported provider must never affect insertion or recording.
            if observe(ctx, hwnd, inserted, history_id, generation).is_err() {
                log::debug!("Automatic spelling learning is unavailable for this text field");
            }
        });
}

unsafe fn read_field(element: &IUIAutomationElement) -> windows::core::Result<Option<String>> {
    // Check password protection before requesting ANY field text, on every read.
    if element.CurrentIsPassword()?.as_bool()
        || !element.CurrentIsEnabled()?.as_bool()
        || element.CurrentIsOffscreen()?.as_bool()
    {
        return Ok(None);
    }
    let kind = element.CurrentControlType()?;
    if kind != UIA_EditControlTypeId && kind != UIA_DocumentControlTypeId {
        return Ok(None);
    }
    let text = if let Ok(pattern) =
        element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
    {
        let range = pattern.DocumentRange()?;
        let readonly = range.GetAttributeValue(UIA_IsReadOnlyAttributeId)?;
        if bool::try_from(&readonly).unwrap_or(true) {
            return Ok(None);
        }
        range.GetText(MAX_FIELD_CHARS as i32 + 1)?.to_string()
    } else {
        let pattern =
            element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)?;
        if pattern.CurrentIsReadOnly()?.as_bool() {
            return Ok(None);
        }
        pattern.CurrentValue()?.to_string()
    };
    if text.chars().count() > MAX_FIELD_CHARS {
        return Ok(None);
    }
    Ok(Some(text.replace("\r\n", "\n").replace('\r', "\n")))
}

fn observe(
    ctx: AppContext,
    hwnd: isize,
    inserted: String,
    history_id: Option<String>,
    generation: u64,
) -> windows::core::Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let _apartment = ComApartment;
        let automation2: IUIAutomation2 =
            CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
        automation2.SetConnectionTimeout(200)?;
        automation2.SetTransactionTimeout(500)?;
        let automation: IUIAutomation = automation2.cast()?;
        let mut pid = 0;
        GetWindowThreadProcessId(HWND(hwnd as *mut _), Some(&mut pid));
        if pid == 0 || pid == std::process::id() {
            return Ok(());
        }
        if crate::platform::foreground_hwnd() != hwnd {
            return Ok(());
        }
        let (_, process) = crate::platform::active_window();
        let allowed = || {
            let settings = ctx.settings_store.get();
            ctx.correction_watch_generation.load(Ordering::SeqCst) == generation
                && settings.auto_learn_dictionary
                && !AppSettings::process_is_excluded(&process, &settings.excluded_apps)
        };
        let active = || allowed() && crate::platform::foreground_hwnd() == hwnd;
        let inserted = inserted.replace("\r\n", "\n").replace('\r', "\n");
        let initialize_until = Instant::now() + Duration::from_secs(2);
        let (element, mut tracked) = loop {
            if !active() || Instant::now() >= initialize_until {
                return Ok(());
            }
            let element = automation.GetFocusedElement()?;
            if element.CurrentProcessId()? as u32 != pid {
                return Ok(());
            }
            let Some(field) = read_field(&element)? else {
                return Ok(());
            };
            if let Some(tracked) = TrackedDictation::new(&field, &inserted) {
                break (element, tracked);
            }
            // Browser pastes are asynchronous; wait for the actual inserted text.
            thread::sleep(Duration::from_millis(100));
        };
        let expires = Instant::now() + Duration::from_secs(120);
        let mut candidate = tracked.text.clone();
        let mut changed_at = Instant::now();
        while active() && Instant::now() < expires {
            thread::sleep(Duration::from_millis(250));
            if !active() {
                break;
            }
            let focused = automation.GetFocusedElement()?;
            if !automation.CompareElements(&element, &focused)?.as_bool() {
                break;
            }
            let Some(field) = read_field(&element)? else {
                break;
            };
            let Some(text) = tracked.extract(&field) else {
                break;
            };
            if text != candidate {
                candidate = text;
                changed_at = Instant::now();
            } else if candidate != tracked.text
                && changed_at.elapsed() >= Duration::from_millis(1500)
            {
                // Quiet typing, not an intermediate deletion/retyping state.
                if !active() {
                    break;
                }
                match record_observed_edit(&ctx, history_id.as_deref(), &tracked.text, &candidate) {
                    Ok(true) => tracked.text = candidate.clone(),
                    _ => break,
                }
            }
        }
        // Leaving a field is also an editing boundary. Read the original element
        // once more so a completed correction is not lost during the debounce.
        // Never flush after opt-out, exclusion, or a new dictation. A cleared or
        // unanchored field cannot prove what was submitted and is ignored.
        if allowed() {
            if let Some(text) = read_field(&element)?.and_then(|field| tracked.extract(&field)) {
                if text != tracked.text {
                    let _ = record_observed_edit(&ctx, history_id.as_deref(), &tracked.text, &text);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{
        core::w,
        Win32::{
            Foundation::{LPARAM, WPARAM},
            System::Threading::GetCurrentThreadId,
            UI::WindowsAndMessaging::*,
        },
    };

    // Own controls only; never reads or types into any user's application.
    struct EditFixture {
        thread_id: u32,
        controls: Vec<isize>,
        thread: Option<thread::JoinHandle<()>>,
    }
    impl EditFixture {
        fn new() -> Self {
            let (sender, receiver) = std::sync::mpsc::channel();
            let thread = thread::spawn(move || unsafe {
                let parent = CreateWindowExW(
                    WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                    w!("STATIC"),
                    w!("Reflow dictionary test fixture"),
                    WS_OVERLAPPEDWINDOW,
                    100,
                    100,
                    360,
                    200,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
                let mut controls = Vec::new();
                for (index, style) in [ES_MULTILINE, ES_READONLY, ES_PASSWORD].iter().enumerate() {
                    let edit = CreateWindowExW(
                        WINDOW_EX_STYLE::default(),
                        w!("EDIT"),
                        w!("Use type script."),
                        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(*style as u32),
                        10,
                        10 + index as i32 * 45,
                        320,
                        35,
                        parent,
                        None,
                        None,
                        None,
                    )
                    .unwrap();
                    controls.push(edit.0 as isize);
                }
                let _ = ShowWindow(parent, SW_SHOWNOACTIVATE);
                sender.send((GetCurrentThreadId(), controls)).unwrap();
                let mut message = MSG::default();
                while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                DestroyWindow(parent).unwrap();
            });
            let (thread_id, controls) = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
            Self {
                thread_id,
                controls,
                thread: Some(thread),
            }
        }
    }
    impl Drop for EditFixture {
        fn drop(&mut self) {
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            let _ = self.thread.take().unwrap().join();
        }
    }

    #[test]
    #[ignore = "requires an interactive Windows desktop; creates its own temporary controls"]
    fn native_uia_reads_edits_and_rejects_password_and_readonly_fields() {
        let fixture = EditFixture::new();
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap();
            let _apartment = ComApartment;
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).unwrap();
            let editable = HWND(fixture.controls[0] as *mut _);
            let element = automation.ElementFromHandle(editable).unwrap();
            let before = read_field(&element).unwrap().unwrap();
            assert_eq!(before, "Use type script.");
            let tracked = TrackedDictation::new(&before, &before).unwrap();
            SetWindowTextW(editable, w!("Use TypeScript.")).unwrap();
            let after = tracked
                .extract(&read_field(&element).unwrap().unwrap())
                .unwrap();
            assert_eq!(after, "Use TypeScript.");
            assert_eq!(
                crate::dictionary::correction_suggestions(&before, &after).len(),
                1
            );
            for handle in &fixture.controls[1..] {
                let element = automation
                    .ElementFromHandle(HWND(*handle as *mut _))
                    .unwrap();
                assert!(read_field(&element).unwrap().is_none());
            }
        }
    }
}
