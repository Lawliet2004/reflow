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

#[path = "../src/hotkey/gesture.rs"]
mod gesture;
use gesture::{Action, Gesture};
use reflow_lib::rewrite::FlowClient;
use reflow_lib::settings::AppSettings;
use std::time::{Duration, Instant};

/// Double-tapping must leave the machine in the hands-free state, and that state
/// is what exempts the session from the stuck-key duration guard.
#[test]
fn double_tap_engages_hands_free() {
    let mut gesture = Gesture::default();
    let t0 = Instant::now();
    assert_eq!(gesture.input(true, t0), vec![Action::Start(t0)]);
    assert!(gesture
        .input(false, t0 + Duration::from_millis(100))
        .is_empty());
    assert!(gesture.deadline().is_some());
    assert_eq!(
        gesture.input(true, t0 + Duration::from_millis(220)),
        vec![Action::Lock]
    );
    assert!(gesture
        .input(false, t0 + Duration::from_millis(300))
        .is_empty());
    assert_eq!(gesture.deadline(), None);
    let stop = t0 + Duration::from_secs(5);
    assert_eq!(gesture.input(true, stop), vec![Action::Stop(stop)]);
}

#[test]
fn a_plain_hold_is_not_hands_free() {
    let mut gesture = Gesture::default();
    let t0 = Instant::now();
    assert_eq!(gesture.input(true, t0), vec![Action::Start(t0)]);
    let released = t0 + Duration::from_millis(1200);
    assert_eq!(gesture.input(false, released), vec![Action::Stop(released)]);
    assert_eq!(gesture.deadline(), None);
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
