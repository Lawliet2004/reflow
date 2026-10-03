//! Multiple shortcut actions must never acquire or stop somebody else's session.
//! These tests exercise admission and release ownership without a keyboard hook,
//! microphone device, window manager, or live speech model.

use reflow_lib::context::AppContext;
use reflow_lib::dory::{CaptureKind, SessionIntent};
use reflow_lib::session;
use reflow_lib::settings::Mode;
use reflow_lib::state::AppStateEnum;

struct Fixture {
    ctx: Option<AppContext>,
    dir: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir =
            std::env::temp_dir().join(format!("reflow_hotkey_ownership_{}", uuid::Uuid::new_v4()));
        Self {
            ctx: Some(AppContext::bootstrap_test(dir.clone())),
            dir,
        }
    }

    fn ctx(&self) -> &AppContext {
        self.ctx.as_ref().unwrap()
    }

    fn recording(&self, intent: SessionIntent, kind: CaptureKind) {
        let ctx = self.ctx();
        *ctx.state_enum.write() = AppStateEnum::Recording;
        *ctx.current_session_id.write() = Some(41);
        *ctx.session_intent.write() = intent;
        *ctx.capture_kind.write() = kind;
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Close the SQLite connection before removing the Windows fixture dir.
        self.ctx.take();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn command_start_cannot_claim_an_external_recording() {
    let fixture = Fixture::new();
    fixture.recording(SessionIntent::Dictate, CaptureKind::External);
    let ctx = fixture.ctx();

    let result =
        session::start_microphone_with_intent_at(ctx, None, SessionIntent::Command, None).await;

    assert_eq!(
        result
            .expect_err("recording admission must be rejected")
            .code,
        "session_busy"
    );
    assert_eq!(*ctx.state_enum.read(), AppStateEnum::Recording);
    assert_eq!(*ctx.current_session_id.read(), Some(41));
    assert_eq!(*ctx.capture_kind.read(), CaptureKind::External);
    assert_eq!(*ctx.session_intent.read(), SessionIntent::Dictate);
}

#[tokio::test]
async fn rejected_start_preserves_the_session_being_processed() {
    let fixture = Fixture::new();
    fixture.recording(SessionIntent::Note, CaptureKind::Microphone);
    let ctx = fixture.ctx();
    *ctx.state_enum.write() = AppStateEnum::Processing;

    let error = session::start_microphone_with_intent_at(ctx, None, SessionIntent::Command, None)
        .await
        .expect_err("processing admission must be rejected");

    assert_eq!(error.code, "session_busy");
    assert_eq!(*ctx.state_enum.read(), AppStateEnum::Processing);
    assert_eq!(*ctx.current_session_id.read(), Some(41));
    assert_eq!(*ctx.session_intent.read(), SessionIntent::Note);
}

#[test]
fn releasing_command_shortcut_cannot_stop_dictation() {
    let fixture = Fixture::new();
    fixture.recording(SessionIntent::Dictate, CaptureKind::Microphone);

    assert!(!session::hotkey_owns_session(fixture.ctx(), "command"));
    assert!(session::hotkey_owns_session(fixture.ctx(), "dictate"));
}

#[test]
fn releasing_dictation_shortcut_cannot_stop_command_assistant_or_note() {
    let fixture = Fixture::new();
    for (intent, action) in [
        (SessionIntent::Command, "command"),
        (SessionIntent::Assistant, "assistant"),
        (SessionIntent::Note, "note"),
    ] {
        fixture.recording(intent, CaptureKind::Microphone);
        assert!(!session::hotkey_owns_session(fixture.ctx(), "dictate"));
        assert!(session::hotkey_owns_session(fixture.ctx(), action));
    }
}

#[test]
fn external_capture_never_belongs_to_a_keyboard_shortcut() {
    let fixture = Fixture::new();
    fixture.recording(SessionIntent::Dictate, CaptureKind::External);

    assert!(!session::hotkey_owns_session(fixture.ctx(), "dictate"));
}

#[test]
fn explicit_mode_release_only_belongs_to_that_mode() {
    let fixture = Fixture::new();
    fixture.recording(SessionIntent::Dictate, CaptureKind::Microphone);
    let ctx = fixture.ctx();
    *ctx.session_mode.write() = Some(Mode {
        id: "email".into(),
        ..Mode::default()
    });
    ctx.session_context.write().explicit_mode = true;

    assert!(session::hotkey_owns_session(ctx, "mode:email"));
    assert!(!session::hotkey_owns_session(ctx, "mode:coding"));
    assert!(!session::hotkey_owns_session(ctx, "dictate"));
}

#[test]
fn automatically_selected_mode_still_belongs_to_the_dictation_shortcut() {
    let fixture = Fixture::new();
    fixture.recording(SessionIntent::Dictate, CaptureKind::Microphone);
    let ctx = fixture.ctx();
    *ctx.session_mode.write() = Some(Mode {
        id: "email".into(),
        ..Mode::default()
    });
    ctx.session_context.write().explicit_mode = false;

    assert!(session::hotkey_owns_session(ctx, "dictate"));
    assert!(!session::hotkey_owns_session(ctx, "mode:email"));
}

#[test]
fn completed_recording_cannot_be_stopped_again_by_a_late_release() {
    let fixture = Fixture::new();
    fixture.recording(SessionIntent::Dictate, CaptureKind::Microphone);
    *fixture.ctx().state_enum.write() = AppStateEnum::Processing;

    assert!(!session::hotkey_owns_session(fixture.ctx(), "dictate"));
}

#[tokio::test]
async fn queued_release_of_old_session_cannot_stop_a_new_recording() {
    let fixture = Fixture::new();
    fixture.recording(SessionIntent::Dictate, CaptureKind::Microphone);
    let ctx = fixture.ctx();
    *ctx.current_session_id.write() = Some(42);

    // The API can report an ended session; it must leave the current one alone.
    let _ = session::stop_hotkey_owned_at(ctx, 41, std::time::Instant::now()).await;

    assert_eq!(*ctx.current_session_id.read(), Some(42));
    assert_eq!(*ctx.state_enum.read(), AppStateEnum::Recording);
}

#[tokio::test]
async fn escape_cancellation_waits_for_in_progress_startup_instead_of_being_lost() {
    let fixture = Fixture::new();
    fixture.recording(SessionIntent::Dictate, CaptureKind::Microphone);
    let ctx = fixture.ctx();
    let startup = ctx.session_operation.lock().await;
    let cancel_ctx = ctx.clone();
    let cancellation =
        tokio::spawn(async move { session::cancel_owned_wait(&cancel_ctx, Some(41)).await });

    tokio::task::yield_now().await;
    assert!(
        !cancellation.is_finished(),
        "Escape must wait for admission to finish"
    );
    assert_eq!(*ctx.state_enum.read(), AppStateEnum::Recording);
    drop(startup);
    tokio::time::timeout(std::time::Duration::from_secs(5), cancellation)
        .await
        .expect("cancellation must not hang")
        .expect("cancellation task must finish")
        .expect("queued cancellation must succeed");

    assert_eq!(*ctx.state_enum.read(), AppStateEnum::Ready);
    assert_eq!(*ctx.current_session_id.read(), None);
    assert_eq!(*ctx.capture_kind.read(), CaptureKind::None);
}

#[tokio::test]
async fn queued_escape_for_an_old_session_cannot_cancel_a_new_recording() {
    let fixture = Fixture::new();
    fixture.recording(SessionIntent::Dictate, CaptureKind::Microphone);
    let ctx = fixture.ctx();
    let startup = ctx.session_operation.lock().await;
    let cancel_ctx = ctx.clone();
    let cancellation =
        tokio::spawn(async move { session::cancel_owned_wait(&cancel_ctx, Some(41)).await });

    tokio::task::yield_now().await;
    *ctx.current_session_id.write() = Some(42);
    drop(startup);
    tokio::time::timeout(std::time::Duration::from_secs(5), cancellation)
        .await
        .expect("stale cancellation must not hang")
        .expect("cancellation task must finish")
        .expect("stale cancellation is harmless");

    assert_eq!(*ctx.current_session_id.read(), Some(42));
    assert_eq!(*ctx.state_enum.read(), AppStateEnum::Recording);
    assert_eq!(*ctx.capture_kind.read(), CaptureKind::Microphone);
}
