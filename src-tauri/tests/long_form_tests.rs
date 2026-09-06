//! Long-form dictation: the limits that used to truncate it.
//!
//! Hands-free mode is reachable by double-tapping the hotkey and was intended for
//! dictation measured in minutes. Four separate caps stood in the way, each one
//! silent:
//!
//! | cap | was | effect on a 10 minute dictation |
//! |-----|-----|---------------------------------|
//! | `max_duration_sec` | 60 s | recording stopped itself after a minute |
//! | `FINAL_MAX_AUDIO_S` | 120 s | only the first fifth was transcribed |
//! | `--ctx-size` | hard-coded 1024 | the configured window was ignored |
//! | `max_tokens` | 256 | polished output cut off at ~190 words |
//!
//! These tests pin the parts of that which are testable without a GPU.

use reflow_lib::hotkey::hook::{hands_free_engaged, HookMode, HotkeyStateMachine};
use reflow_lib::rewrite::FlowClient;
use reflow_lib::settings::AppSettings;
use std::time::{Duration, Instant};

/// Double-tapping must leave the machine in the hands-free state, and that state
/// is what exempts the session from the stuck-key duration guard.
#[test]
fn double_tap_engages_hands_free() {
    let mut sm = HotkeyStateMachine::new(350, 350);
    let t0 = Instant::now();

    // First tap: press and quick release -> waiting for a second tap.
    sm.on_combo_down(t0);
    sm.on_combo_up(t0 + Duration::from_millis(100));
    assert!(
        matches!(sm.mode, HookMode::PendingDoubleTap { .. }),
        "a quick tap should wait for a second one, got {:?}",
        sm.mode
    );

    // Second tap inside the window -> locked on.
    sm.on_combo_down(t0 + Duration::from_millis(220));
    assert_eq!(sm.mode, HookMode::AutoLocked);

    // Releasing does not stop a hands-free session.
    sm.on_combo_up(t0 + Duration::from_millis(300));
    assert_eq!(sm.mode, HookMode::AutoLocked);

    // Tapping again ends it.
    sm.on_combo_down(t0 + Duration::from_millis(5_000));
    assert_eq!(sm.mode, HookMode::Idle);
}

/// A held key still gets a duration guard, because a missed key-up would
/// otherwise record forever.
#[test]
fn a_plain_hold_is_not_hands_free() {
    let mut sm = HotkeyStateMachine::new(350, 350);
    let t0 = Instant::now();
    sm.on_combo_down(t0);
    assert!(matches!(sm.mode, HookMode::Holding { .. }));
    // Held past the double-tap window, then released: a normal hold-to-talk.
    sm.on_combo_up(t0 + Duration::from_millis(1_200));
    assert_eq!(sm.mode, HookMode::Idle);
    assert!(
        !hands_free_engaged(),
        "a hold must not be treated as a hands-free session"
    );
}

/// The completion budget has to scale with the context window, or a long
/// dictation is truncated by the very setting meant to allow it.
#[test]
fn the_completion_budget_follows_the_context_window() {
    let small = FlowClient::completion_budget(1024);
    let large = FlowClient::completion_budget(8192);
    assert!(
        large > small,
        "a bigger window must permit a longer completion: {small} vs {large}"
    );

    // The old ceiling. A 10 minute dictation is far more than 256 tokens of
    // output, and this is the number that used to cut it off.
    assert!(
        large > 256,
        "an 8k window must allow more than the old 256-token cap, got {large}"
    );

    // Never zero or negative, however small the window.
    assert!(FlowClient::completion_budget(0) >= 128);
    assert!(FlowClient::completion_budget(256) >= 128);
}

/// A client built for a window reports that window's budget.
#[test]
fn a_client_carries_the_budget_of_its_server() {
    let client = FlowClient::new_url("http://127.0.0.1:1".into(), Duration::from_secs(1))
        .with_context_size(4096);
    assert_eq!(
        client.max_completion_tokens,
        FlowClient::completion_budget(4096)
    );
}

/// The shipped default must not re-introduce a one-minute ceiling on a feature
/// intended for long dictation.
#[test]
fn the_default_duration_guard_is_not_a_feature_limit() {
    let settings = AppSettings::default();
    assert!(
        settings.max_duration_sec == 0 || settings.max_duration_sec >= 300,
        "a {}s guard is short enough to cut off ordinary dictation",
        settings.max_duration_sec
    );
    // And the configured context window must be a real, non-zero value that the
    // launch path can pass through.
    assert!(settings.refinement.context_size >= 1024);
}
