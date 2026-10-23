//! Low-level keyboard hook for modifier-only shortcut transitions (Shift+Win).
//!
//! Windows' RegisterHotKey API cannot register a hotkey made purely of
//! modifier keys, so combos like Shift+Win are tracked with a
//! WH_KEYBOARD_LL hook instead.
//!
//! Hold and double-tap gestures are handled by the shared shortcut dispatcher.
//!
//! The hook is transparent — it never swallows keys — except the Win
//! key-up right after a dictation, which is suppressed so the Start
//! menu doesn't open when the combo is released.

use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
use std::time::{Duration, Instant};

/// `true` while a hands-free (double-tap) session is running.
///
/// Set by the shared shortcut dispatcher on the second tap. The session layer
/// consults this to decide whether the hold-to-talk duration guard still applies.
static HANDS_FREE: AtomicBool = AtomicBool::new(false);

/// `true` when the current dictation was locked on with a double-tap.
///
/// A held key can get stuck — a missed key-up would otherwise record forever —
/// so hold-to-talk keeps a duration cap. A hands-free session was deliberately
/// started and is deliberately ended, so it has no such cap: the user asked for
/// hours, and cutting them off at a timeout would lose the recording.
pub fn hands_free_engaged() -> bool {
    HANDS_FREE.load(Ordering::Acquire)
}

pub(crate) fn set_hands_free(engaged: bool) {
    HANDS_FREE.store(engaged, Ordering::Release);
}

#[cfg(windows)]
#[derive(Clone, Copy)]
enum StateMachineAction {
    None,
    StartRecording,
    StopRecording,
}

/// Which entries of a held-key set survive live-state reconciliation in the
/// Windows hook. Pure (no Win32 calls) so it is unit-testable everywhere:
/// `is_modifier` identifies keys tracked for the combo and `physically_down`
/// is `GetAsyncKeyState`'s down bit. A modifier whose key-down the hook
/// swallowed never reaches the OS key state, so it would read as "up" while
/// still physically held — entries in `swallowed` are exempt.
#[cfg_attr(not(windows), allow(dead_code))] // Only the Windows hook calls this outside tests.
fn reconcile_held(
    down: &mut std::collections::HashSet<i32>,
    swallowed: &std::collections::HashSet<i32>,
    is_modifier: impl Fn(i32) -> bool,
    physically_down: impl Fn(i32) -> bool,
) {
    down.retain(|&vk| !is_modifier(vk) || swallowed.contains(&vk) || physically_down(vk));
}

#[cfg(windows)]
pub mod platform {
    use super::*;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};

    use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, SetWindowsHookExW, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG,
        WH_KEYBOARD_LL,
    };

    const WM_KEYDOWN: u32 = 0x0100;
    const WM_KEYUP: u32 = 0x0101;
    const WM_SYSKEYDOWN: u32 = 0x0104;
    const WM_SYSKEYUP: u32 = 0x0105;
    const LLKHF_INJECTED: u32 = 0x10;

    /// Escape hatch so automated tests can drive the hook with injected
    /// keys; injected events are ignored otherwise.
    fn allow_injected() -> bool {
        static FLAG: OnceLock<bool> = OnceLock::new();
        *FLAG.get_or_init(|| {
            std::env::var("REFLOW_TEST_INJECTED")
                .map(|v| v == "1")
                .unwrap_or(false)
        })
    }

    const MOD_SHIFT: u8 = 1;
    const MOD_CTRL: u8 = 2;
    const MOD_ALT: u8 = 4;
    const MOD_WIN: u8 = 8;

    /// The `Instant` argument is the moment the key event was observed
    /// *inside the hook callback*, before any locking, thread spawn,
    /// `SendInput` or tokio scheduling. Passing it through is what makes
    /// `hotkey_to_recording_ms` an honest measurement instead of zero.
    pub type ComboFn = Arc<dyn Fn(Instant) + Send + Sync + 'static>;

    struct HookState {
        required: u8,
        on_combo: ComboFn,
        on_release: ComboFn,
        down: HashSet<i32>,
        swallowed: HashSet<i32>,
        active: bool,
    }

    static HOOK: OnceLock<Mutex<Vec<HookState>>> = OnceLock::new();
    static HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);
    static HOOK_STARTED: AtomicBool = AtomicBool::new(false);

    fn state() -> &'static Mutex<Vec<HookState>> {
        HOOK.get_or_init(|| Mutex::new(Vec::new()))
    }

    static CANCEL: OnceLock<Mutex<Option<ComboFn>>> = OnceLock::new();
    fn cancel_callback() -> &'static Mutex<Option<ComboFn>> {
        CANCEL.get_or_init(|| Mutex::new(None))
    }
    pub fn set_cancel(callback: Option<ComboFn>) {
        *cancel_callback().lock().unwrap_or_else(|p| p.into_inner()) = callback;
        if cancel_callback()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
        {
            ensure_hook();
        }
    }
    fn ensure_hook() -> bool {
        // Installing the LL hook can outlive a short wait on a cold or busy
        // machine: give each attempt 500ms, respawn the pump thread if it
        // died, and keep waiting up to ~2s before reporting failure.
        let deadline = Instant::now() + Duration::from_millis(2000);
        while Instant::now() < deadline {
            if !HOOK_STARTED.swap(true, Ordering::SeqCst)
                && std::thread::Builder::new()
                    .name("mod-hotkey-hook".into())
                    .spawn(pump_thread)
                    .is_err()
            {
                HOOK_STARTED.store(false, Ordering::SeqCst);
                return false;
            }
            let wait_until = (Instant::now() + Duration::from_millis(500)).min(deadline);
            while Instant::now() < wait_until
                && HOOK_STARTED.load(Ordering::SeqCst)
                && !HOOK_INSTALLED.load(Ordering::SeqCst)
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            if HOOK_INSTALLED.load(Ordering::SeqCst) {
                return true;
            }
            // HOOK_STARTED still true means installation is merely pending —
            // keep waiting instead of reporting a failure that may resolve.
            // A dead pump thread clears HOOK_STARTED and the loop respawns it.
        }
        HOOK_INSTALLED.load(Ordering::SeqCst)
    }

    fn vk_family(vk: i32) -> Option<u8> {
        let v = vk as u16;
        if v == VK_LSHIFT.0 || v == VK_RSHIFT.0 {
            Some(MOD_SHIFT)
        } else if v == VK_LCONTROL.0 || v == VK_RCONTROL.0 {
            Some(MOD_CTRL)
        } else if v == VK_LMENU.0 || v == VK_RMENU.0 {
            Some(MOD_ALT)
        } else if v == VK_LWIN.0 || v == VK_RWIN.0 {
            Some(MOD_WIN)
        } else {
            None
        }
    }

    fn is_required_modifier(st: &HookState, vk: i32) -> bool {
        vk_family(vk).is_some_and(|f| st.required & f != 0)
    }

    /// WH_KEYBOARD_LL runs before GetAsyncKeyState reflects the current event.
    /// The physical event set includes that event and excludes injected keys.
    /// Swallowing an event never skips updating this set.
    fn held_modifiers(down: &HashSet<i32>) -> u8 {
        down.iter()
            .filter_map(|&vk| vk_family(vk))
            .fold(0, |mask, modifier| mask | modifier)
    }

    /// Cancels the Start menu / layout-switch that would fire when Win is held.
    fn inject_dummy_key() {
        unsafe {
            let down = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0xE8),
                        wScan: 0,
                        dwFlags: KEYBD_EVENT_FLAGS(0),
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            let up = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0xE8),
                        wScan: 0,
                        dwFlags: KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            let _ = SendInput(&[down, up], std::mem::size_of::<INPUT>() as i32);
        }
    }

    fn execute_action(
        action: StateMachineAction,
        event_at: Instant,
        on_combo: &ComboFn,
        on_release: &ComboFn,
    ) {
        // Gesture recognition belongs to the shared dispatcher. Deliver raw
        // transitions synchronously so release cannot overtake press.
        match action {
            StateMachineAction::StartRecording => {
                inject_dummy_key();
                on_combo(event_at);
            }
            StateMachineAction::StopRecording => on_release(event_at),
            _ => {}
        }
    }

    unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            // Timestamp first: everything after this point (lock acquisition,
            // state machine, thread spawn, SendInput, IPC) is latency the
            // user pays for and must therefore be inside the measurement.
            let event_at = Instant::now();
            let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            let msg = wparam.0 as u32;
            let vk = kb.vkCode as i32;
            let injected = (kb.flags.0 & LLKHF_INJECTED) != 0;

            let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
            if injected && !allow_injected() {
                return CallNextHookEx(None, code, wparam, lparam);
            }
            if vk == i32::from(VK_ESCAPE.0) && is_down {
                let callback = {
                    cancel_callback()
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone()
                };
                if let Some(cb) = callback {
                    std::thread::spawn(move || cb(event_at));
                }
                // Escape is deliberately transparent.
                return CallNextHookEx(None, code, wparam, lparam);
            }
            let mut guard = state().lock().unwrap_or_else(|p| p.into_inner());
            let mut actions = Vec::new();
            let mut swallow = false;
            for st in guard.iter_mut() {
                // A missed key-up (focus loss, hook timeout, debugger pause)
                // would leave a modifier in `down` forever and the combo could
                // never match again. Reconcile modifier entries against the
                // kernel's live key state — except modifiers whose key-down
                // this hook swallowed, which GetAsyncKeyState never saw go
                // down. The current event is applied right after, so its own
                // vk is authoritative either way.
                reconcile_held(
                    &mut st.down,
                    &st.swallowed,
                    |vk| vk_family(vk).is_some(),
                    |vk| unsafe { GetAsyncKeyState(vk) } < 0,
                );
                if is_down {
                    st.down.insert(vk);
                } else if is_up {
                    st.down.remove(&vk);
                }
                let combo_matched = held_modifiers(&st.down) == st.required;
                let action = if !st.active && combo_matched {
                    st.active = true;
                    st.swallowed.insert(vk);
                    swallow = true;
                    StateMachineAction::StartRecording
                } else if st.active && !combo_matched {
                    st.active = false;
                    swallow |= st.swallowed.remove(&vk);
                    st.swallowed.clear();
                    StateMachineAction::StopRecording
                } else {
                    if st.active && is_required_modifier(st, vk) && is_down {
                        st.swallowed.insert(vk);
                        swallow = true;
                    }
                    StateMachineAction::None
                };
                actions.push((action, st.on_combo.clone(), st.on_release.clone()));
            }
            drop(guard);
            for (action, on_combo, on_release) in actions {
                execute_action(action, event_at, &on_combo, &on_release);
            }
            if swallow {
                return LRESULT(1);
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    fn pump_thread() {
        unsafe {
            use windows::Win32::System::Threading::{
                GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
            };
            // Priority helps the callback answer Windows within the strict
            // low-level-hook timeout even while the app is busy.
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);

            // Global hooks require the module handle of the module that
            // contains the hook procedure — NULL is not guaranteed to work.
            let hmodule =
                windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
            let hmod = HINSTANCE(hmodule.0);

            let install = || SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), hmod, 0);

            let current = match install() {
                Err(err) => {
                    log::error!("Failed to install keyboard hook: {err}");
                    HOOK_STARTED.store(false, Ordering::SeqCst);
                    return;
                }
                Ok(h) => {
                    log::info!("Keyboard hook installed (LL keyboard)");
                    HOOK_INSTALLED.store(true, Ordering::SeqCst);
                    h
                }
            };

            // Do not Unhook/reinstall on a timer — that drops in-flight
            // key-ups and stalls the callback. Keep the hook for the
            // process lifetime.
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
            log::warn!("Keyboard hook message loop exited");
            let _ = UnhookWindowsHookEx(current);
            HOOK_INSTALLED.store(false, Ordering::SeqCst);
            HOOK_STARTED.store(false, Ordering::SeqCst);
        }
    }

    /// Configure (or reconfigure) the combo. Starts the hook thread once.
    pub fn set_combo(required_modifiers: u8, on_combo: ComboFn, on_release: ComboFn) -> bool {
        if required_modifiers.count_ones() < 2 {
            return false;
        }
        {
            let mut guard = state().lock().unwrap_or_else(|p| p.into_inner());
            guard.push(HookState {
                required: required_modifiers,
                on_combo,
                on_release,
                down: HashSet::new(),
                swallowed: HashSet::new(),
                active: false,
            });
        }
        ensure_hook()
    }

    pub fn reset_mode() {
        super::set_hands_free(false);
        // Keep physical key state until key-up. Clearing `active` while the
        // stop key is held would interpret its auto-repeat as a fresh press.
    }

    pub fn clear_combo() {
        let mut guard = state().lock().unwrap_or_else(|p| p.into_inner());
        guard.clear();
    }

    pub const fn flags(shift: bool, ctrl: bool, alt: bool, win: bool) -> u8 {
        (shift as u8) | ((ctrl as u8) << 1) | ((alt as u8) << 2) | ((win as u8) << 3)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn modifier_mask_uses_current_press_and_release() {
            let mut down = HashSet::new();
            down.insert(i32::from(VK_LSHIFT.0));
            assert_eq!(held_modifiers(&down), MOD_SHIFT);
            down.insert(i32::from(VK_LWIN.0));
            assert_eq!(held_modifiers(&down), MOD_SHIFT | MOD_WIN);
            down.remove(&i32::from(VK_LWIN.0));
            assert_eq!(held_modifiers(&down), MOD_SHIFT);
        }

        #[test]
        fn extra_modifier_breaks_combo_and_both_sides_share_family() {
            let mut down = HashSet::from([
                i32::from(VK_LSHIFT.0),
                i32::from(VK_RSHIFT.0),
                i32::from(VK_LWIN.0),
                i32::from(VK_LCONTROL.0),
            ]);
            assert_ne!(held_modifiers(&down), MOD_SHIFT | MOD_WIN);
            down.remove(&i32::from(VK_LCONTROL.0));
            down.remove(&i32::from(VK_LSHIFT.0));
            assert_eq!(held_modifiers(&down), MOD_SHIFT | MOD_WIN);
        }
    }
}

#[cfg(windows)]
pub use platform::{clear_combo, flags, reset_mode, set_cancel, set_combo};

#[cfg(not(windows))]
pub mod platform {
    pub type ComboFn = std::sync::Arc<dyn Fn(std::time::Instant) + Send + Sync + 'static>;
    pub fn set_combo(_required: u8, _a: ComboFn, _b: ComboFn) -> bool {
        false
    }
    pub fn set_cancel(_callback: Option<ComboFn>) {}
    pub fn clear_combo() {}
    pub fn reset_mode() {
        super::set_hands_free(false);
    }
    pub const fn flags(_shift: bool, _ctrl: bool, _alt: bool, _win: bool) -> u8 {
        0
    }
}

#[cfg(not(windows))]
pub use platform::{clear_combo, flags, reset_mode, set_cancel, set_combo};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconcile_keeps_swallowed_modifiers_despite_reported_key_up() {
        use std::collections::HashSet;
        const SHIFT: i32 = 0xA0;
        const WIN: i32 = 0x5B;
        const KEY_A: i32 = 0x41;
        let is_modifier = |vk: i32| vk == SHIFT || vk == WIN;
        // Win's key-down was swallowed by the hook, so GetAsyncKeyState
        // reports it up even while held; Shift reports down normally.
        let physically_down = |vk: i32| vk == SHIFT;
        let mut down = HashSet::from([SHIFT, WIN, KEY_A]);
        let swallowed = HashSet::from([WIN]);
        reconcile_held(&mut down, &swallowed, is_modifier, physically_down);
        assert_eq!(down, HashSet::from([SHIFT, WIN, KEY_A]));
    }

    #[test]
    fn reconcile_drops_modifier_with_missed_key_up() {
        use std::collections::HashSet;
        const SHIFT: i32 = 0xA0;
        const WIN: i32 = 0x5B;
        let is_modifier = |vk: i32| vk == SHIFT || vk == WIN;
        // Both modifiers report up and neither was swallowed: a missed
        // key-up must not wedge the combo — the stale entries are dropped.
        let mut down = HashSet::from([SHIFT, WIN]);
        reconcile_held(&mut down, &HashSet::new(), is_modifier, |_| false);
        assert!(down.is_empty());
    }
}
