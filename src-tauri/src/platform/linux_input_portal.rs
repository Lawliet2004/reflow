//! Keyboard-only Wayland input through the user-approved desktop portal.
//! One asynchronous worker owns a session until recovery, suspend, or exit. Pending
//! approval never queues keystrokes against a window the user may later change.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use super::desktop::{self, KeyboardState};
use futures_util::{FutureExt, StreamExt};
use tauri::{AppHandle, Emitter};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, Proxy};

const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.RemoteDesktop";
const KEYBOARD: u32 = 1;
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);
const INPUT_TIMEOUT: Duration = Duration::from_secs(2);
const PENDING: &str = "Allow Reflow keyboard access in the desktop permission dialog to enable automatic paste. Use Copy in Reflow for manual paste while waiting.";

static APP: OnceLock<AppHandle> = OnceLock::new();
static WORKER: parking_lot::Mutex<Option<Worker>> = parking_lot::Mutex::new(None);
static TASK: parking_lot::Mutex<Option<tauri::async_runtime::JoinHandle<()>>> =
    parking_lot::Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
enum Status {
    Pending,
    Ready,
    Failed(String),
}

struct Request {
    events: Vec<(i32, u32)>,
    reply: std::sync::mpsc::SyncSender<Result<(), String>>,
    deadline: Instant,
    wall_deadline: std::time::SystemTime,
    cancelled: Arc<AtomicBool>,
    owner_cancelled: Option<Arc<AtomicBool>>,
}

#[derive(Clone)]
struct Worker {
    sender: tokio::sync::mpsc::Sender<Request>,
    status: Arc<parking_lot::Mutex<Status>>,
}

struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

impl Request {
    fn unavailable(&self) -> Option<String> {
        if self.cancelled.load(Ordering::Acquire)
            || self
                .owner_cancelled
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            Some("desktop keyboard input was cancelled".into())
        } else if Instant::now() >= self.deadline
            || std::time::SystemTime::now() >= self.wall_deadline
        {
            Some("desktop keyboard input expired. Text stays copied.".into())
        } else {
            None
        }
    }
}

pub fn configure(app: AppHandle) {
    let _ = APP.set(app);
}

fn notify(message: &str) {
    log::info!("{message}");
    if let Some(app) = APP.get() {
        let _ = app.emit("app:warning", message);
    }
}

fn token_path() -> std::path::PathBuf {
    crate::platform::PlatformSys::get_app_dir()
        .join("config")
        .join("keyboard-portal-token")
}

fn load_token() -> Option<String> {
    std::fs::read_to_string(token_path())
        .ok()
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
}

fn save_token(token: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = token_path();
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let temporary = path.with_extension("tmp");
    let _ = std::fs::remove_file(&temporary);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(token.as_bytes())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary, &path)
}

fn clear_token() {
    let _ = std::fs::remove_file(token_path());
}

fn select_devices_options<'a>(
    handle_token: &'a str,
    restore_token: Option<&'a str>,
) -> HashMap<&'static str, Value<'a>> {
    let mut options = HashMap::from([
        ("handle_token", Value::from(handle_token)),
        ("types", Value::from(KEYBOARD)),
        // Persistent until revoked; still no screen or pointer access.
        ("persist_mode", Value::from(2_u32)),
    ]);
    if let Some(token) = restore_token {
        options.insert("restore_token", Value::from(token));
    }
    options
}

fn publish_status(status: &parking_lot::Mutex<Status>, next: Status, generation: u64) {
    let (state, message) = match &next {
        Status::Pending => (KeyboardState::Pending, Some(PENDING.into())),
        Status::Ready => (KeyboardState::Ready, None),
        Status::Failed(error) => {
            let denied = error.contains("cancelled")
                || error.contains("rejected")
                || error.contains("not granted");
            (
                if denied {
                    KeyboardState::Denied
                } else {
                    KeyboardState::Unavailable
                },
                Some(error.clone()),
            )
        }
    };
    *status.lock() = next;
    if GENERATION.load(Ordering::Acquire) == generation {
        desktop::keyboard(state, message);
    }
}

fn worker() -> Worker {
    let mut slot = WORKER.lock();
    if let Some(worker) = slot.as_ref() {
        return worker.clone();
    }
    let generation = GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    let status = Arc::new(parking_lot::Mutex::new(Status::Pending));
    publish_status(&status, Status::Pending, generation);
    let task_status = Arc::clone(&status);
    let task = tauri::async_runtime::spawn(async move {
        if let Err(error) = run(receiver, &task_status, generation).await {
            let message = format!("Automatic paste is unavailable: {error}. Text stays copied. Retry keyboard access in Settings → General.");
            publish_status(&task_status, Status::Failed(message.clone()), generation);
            if GENERATION.load(Ordering::Acquire) == generation {
                notify(&message);
            }
        }
    });
    *TASK.lock() = Some(task);
    let worker = Worker { sender, status };
    *slot = Some(worker.clone());
    worker
}

/// Explicit recovery; dictation itself never re-prompts after denial.
pub fn retry_access() -> Result<(), String> {
    let existing = WORKER.lock().clone();
    if let Some(existing) = existing {
        if matches!(*existing.status.lock(), Status::Ready | Status::Pending) {
            return Ok(());
        }
        shutdown();
    }
    let _ = worker();
    Ok(())
}

/// Drop the owned connection/session and reject queued input before exit or suspend.
pub fn shutdown() {
    GENERATION.fetch_add(1, Ordering::AcqRel);
    if let Some(worker) = WORKER.lock().take() {
        *worker.status.lock() = Status::Failed("Desktop keyboard session closed".into());
    }
    if let Some(task) = TASK.lock().take() {
        task.abort();
    }
    desktop::keyboard(KeyboardState::NotRequested, None);
}

/// Resume a previously approved session without opening a fresh consent dialog.
pub fn restore_access() {
    if load_token().is_some() {
        let _ = request_access();
    }
}

/// Starts consent lazily; calling this never sends a key.
pub fn request_access() -> Result<(), String> {
    access_status(&worker())
}

fn access_status(worker: &Worker) -> Result<(), String> {
    match worker.status.lock().clone() {
        Status::Ready => Ok(()),
        Status::Pending => Err(PENDING.into()),
        Status::Failed(error) => Err(error),
    }
}

fn send(events: Vec<(i32, u32)>) -> Result<(), String> {
    send_to_worker(&worker(), events)
}

fn send_to_worker(worker: &Worker, events: Vec<(i32, u32)>) -> Result<(), String> {
    access_status(worker)?;
    let (reply, response) = std::sync::mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let _cancellation = CancelOnDrop(Arc::clone(&cancelled));
    worker
        .sender
        .try_send(Request {
            events,
            reply,
            deadline: Instant::now() + INPUT_TIMEOUT,
            wall_deadline: std::time::SystemTime::now() + INPUT_TIMEOUT,
            cancelled,
            owner_cancelled: crate::injection::cancellation::active(),
        })
        .map_err(|_| "Desktop keyboard queue is full or closed. Text stays copied.".to_owned())?;
    response
        .recv_timeout(INPUT_TIMEOUT + Duration::from_secs(1))
        .map_err(|_| "Desktop keyboard input timed out. Text stays copied.".to_owned())?
}

fn chord_events(ctrl: bool, shift: bool, keysym: i32) -> Vec<(i32, u32)> {
    let mut events = Vec::with_capacity(6);
    if ctrl {
        events.push((0xffe3, 1)); // Control_L
    }
    if shift {
        events.push((0xffe1, 1)); // Shift_L
    }
    events.extend([(keysym, 1), (keysym, 0)]);
    if shift {
        events.push((0xffe1, 0));
    }
    if ctrl {
        events.push((0xffe3, 0));
    }
    events
}

pub fn chord(ctrl: bool, shift: bool, keysym: i32) -> Result<(), String> {
    send(chord_events(ctrl, shift, keysym))
}

pub fn type_text(text: &str) -> Result<(), String> {
    let mut events = Vec::with_capacity(text.chars().count() * 2);
    for character in text.chars() {
        let keysym = match character {
            '\n' | '\r' => 0xff0d,
            '\t' => 0xff09,
            '\0' => continue,
            character if u32::from(character) <= 0xff => character as i32,
            character => (0x0100_0000 | u32::from(character)) as i32,
        };
        events.extend([(keysym, 1), (keysym, 0)]);
    }
    send(events)
}

fn request_path(connection: &Connection, token: &str) -> String {
    let sender = connection
        .unique_name()
        .expect("session bus connection has a unique name")
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    format!("{PATH}/request/{sender}/{token}")
}

async fn request<B>(
    connection: &Connection,
    portal: &Proxy<'_>,
    method: &str,
    token: &str,
    body: &B,
) -> Result<HashMap<String, OwnedValue>, String>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let path = request_path(connection, token);
    let response = Proxy::new(
        connection,
        DESTINATION,
        path.as_str(),
        "org.freedesktop.portal.Request",
    )
    .await
    .map_err(|error| error.to_string())?;
    let mut replies = response
        .receive_signal("Response")
        .await
        .map_err(|error| error.to_string())?;
    let mut pending = super::portal_request::PendingRequest::new(connection, &path);
    let _: OwnedObjectPath = portal
        .call(method, body)
        .await
        .map_err(|error| error.to_string())?;
    let message = tokio::time::timeout(APPROVAL_TIMEOUT, replies.next())
        .await
        .map_err(|_| "desktop keyboard permission timed out".to_owned())?
        .ok_or_else(|| "desktop portal disconnected".to_owned())?;
    let (status, results): (u32, HashMap<String, OwnedValue>) = message
        .body()
        .deserialize()
        .map_err(|error| error.to_string())?;
    pending.responded();
    match status {
        0 => Ok(results),
        1 => Err("desktop keyboard permission was cancelled".into()),
        _ => Err(format!("desktop keyboard request was rejected ({status})")),
    }
}

async fn run(
    receiver: tokio::sync::mpsc::Receiver<Request>,
    status: &Arc<parking_lot::Mutex<Status>>,
    generation: u64,
) -> Result<(), String> {
    let connection = zbus::connection::Builder::session()
        .map_err(|error| error.to_string())?
        .method_timeout(Duration::from_secs(15))
        .build()
        .await
        .map_err(|error| error.to_string())?;
    let app_id = APP
        .get()
        .map_or("com.reflow.app", |app| app.config().identifier.as_str());
    crate::hotkey::portal::register_application(&connection, app_id).await?;
    let portal = Proxy::new(&connection, DESTINATION, PATH, INTERFACE)
        .await
        .map_err(|error| error.to_string())?;
    let available = portal
        .get_property::<u32>("AvailableDeviceTypes")
        .await
        .map_err(|_| "this desktop has no keyboard-control portal".to_owned())?;
    if available & KEYBOARD == 0 {
        return Err("this desktop does not allow keyboard control".into());
    }
    let mut owners = portal
        .receive_owner_changed()
        .await
        .map_err(|error| error.to_string())?;
    tokio::select! {
        biased;
        _ = owners.next() => Err("desktop portal restarted or disconnected; reconnect in Settings → General".into()),
        result = run_session(receiver, &connection, &portal, status, generation) => result,
    }
}

/// Consent negotiation, then the ready loop serving queued key input.
async fn run_session(
    mut receiver: tokio::sync::mpsc::Receiver<Request>,
    connection: &Connection,
    portal: &Proxy<'_>,
    status: &Arc<parking_lot::Mutex<Status>>,
    generation: u64,
) -> Result<(), String> {
    let token = format!("reflow_keyboard_create_{}", uuid::Uuid::new_v4().simple());
    let session_token = format!("reflow_keyboard_session_{}", uuid::Uuid::new_v4().simple());
    let options = HashMap::from([
        ("handle_token", Value::from(token.as_str())),
        ("session_handle_token", Value::from(session_token.as_str())),
    ]);
    let result = request(connection, portal, "CreateSession", &token, &(options,)).await?;
    let session_path = result
        .get("session_handle")
        .and_then(|value| <&str>::try_from(value).ok())
        .ok_or_else(|| "desktop portal returned no keyboard session".to_owned())?;
    let session = OwnedObjectPath::try_from(session_path).map_err(|error| error.to_string())?;
    let session_proxy = Proxy::new(
        connection,
        DESTINATION,
        session.as_str(),
        "org.freedesktop.portal.Session",
    )
    .await
    .map_err(|error| error.to_string())?;
    // Subscribe before SelectDevices/Start so immediate revocation is queued.
    let mut closed = session_proxy
        .receive_signal("Closed")
        .await
        .map_err(|error| error.to_string())?;
    // A saved token is single-use per Start; refusal or failure discards it so it cannot loop.
    let saved_token = load_token();
    let forget_saved_token = |_: &String| {
        if saved_token.is_some() {
            clear_token();
        }
    };
    let token = format!("reflow_keyboard_select_{}", uuid::Uuid::new_v4().simple());
    let options = select_devices_options(&token, saved_token.as_deref());
    while_session_open(
        &mut closed,
        request(
            connection,
            portal,
            "SelectDevices",
            &token,
            &(&session, options),
        ),
    )
    .await
    .inspect_err(forget_saved_token)?;
    let token = format!("reflow_keyboard_start_{}", uuid::Uuid::new_v4().simple());
    // Start consumes a restore token even when it returns no replacement.
    // Never retain the consumed token on disk and try it again next launch.
    if saved_token.is_some() {
        clear_token();
    }
    let options = HashMap::from([("handle_token", Value::from(token.as_str()))]);
    let result = while_session_open(
        &mut closed,
        request(
            connection,
            portal,
            "Start",
            &token,
            &(&session, "", options),
        ),
    )
    .await
    .inspect_err(forget_saved_token)?;
    let devices = result
        .get("devices")
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(0);
    if devices & KEYBOARD == 0 {
        forget_saved_token(&String::new());
        return Err("keyboard access was not granted".into());
    }
    if closed.next().now_or_never().is_some() {
        return Err("desktop keyboard session closed".into());
    }
    if let Some(restore) = result
        .get("restore_token")
        .and_then(|value| <&str>::try_from(value).ok())
    {
        if let Err(error) = save_token(restore) {
            log::warn!("Could not save keyboard permission token: {error}");
        }
    }
    publish_status(status, Status::Ready, generation);
    if saved_token.is_none() {
        notify("Reflow keyboard access is ready. Future dictations can paste automatically.");
    }
    loop {
        let command = tokio::select! {
            biased;
            _ = closed.next() => return Err("desktop keyboard session closed".into()),
            command = receiver.recv() => command,
        };
        let Some(command) = command else {
            return Ok(());
        };
        if let Some(error) = command.unavailable() {
            let _ = command.reply.send(Err(error));
            continue;
        }
        let result = send_events(portal, &session, &command).await;
        let failed = result.is_err();
        let expired_or_cancelled = command.unavailable().is_some();
        let _ = command.reply.send(result);
        if failed && !expired_or_cancelled {
            return Err("desktop keyboard input failed; keyboard session closed".into());
        }
    }
}

async fn while_session_open<T>(
    closed: &mut (impl futures_util::Stream + Unpin),
    action: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::select! {
        biased;
        _ = closed.next() => Err("desktop keyboard session closed".into()),
        result = action => result,
    }
}

async fn send_events(
    portal: &Proxy<'_>,
    session: &OwnedObjectPath,
    request: &Request,
) -> Result<(), String> {
    let mut pressed = Vec::new();
    let deadline = tokio::time::Instant::from_std(request.deadline);
    let options: HashMap<&str, Value<'_>> = HashMap::new();
    for &(keysym, state) in &request.events {
        if let Some(error) = request.unavailable() {
            release_pressed(portal, session, &pressed).await;
            return Err(error);
        }
        // Track before sending: the compositor can receive a press even when
        // its D-Bus acknowledgement fails or reaches the deadline.
        if state == 1 {
            pressed.push(keysym);
        }
        let result = tokio::time::timeout_at(
            deadline,
            portal.call::<_, _, ()>("NotifyKeyboardKeysym", &(session, &options, keysym, state)),
        )
        .await
        .map_err(|_| "desktop keyboard input timed out".to_owned())
        .and_then(|result| result.map_err(|error| error.to_string()));
        if let Err(error) = result {
            // Release any modifiers already pressed even if a later key fails.
            release_pressed(portal, session, &pressed).await;
            return Err(error);
        }
        if state == 0 {
            pressed.retain(|key| *key != keysym);
        }
    }
    Ok(())
}

async fn release_pressed(portal: &Proxy<'_>, session: &OwnedObjectPath, pressed: &[i32]) {
    let options: HashMap<&str, Value<'_>> = HashMap::new();
    for keysym in pressed.iter().rev() {
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            portal.call::<_, _, ()>("NotifyKeyboardKeysym", &(session, &options, *keysym, 0_u32)),
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_session_rejects_input_even_while_its_blocking_worker_is_alive() {
        let owner = Arc::new(AtomicBool::new(false));
        let (reply, _response) = std::sync::mpsc::sync_channel(1);
        let request = Request {
            events: chord_events(true, false, 'v' as i32),
            reply,
            deadline: Instant::now() + INPUT_TIMEOUT,
            wall_deadline: std::time::SystemTime::now() + INPUT_TIMEOUT,
            cancelled: Arc::new(AtomicBool::new(false)),
            owner_cancelled: Some(Arc::clone(&owner)),
        };
        assert!(request.unavailable().is_none());
        drop(crate::injection::cancellation::CancelOnDrop(owner));
        assert!(request.unavailable().unwrap().contains("cancelled"));
    }

    #[tokio::test]
    async fn revoked_keyboard_approval_wins_over_a_successful_portal_reply() {
        for mut closed in [
            futures_util::stream::iter(vec![()]),
            futures_util::stream::iter(vec![]),
        ] {
            let result = while_session_open(&mut closed, async { Ok(()) }).await;
            assert!(result.unwrap_err().contains("session closed"));
        }
        let mut closed = futures_util::stream::pending::<()>();
        assert!(while_session_open(&mut closed, async { Ok(()) })
            .await
            .is_ok());
    }

    #[test]
    fn paste_chord_releases_every_pressed_modifier_in_reverse_order() {
        assert_eq!(
            chord_events(true, true, 'v' as i32),
            vec![
                (0xffe3, 1),
                (0xffe1, 1),
                ('v' as i32, 1),
                ('v' as i32, 0),
                (0xffe1, 0),
                (0xffe3, 0),
            ]
        );
    }

    #[test]
    fn select_devices_persists_keyboard_access_with_optional_restore_token() {
        let fresh = select_devices_options("handle", None);
        assert_eq!(u32::try_from(&fresh["types"]).unwrap(), KEYBOARD);
        assert_eq!(u32::try_from(&fresh["persist_mode"]).unwrap(), 2);
        assert!(!fresh.contains_key("restore_token"));
        let restored = select_devices_options("handle", Some("saved-token"));
        assert_eq!(u32::try_from(&restored["persist_mode"]).unwrap(), 2);
        assert_eq!(
            <&str>::try_from(&restored["restore_token"]).unwrap(),
            "saved-token"
        );
    }

    #[test]
    fn pending_permission_never_queues_keys_for_later_delivery() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
        let worker = Worker {
            sender,
            status: Arc::new(parking_lot::Mutex::new(Status::Pending)),
        };
        assert!(
            send_to_worker(&worker, chord_events(true, false, 'v' as i32))
                .unwrap_err()
                .contains("Allow Reflow")
        );
        assert!(matches!(
            receiver.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        *worker.status.lock() = Status::Ready;
        assert!(matches!(
            receiver.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn denied_permission_remains_a_clipboard_fallback_without_reprompting() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
        let worker = Worker {
            sender,
            status: Arc::new(parking_lot::Mutex::new(Status::Failed(
                "keyboard permission cancelled".into(),
            ))),
        };
        for _ in 0..3 {
            assert!(
                send_to_worker(&worker, chord_events(true, false, 'v' as i32))
                    .unwrap_err()
                    .contains("cancelled")
            );
        }
        assert!(matches!(
            receiver.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn an_expired_queued_request_cannot_start_after_the_caller_gives_up() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
        let (reply, _response) = std::sync::mpsc::sync_channel(1);
        sender
            .try_send(Request {
                events: chord_events(true, false, 'v' as i32),
                reply,
                deadline: Instant::now() - Duration::from_millis(1),
                wall_deadline: std::time::SystemTime::now() + INPUT_TIMEOUT,
                cancelled: Arc::new(AtomicBool::new(false)),
                owner_cancelled: None,
            })
            .unwrap();
        let queued = receiver.try_recv().unwrap();
        assert!(queued.unavailable().unwrap().contains("expired"));
    }

    #[test]
    fn caller_return_or_unwind_cancels_all_remaining_key_presses() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let (reply, _response) = std::sync::mpsc::sync_channel(1);
        let request = Request {
            events: chord_events(true, false, 'v' as i32),
            reply,
            deadline: Instant::now() + INPUT_TIMEOUT,
            wall_deadline: std::time::SystemTime::now() + INPUT_TIMEOUT,
            cancelled: Arc::clone(&cancelled),
            owner_cancelled: None,
        };
        assert!(request.unavailable().is_none());
        drop(CancelOnDrop(cancelled));
        assert!(request.unavailable().unwrap().contains("cancelled"));
    }

    #[test]
    fn suspend_elapsed_wall_time_expires_input_even_if_monotonic_time_stops() {
        let (reply, _response) = std::sync::mpsc::sync_channel(1);
        let request = Request {
            events: chord_events(true, false, 'v' as i32),
            reply,
            deadline: Instant::now() + INPUT_TIMEOUT,
            wall_deadline: std::time::SystemTime::now() - Duration::from_secs(1),
            cancelled: Arc::new(AtomicBool::new(false)),
            owner_cancelled: None,
        };
        assert!(request.unavailable().unwrap().contains("expired"));
    }

    #[test]
    #[ignore = "requires user-approved keyboard access and an explicitly prepared isolated input field"]
    fn keyboard_portal_types_only_into_an_explicitly_prepared_test_field() {
        assert_eq!(std::env::var("REFLOW_PORTAL_TEST_TARGET_READY").as_deref(), Ok("isolated-test-field"), "Do not run against the user's active application; prepare and focus an isolated test field first");
        let _ = request_access();
        let deadline = std::time::Instant::now() + APPROVAL_TIMEOUT;
        loop {
            match worker().status.lock().clone() {
                Status::Ready => break,
                Status::Failed(error) => panic!("keyboard permission failed: {error}"),
                Status::Pending => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "approve the keyboard-only desktop request to continue"
                    );
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
        eprintln!("Keyboard permission approved. Focus the isolated test field within 10 seconds. This test sends no Enter or send action.");
        std::thread::sleep(Duration::from_secs(10));
        type_text("Reflow native keyboard check")
            .expect("send text through the approved keyboard portal");
    }
}
