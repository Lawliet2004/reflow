//! Low-level keyboard hook for modifier-only push-to-talk and double-tap auto-mode combos (Shift+Win).
//!
//! Windows' RegisterHotKey API cannot register a hotkey made purely of
//! modifier keys, so combos like Shift+Win are tracked with a
//! WH_KEYBOARD_LL hook instead.
//!
//! Features:
//! 1. Push-to-Talk (Hold): Holding Shift+Win for >350ms starts recording immediately; releasing stops recording & transcribes.
//! 2. Auto Mode (Double-Tap): Double-tapping Shift+Win within 350ms locks recording indefinitely; pressing Shift+Win once stops recording.
//! 3. Single-Tap Timeout: A quick single tap (<350ms) waits for a possible 2nd tap; if none arrives within 350ms, it stops recording & transcribes.
//!
//! The hook is transparent — it never swallows keys — except the Win
//! key-up right after a dictation, which is suppressed so the Start
//! menu doesn't open when the combo is released.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookMode {
    /// No recording active.
    Idle,
    /// Shift+Win is physically held down for a potential Push-to-Talk or first tap of a double tap.
    Holding { pressed_at: Instant },
    /// First tap was released within double_tap_max_hold.
    /// Recording continues running; waiting for a 2nd press within double_tap_window.
    PendingDoubleTap { released_at: Instant, generation: u64 },
    /// Double-tap confirmed! Recording is locked in Auto Mode (hands-free).
    AutoLocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateMachineAction {
    None,
    StartRecording,
    StopRecording,
    ScheduleDoubleTapTimer { generation: u64, delay_ms: u64 },
    CancelTimer,
}

#[derive(Debug, Clone)]
pub struct HotkeyStateMachine {
    pub mode: HookMode,
    pub generation: u64,
    pub double_tap_max_hold: Duration,
    pub double_tap_window: Duration,
}

impl Default for HotkeyStateMachine {
    fn default() -> Self {
        Self {
            mode: HookMode::Idle,
            generation: 0,
            double_tap_max_hold: Duration::from_millis(350),
            double_tap_window: Duration::from_millis(350),
        }
    }
}

impl HotkeyStateMachine {
    pub fn new(max_hold_ms: u64, window_ms: u64) -> Self {
        Self {
            mode: HookMode::Idle,
            generation: 0,
            double_tap_max_hold: Duration::from_millis(max_hold_ms),
            double_tap_window: Duration::from_millis(window_ms),
        }
    }

    pub fn on_combo_down(&mut self, now: Instant) -> StateMachineAction {
        match self.mode {
            HookMode::Idle => {
                self.mode = HookMode::Holding { pressed_at: now };
                StateMachineAction::StartRecording
            }
            HookMode::PendingDoubleTap { .. } => {
                // Second tap within window -> promote to auto locked hands-free mode!
                self.generation = self.generation.wrapping_add(1);
                self.mode = HookMode::AutoLocked;
                StateMachineAction::CancelTimer
            }
            HookMode::AutoLocked => {
                // Press combo while in auto mode -> stop recording immediately
                self.mode = HookMode::Idle;
                StateMachineAction::StopRecording
            }
            HookMode::Holding { .. } => {
                // Key repeats or already held
                StateMachineAction::None
            }
        }
    }

    pub fn on_combo_up(&mut self, now: Instant) -> StateMachineAction {
        match self.mode {
            HookMode::Holding { pressed_at } => {
                let duration = now.saturating_duration_since(pressed_at);
                if duration > self.double_tap_max_hold {
                    // Normal hold-to-talk release!
                    self.mode = HookMode::Idle;
                    StateMachineAction::StopRecording
                } else {
                    // Quick tap release -> wait for possible second tap
                    self.generation = self.generation.wrapping_add(1);
                    let gen = self.generation;
                    self.mode = HookMode::PendingDoubleTap {
                        released_at: now,
                        generation: gen,
                    };
                    StateMachineAction::ScheduleDoubleTapTimer {
                        generation: gen,
                        delay_ms: self.double_tap_window.as_millis() as u64,
                    }
                }
            }
            HookMode::AutoLocked => {
                // Release during auto mode is ignored (keeps recording hands-free)
                StateMachineAction::None
            }
            HookMode::PendingDoubleTap { .. } | HookMode::Idle => StateMachineAction::None,
        }
    }

    pub fn on_timer_expired(&mut self, generation: u64) -> StateMachineAction {
        if let HookMode::PendingDoubleTap {
            generation: cur_gen,
            ..
        } = self.mode
        {
            if cur_gen == generation {
                self.mode = HookMode::Idle;
                return StateMachineAction::StopRecording;
            }
        }
        StateMachineAction::None
    }

    pub fn reset(&mut self) {
        self.mode = HookMode::Idle;
        self.generation = self.generation.wrapping_add(1);
    }
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
        CallNextHookEx, GetMessageW, SetWindowsHookExW, UnhookWindowsHookEx, KBDLLHOOKSTRUCT,
        MSG, WH_KEYBOARD_LL,
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
            std::env::var("REFLOW_TEST_INJECTED").map(|v| v == "1").unwrap_or(false)
        })
    }

    const MOD_SHIFT: u8 = 1;
    const MOD_CTRL: u8 = 2;
    const MOD_ALT: u8 = 4;
    const MOD_WIN: u8 = 8;

    type ComboFn = Arc<dyn Fn() + Send + Sync + 'static>;

    struct HookState {
        required: u8,
        on_combo: ComboFn,
        on_release: ComboFn,
        down: HashSet<i32>,
        swallowed: HashSet<i32>,
        active: bool,
        sm: HotkeyStateMachine,
    }

    static HOOK: OnceLock<Mutex<Option<HookState>>> = OnceLock::new();
    static HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

    fn state() -> &'static Mutex<Option<HookState>> {
        HOOK.get_or_init(|| Mutex::new(None))
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

    fn key_down(vk: VIRTUAL_KEY) -> bool {
        unsafe { GetAsyncKeyState(i32::from(vk.0)) < 0 }
    }

    /// Physical modifier mask from the OS, not our HashSet (which desyncs
    /// when we swallow a key-up). Extra keys like Ctrl must not count.
    fn live_held() -> u8 {
        let mut flags = 0u8;
        if key_down(VK_LSHIFT) || key_down(VK_RSHIFT) {
            flags |= MOD_SHIFT;
        }
        if key_down(VK_LCONTROL) || key_down(VK_RCONTROL) {
            flags |= MOD_CTRL;
        }
        if key_down(VK_LMENU) || key_down(VK_RMENU) {
            flags |= MOD_ALT;
        }
        if key_down(VK_LWIN) || key_down(VK_RWIN) {
            flags |= MOD_WIN;
        }
        flags
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

    fn execute_action(action: StateMachineAction, on_combo: &ComboFn, on_release: &ComboFn) {
        match action {
            StateMachineAction::StartRecording => {
                let cb = on_combo.clone();
                std::thread::spawn(move || {
                    log::info!("Hotkey combo engaged -> Start recording");
                    inject_dummy_key();
                    cb();
                });
            }
            StateMachineAction::StopRecording => {
                let cb = on_release.clone();
                std::thread::spawn(move || {
                    log::info!("Hotkey combo released/stopped -> Stop recording");
                    cb();
                });
            }
            StateMachineAction::ScheduleDoubleTapTimer { generation, delay_ms } => {
                let cb = on_release.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                    let mut guard = state().lock().unwrap_or_else(|p| p.into_inner());
                    if let Some(st) = guard.as_mut() {
                        let act = st.sm.on_timer_expired(generation);
                        if act == StateMachineAction::StopRecording {
                            drop(guard);
                            log::info!("Single-tap double-tap window expired -> Stop recording");
                            cb();
                        }
                    }
                });
            }
            StateMachineAction::CancelTimer => {
                log::info!("Hotkey double-tap confirmed -> Auto Mode engaged (hands-free)");
            }
            StateMachineAction::None => {}
        }
    }

    unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            let msg = wparam.0 as u32;
            let vk = kb.vkCode as i32;
            let injected = (kb.flags.0 & LLKHF_INJECTED) != 0;

            let mut guard = state().lock().unwrap_or_else(|p| p.into_inner());
            if let Some(st) = guard.as_mut() {
                let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
                let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;

                // Injected dummy keys (used to cancel the Start menu) must
                // reach Windows, but they must not drive the combo state.
                if injected && !allow_injected() {
                    drop(guard);
                    return CallNextHookEx(None, code, wparam, lparam);
                }

                if is_down {
                    st.down.insert(vk);
                } else if is_up {
                    st.down.remove(&vk);
                }

                if st.required.count_ones() >= 2 {
                    // Exact match only: Ctrl+Win must not fire a Shift+Win
                    // combo, even if Shift was left stuck in our HashSet.
                    let held = live_held();
                    let combo_matched = held == st.required;

                    if !st.active && combo_matched {
                        st.active = true;
                        st.swallowed.insert(vk);
                        let action = st.sm.on_combo_down(Instant::now());
                        let on_combo = st.on_combo.clone();
                        let on_release = st.on_release.clone();
                        drop(guard);
                        execute_action(action, &on_combo, &on_release);
                        return LRESULT(1);
                    }
                    if st.active && !combo_matched {
                        st.active = false;
                        let swallow_this_up = st.swallowed.remove(&vk);
                        st.swallowed.clear();
                        let action = st.sm.on_combo_up(Instant::now());
                        let on_combo = st.on_combo.clone();
                        let on_release = st.on_release.clone();
                        drop(guard);
                        execute_action(action, &on_combo, &on_release);
                        if swallow_this_up {
                            return LRESULT(1);
                        }
                        return CallNextHookEx(None, code, wparam, lparam);
                    }
                    if st.active && is_required_modifier(st, vk) {
                        if is_down {
                            st.swallowed.insert(vk);
                            return LRESULT(1);
                        }
                        // Let key-up events pass through so GetAsyncKeyState
                        // reflects the true physical state for live_held().
                    }
                }
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
            let hmodule = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                .unwrap_or_default();
            let hmod = HINSTANCE(hmodule.0);

            let install =
                || SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), hmod, 0);

            let current = match install() {
                Err(err) => {
                    log::error!("Failed to install keyboard hook: {err}");
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
        }
    }

    /// Configure (or reconfigure) the combo. Starts the hook thread once.
    pub fn set_combo(required_modifiers: u8, on_combo: ComboFn, on_release: ComboFn) -> bool {
        if required_modifiers.count_ones() < 2 {
            return false;
        }
        {
            let mut guard = state().lock().unwrap_or_else(|p| p.into_inner());
            *guard = Some(HookState {
                required: required_modifiers,
                on_combo,
                on_release,
                down: HashSet::new(),
                swallowed: HashSet::new(),
                active: false,
                sm: HotkeyStateMachine::default(),
            });
        }
        if !HOOK_INSTALLED.load(Ordering::SeqCst) {
            std::thread::Builder::new()
                .name("mod-hotkey-hook".into())
                .spawn(pump_thread)
                .is_ok()
            } else {
            true
        }
    }

    pub fn reset_mode() {
        let mut guard = state().lock().unwrap_or_else(|p| p.into_inner());
        if let Some(st) = guard.as_mut() {
            st.sm.reset();
            st.active = false;
            st.swallowed.clear();
        }
    }

    pub fn clear_combo() {
        let mut guard = state().lock().unwrap_or_else(|p| p.into_inner());
        *guard = None;
    }

    pub const fn flags(shift: bool, ctrl: bool, alt: bool, win: bool) -> u8 {
        (shift as u8)
            | ((ctrl as u8) << 1)
            | ((alt as u8) << 2)
            | ((win as u8) << 3)
    }
}

#[cfg(windows)]
pub use platform::{clear_combo, flags, reset_mode, set_combo};

#[cfg(not(windows))]
pub mod platform {
    pub type ComboFn = std::sync::Arc<dyn Fn() + Send + Sync + 'static>;
    pub fn set_combo(_required: u8, _a: ComboFn, _b: ComboFn) -> bool {
        false
    }
    pub fn clear_combo() {}
    pub fn reset_mode() {}
    pub const fn flags(_shift: bool, _ctrl: bool, _alt: bool, _win: bool) -> u8 {
        0
    }
}

#[cfg(not(windows))]
pub use platform::{clear_combo, flags, reset_mode, set_combo};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hold_to_talk_workflow() {
        let mut sm = HotkeyStateMachine::new(350, 350);
        let t0 = Instant::now();

        // 1. Press combo -> Start recording
        let act = sm.on_combo_down(t0);
        assert_eq!(act, StateMachineAction::StartRecording);
        assert!(matches!(sm.mode, HookMode::Holding { .. }));

        // 2. Held for 1.2s (> 350ms) -> Release stops recording
        let act2 = sm.on_combo_up(t0 + Duration::from_millis(1200));
        assert_eq!(act2, StateMachineAction::StopRecording);
        assert_eq!(sm.mode, HookMode::Idle);
    }

    #[test]
    fn test_double_tap_auto_mode_workflow() {
        let mut sm = HotkeyStateMachine::new(350, 350);
        let t0 = Instant::now();

        // Tap 1 Down -> Start recording
        let a1 = sm.on_combo_down(t0);
        assert_eq!(a1, StateMachineAction::StartRecording);

        // Tap 1 Up at 100ms (< 350ms) -> Schedule timer, mode is PendingDoubleTap
        let a2 = sm.on_combo_up(t0 + Duration::from_millis(100));
        assert!(matches!(a2, StateMachineAction::ScheduleDoubleTapTimer { generation: 1, delay_ms: 350 }));
        assert!(matches!(sm.mode, HookMode::PendingDoubleTap { generation: 1, .. }));

        // Tap 2 Down at 220ms -> Promotes to AutoLocked, cancels timer
        let a3 = sm.on_combo_down(t0 + Duration::from_millis(220));
        assert_eq!(a3, StateMachineAction::CancelTimer);
        assert_eq!(sm.mode, HookMode::AutoLocked);

        // Tap 2 Up at 300ms -> Ignored, remains AutoLocked
        let a4 = sm.on_combo_up(t0 + Duration::from_millis(300));
        assert_eq!(a4, StateMachineAction::None);
        assert_eq!(sm.mode, HookMode::AutoLocked);

        // Old timer expires at 450ms -> Ignored because generation changed
        let a5 = sm.on_timer_expired(1);
        assert_eq!(a5, StateMachineAction::None);
        assert_eq!(sm.mode, HookMode::AutoLocked);

        // User speaks hands-free for 10 seconds, then presses combo again to stop
        let a6 = sm.on_combo_down(t0 + Duration::from_secs(10));
        assert_eq!(a6, StateMachineAction::StopRecording);
        assert_eq!(sm.mode, HookMode::Idle);

        // Release after stop -> Ignored
        let a7 = sm.on_combo_up(t0 + Duration::from_secs(10) + Duration::from_millis(100));
        assert_eq!(a7, StateMachineAction::None);
        assert_eq!(sm.mode, HookMode::Idle);
    }

    #[test]
    fn test_single_tap_timeout_workflow() {
        let mut sm = HotkeyStateMachine::new(350, 350);
        let t0 = Instant::now();

        // Single quick tap
        let a1 = sm.on_combo_down(t0);
        assert_eq!(a1, StateMachineAction::StartRecording);

        let a2 = sm.on_combo_up(t0 + Duration::from_millis(50));
        assert!(matches!(a2, StateMachineAction::ScheduleDoubleTapTimer { generation: 1, .. }));

        // No second tap -> timer fires
        let a3 = sm.on_timer_expired(1);
        assert_eq!(a3, StateMachineAction::StopRecording);
        assert_eq!(sm.mode, HookMode::Idle);
    }

    #[test]
    fn test_reset_mode_clears_state() {
        let mut sm = HotkeyStateMachine::new(350, 350);
        sm.mode = HookMode::AutoLocked;
        sm.reset();
        assert_eq!(sm.mode, HookMode::Idle);
    }
}

