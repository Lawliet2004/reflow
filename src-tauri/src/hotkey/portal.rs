//! Native Wayland shortcuts. X11 grabs cannot receive keys in Wayland apps;
//! the desktop portal provides activation and release events with user consent.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures_util::{FutureExt, StreamExt};
use tauri::{AppHandle, Emitter, Manager};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, Proxy};

use crate::commands::AppContext;
use crate::hotkey::hook::platform::ComboFn;
use crate::platform::desktop::{self, ShortcutState};

const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);
const METHOD_TIMEOUT: Duration = Duration::from_secs(15);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static TASK: parking_lot::Mutex<Option<tauri::async_runtime::JoinHandle<()>>> =
    parking_lot::Mutex::new(None);
type ConfigureReply = tokio::sync::oneshot::Sender<Result<(), String>>;
static CONFIGURE: parking_lot::Mutex<Option<tokio::sync::mpsc::UnboundedSender<ConfigureReply>>> =
    parking_lot::Mutex::new(None);

pub struct PortalBinding {
    pub action: String,
    pub shortcut: String,
    pub press: ComboFn,
    pub release: ComboFn,
    pub toggle: ComboFn,
    pub cancel: ComboFn,
}

/// Remember the mode at key-down and suppress repeats. A physical key-up
/// finishes hold-to-talk; cancellation is reserved for revocation/rebinding.
fn dispatch_event(
    binding: &PortalBinding,
    pressed: bool,
    push_to_talk: bool,
    held: &mut HashMap<String, bool>,
    at: Instant,
) {
    if pressed {
        if held.contains_key(&binding.action) {
            return;
        }
        held.insert(binding.action.clone(), push_to_talk);
        if push_to_talk {
            (binding.press)(at);
        } else {
            (binding.toggle)(at);
        }
    } else if held.remove(&binding.action) == Some(true) {
        (binding.release)(at);
    }
}

pub fn clear() {
    CONFIGURE.lock().take();
    GENERATION.fetch_add(1, Ordering::AcqRel);
    if let Some(task) = TASK.lock().take() {
        task.abort();
    }
}

pub fn register(app: AppHandle, bindings: Vec<PortalBinding>) {
    clear();
    let generation = GENERATION.load(Ordering::Acquire);
    let pending = "Allow Reflow's global shortcuts in the desktop permission dialog. You can use Record in Reflow while waiting.";
    *app.state::<AppContext>().hotkey_error.write() = Some(pending.into());
    let _ = app.emit("hotkey:error", pending);
    desktop::shortcut(ShortcutState::Pending, None, Some(pending.into()));
    let (configure, configure_requests) = tokio::sync::mpsc::unbounded_channel();
    *CONFIGURE.lock() = Some(configure);
    *TASK.lock() = Some(tauri::async_runtime::spawn(async move {
        if let Err(error) = listen(&app, bindings, generation, configure_requests).await {
            if GENERATION.load(Ordering::Acquire) == generation {
                let message = format!(
                    "Wayland global shortcut unavailable: {error}. Use Record in Reflow, or allow shortcuts in your desktop settings."
                );
                log::error!("{message}");
                *app.state::<AppContext>().hotkey_error.write() = Some(message.clone());
                desktop::shortcut(ShortcutState::Unavailable, None, Some(message.clone()));
                let _ = app.emit("hotkey:error", message);
            }
        }
    }));
}

pub fn configure() -> Result<tokio::sync::oneshot::Receiver<Result<(), String>>, String> {
    let (reply, response) = tokio::sync::oneshot::channel();
    CONFIGURE
        .lock()
        .as_ref()
        .ok_or_else(|| "Reconnect shortcuts in Settings → General".to_owned())?
        .send(reply)
        .map_err(|_| "Reconnect shortcuts in Settings → General".to_owned())?;
    Ok(response)
}

type BoundShortcuts = Vec<(String, HashMap<String, OwnedValue>)>;

fn dictation_trigger(bound: &BoundShortcuts) -> Option<&str> {
    bound
        .iter()
        .find(|(id, _)| id == "dictate")
        .and_then(|(_, properties)| properties.get("trigger_description"))
        .and_then(|value| <&str>::try_from(value).ok())
        .filter(|description| !description.trim().is_empty())
}

fn update_status(app: &AppHandle, bound: &BoundShortcuts) {
    let assignments = bound
        .iter()
        .filter_map(|(id, properties)| {
            properties
                .get("trigger_description")
                .and_then(|value| <&str>::try_from(value).ok())
                .filter(|trigger| !trigger.trim().is_empty())
                .map(|trigger| (id.clone(), trigger.to_owned()))
        })
        .collect();
    desktop::assignments(assignments);
    if let Some(trigger) = dictation_trigger(bound) {
        *app.state::<AppContext>().hotkey_error.write() = None;
        desktop::shortcut(ShortcutState::Ready, Some(trigger.into()), None);
        let _ = app.emit("hotkey:ready", trigger);
        log::info!("Native Wayland dictation shortcut ready: {trigger}");
    } else {
        let message = if crate::platform::kde_shortcuts::supported_desktop() {
            "No dictation key is assigned. Open Settings → General, click Choose shortcut, then press your preferred combination. You can use Record in Reflow meanwhile."
        } else {
            "No dictation key is assigned by your Wayland desktop. Choose Configure desktop shortcuts below and assign your dictation shortcut. You can use Record in Reflow meanwhile."
        };
        *app.state::<AppContext>().hotkey_error.write() = Some(message.into());
        desktop::shortcut(ShortcutState::Unassigned, None, Some(message.into()));
        let _ = app.emit("hotkey:error", message);
        log::warn!("{message}");
    }
}

fn preferred_trigger(shortcut: &str) -> String {
    shortcut
        .split('+')
        .map(|part| match part {
            "Ctrl" | "Control" => "CTRL".into(),
            "Alt" => "ALT".into(),
            "Shift" => "SHIFT".into(),
            "Super" | "Win" | "Meta" => "LOGO".into(),
            "Space" => "space".into(),
            "Enter" => "Return".into(),
            "Esc" => "Escape".into(),
            "ArrowLeft" => "Left".into(),
            "ArrowRight" => "Right".into(),
            "ArrowUp" => "Up".into(),
            "ArrowDown" => "Down".into(),
            "Backspace" => "BackSpace".into(),
            "PageUp" => "Prior".into(),
            "PageDown" => "Next".into(),
            "Minus" => "minus".into(),
            "Equal" => "equal".into(),
            "Comma" => "comma".into(),
            "Period" => "period".into(),
            "Slash" => "slash".into(),
            "Backslash" => "backslash".into(),
            "Semicolon" => "semicolon".into(),
            "Quote" => "apostrophe".into(),
            "Backquote" => "grave".into(),
            "BracketLeft" => "bracketleft".into(),
            "BracketRight" => "bracketright".into(),
            key if key.starts_with("Key") && key.len() == 4 => key[3..].to_lowercase(),
            key if key.starts_with("Digit") && key.len() == 6 => key[5..].into(),
            key if key.len() == 1 => key.to_lowercase(),
            key => key.into(),
        })
        .collect::<Vec<String>>()
        .join("+")
}

async fn request_response(
    stream: &mut zbus::proxy::SignalStream<'_>,
) -> Result<HashMap<String, OwnedValue>, String> {
    let message = tokio::time::timeout(APPROVAL_TIMEOUT, stream.next())
        .await
        .map_err(|_| "desktop approval timed out".to_owned())?
        .ok_or_else(|| "desktop portal disconnected".to_owned())?;
    let (status, results): (u32, HashMap<String, OwnedValue>) = message
        .body()
        .deserialize()
        .map_err(|error| error.to_string())?;
    match status {
        0 => Ok(results),
        1 => Err("desktop shortcut permission was cancelled".into()),
        _ => Err(format!(
            "desktop portal rejected the shortcut request ({status})"
        )),
    }
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

struct ReleaseOnDisconnect<'a>(&'a [PortalBinding]);

impl Drop for ReleaseOnDisconnect<'_> {
    fn drop(&mut self) {
        // A revoked session, bus disconnect, or aborted registration may never
        // send key-up. Stop only recordings owned by these binding callbacks.
        let at = Instant::now();
        for binding in self.0 {
            (binding.cancel)(at);
        }
    }
}

fn closed_session_error() -> String {
    "the desktop revoked or closed its global shortcut session".into()
}

async fn listen(
    app: &AppHandle,
    bindings: Vec<PortalBinding>,
    generation: u64,
    configure_requests: tokio::sync::mpsc::UnboundedReceiver<ConfigureReply>,
) -> Result<(), String> {
    let _release_on_disconnect = ReleaseOnDisconnect(&bindings);
    // This connection belongs to this registration. Dropping it on rebind or
    // quit closes the portal session and releases its global shortcuts.
    let connection = zbus::connection::Builder::session()
        .map_err(|error| error.to_string())?
        .method_timeout(METHOD_TIMEOUT)
        .build()
        .await
        .map_err(|error| error.to_string())?;
    register_application(&connection, &app.config().identifier).await?;
    let portal = Proxy::new(&connection, DESTINATION, PATH, INTERFACE)
        .await
        .map_err(|error| error.to_string())?;
    portal
        .get_property::<u32>("version")
        .await
        .map_err(|_| "this desktop does not provide the GlobalShortcuts portal".to_owned())?;

    let mut owners = portal
        .receive_owner_changed()
        .await
        .map_err(|error| error.to_string())?;
    tokio::select! {
        biased;
        _ = owners.next() => Err("desktop portal restarted or disconnected; reconnect in Settings → General".into()),
        result = serve_session(app, &bindings, generation, &connection, &portal, configure_requests) => result,
    }
}

/// Session creation and shortcut binding, then the press/release event loop.
async fn serve_session(
    app: &AppHandle,
    bindings: &[PortalBinding],
    generation: u64,
    connection: &Connection,
    portal: &Proxy<'_>,
    mut configure_requests: tokio::sync::mpsc::UnboundedReceiver<ConfigureReply>,
) -> Result<(), String> {
    let token = format!("reflow_create_{}", uuid::Uuid::new_v4().simple());
    let response_path = request_path(connection, &token);
    let response = Proxy::new(
        connection,
        DESTINATION,
        response_path.as_str(),
        "org.freedesktop.portal.Request",
    )
    .await
    .map_err(|error| error.to_string())?;
    // Subscribe before calling: a portal can respond before its method reply.
    let mut responses = response
        .receive_signal("Response")
        .await
        .map_err(|error| error.to_string())?;
    let mut pending_create =
        crate::platform::portal_request::PendingRequest::new(connection, &response_path);
    let session_token = format!("reflow_session_{}", uuid::Uuid::new_v4().simple());
    let options = HashMap::from([
        ("handle_token", Value::from(token.as_str())),
        ("session_handle_token", Value::from(session_token.as_str())),
    ]);
    let _: OwnedObjectPath = portal
        .call("CreateSession", &(options,))
        .await
        .map_err(|error| error.to_string())?;
    let results = request_response(&mut responses).await?;
    pending_create.responded();
    let session_handle = results
        .get("session_handle")
        .and_then(|value| <&str>::try_from(value).ok())
        .ok_or_else(|| "desktop portal returned no shortcut session".to_owned())?;
    let session = OwnedObjectPath::try_from(session_handle).map_err(|error| error.to_string())?;

    let session_proxy = Proxy::new(
        connection,
        DESTINATION,
        session.as_str(),
        "org.freedesktop.portal.Session",
    )
    .await
    .map_err(|error| error.to_string())?;
    let mut closed = session_proxy
        .receive_signal("Closed")
        .await
        .map_err(|error| error.to_string())?;
    // Subscribe before binding, and keep press/release on one stream so a
    // quick key-up cannot be lost or reordered behind another key press.
    let mut events = portal
        .receive_all_signals()
        .await
        .map_err(|error| error.to_string())?;

    let token = format!("reflow_bind_{}", uuid::Uuid::new_v4().simple());
    let response_path = request_path(connection, &token);
    let response = Proxy::new(
        connection,
        DESTINATION,
        response_path.as_str(),
        "org.freedesktop.portal.Request",
    )
    .await
    .map_err(|error| error.to_string())?;
    let mut responses = response
        .receive_signal("Response")
        .await
        .map_err(|error| error.to_string())?;
    let triggers: Vec<String> = bindings
        .iter()
        .map(|binding| preferred_trigger(&binding.shortcut))
        .collect();
    let shortcuts: Vec<(&str, HashMap<&str, Value<'_>>)> = bindings
        .iter()
        .zip(&triggers)
        .map(|(binding, trigger)| {
            (
                binding.action.as_str(),
                HashMap::from([
                    ("description", Value::from(binding.action.as_str())),
                    ("preferred_trigger", Value::from(trigger.as_str())),
                ]),
            )
        })
        .collect();
    let options = HashMap::from([("handle_token", Value::from(token.as_str()))]);
    let bind_request = (&session, shortcuts, "", options);
    let mut pending_bind =
        crate::platform::portal_request::PendingRequest::new(connection, &response_path);
    let _: OwnedObjectPath = tokio::select! {
        biased;
        _ = closed.next() => return Err(closed_session_error()),
        reply = portal.call("BindShortcuts", &bind_request) => reply.map_err(|error| error.to_string())?,
    };
    let mut result = tokio::select! {
        biased;
        _ = closed.next() => return Err(closed_session_error()),
        response = request_response(&mut responses) => response?,
    };
    pending_bind.responded();
    let bound: BoundShortcuts = result
        .remove("shortcuts")
        .ok_or_else(|| "desktop portal returned no bound shortcuts".to_owned())?
        .try_into()
        .map_err(|error: zbus::zvariant::Error| error.to_string())?;
    if !bound.iter().any(|(id, _)| id == "dictate") {
        return Err("the dictation shortcut was not approved".into());
    }
    if GENERATION.load(Ordering::Acquire) != generation {
        return Ok(());
    }
    // A close queued while setting up activation listeners must win over the
    // successful bind reply; never publish a ready status for a revoked session.
    if closed.next().now_or_never().is_some() {
        return Err(closed_session_error());
    }
    update_status(app, &bound);
    let mut held = HashMap::new();
    loop {
        let message = tokio::select! {
            biased;
            _ = closed.next() => return Err(closed_session_error()),
            message = events.next() => message,
            Some(reply) = configure_requests.recv() => {
                // Desktop configuration can be slow. It must not block the
                // release or revocation signals needed to stop recording.
                let connection = connection.clone();
                let session = session.clone();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let result = configure_session(connection, session, app).await;
                    if let Err(error) = &result {
                        log::warn!("Could not open desktop shortcut settings: {error}");
                    }
                    let _ = reply.send(result);
                });
                continue;
            },
        };
        let Some(message) = message else {
            return Err("desktop shortcut session disconnected".into());
        };
        if GENERATION.load(Ordering::Acquire) != generation {
            return Ok(());
        }
        let header = message.header();
        let member = header.member().map(|member| member.as_str());
        if member == Some("ShortcutsChanged") {
            let (changed_session, bound): (OwnedObjectPath, BoundShortcuts) = message
                .body()
                .deserialize()
                .map_err(|error| error.to_string())?;
            if changed_session == session {
                update_status(app, &bound);
            }
            continue;
        }
        let pressed = match member {
            Some("Activated") => true,
            Some("Deactivated") => false,
            _ => continue,
        };
        let (event_session, action, _, _): (
            OwnedObjectPath,
            String,
            u64,
            HashMap<String, OwnedValue>,
        ) = message
            .body()
            .deserialize()
            .map_err(|error| error.to_string())?;
        if event_session != session {
            continue;
        }
        if let Some(binding) = bindings.iter().find(|binding| binding.action == action) {
            let at = Instant::now();
            let push_to_talk = app.state::<AppContext>().settings_store.get().push_to_talk;
            log::debug!(
                "Wayland shortcut action={action} pressed={pressed} generation={generation}"
            );
            dispatch_event(binding, pressed, push_to_talk, &mut held, at);
        }
    }
}

async fn configure_session(
    connection: Connection,
    session: OwnedObjectPath,
    app: AppHandle,
) -> Result<(), String> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if desktop.split(':').any(|desktop| {
        desktop.eq_ignore_ascii_case("kde") || desktop.eq_ignore_ascii_case("plasma")
    }) {
        // KDE's portal opens a systemsettings URL without an activation token.
        // An existing Settings window can stay behind Reflow. Open a dedicated
        // Shortcuts window instead, selecting this application's actions.
        let args = ["kcm_keys", "--args", app.config().identifier.as_str()];
        let mut failures = Vec::new();
        for program in ["kcmshell6", "systemsettings"] {
            match launch_settings(program, &args).await {
                Ok(()) => {
                    log::info!("Opened KDE shortcut settings with {program}");
                    return Ok(());
                }
                Err(error) => failures.push(error),
            }
        }
        return Err(format!(
            "{}. Open System Settings → Keyboard → Shortcuts → Reflow.",
            failures.join("; ")
        ));
    }
    let portal = Proxy::new(&connection, DESTINATION, PATH, INTERFACE)
        .await
        .map_err(|error| error.to_string())?;
    if portal
        .get_property::<u32>("version")
        .await
        .map_err(|error| error.to_string())?
        < 2
    {
        return Err("This desktop portal cannot open shortcut configuration. Open keyboard shortcuts in your desktop settings and select Reflow.".into());
    }
    let options: HashMap<&str, Value<'_>> = HashMap::new();
    portal
        .call::<_, _, ()>("ConfigureShortcuts", &(&session, "", options))
        .await
        .map_err(|error| {
            format!("{error}. Open keyboard shortcuts in your desktop settings and select Reflow.")
        })
}

async fn launch_settings(program: &str, args: &[&str]) -> Result<(), String> {
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not launch {program}: {error}"))?;
    match tokio::time::timeout(Duration::from_millis(300), child.wait()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => Err(format!("{program} exited before opening ({status})")),
        Ok(Err(error)) => Err(format!("Could not launch {program}: {error}")),
        Err(_) => {
            // Keep the window alive and reap the process when it closes.
            tauri::async_runtime::spawn(async move {
                let _ = child.wait().await;
            });
            Ok(())
        }
    }
}

/// Modern portals require an explicit app identity for unsandboxed executables,
/// including `tauri dev`. The hidden entry supplies desktop metadata without
/// replacing an installed or user-customized launcher, or enabling autostart.
pub(crate) async fn register_application(
    connection: &Connection,
    app_id: &str,
) -> Result<(), String> {
    if std::path::Path::new("/.flatpak-info").exists() || std::env::var_os("SNAP").is_some() {
        return Ok(()); // Sandboxed identities are supplied by the portal itself.
    }
    ensure_desktop_identity(app_id)?;
    let registry = Proxy::new(
        connection,
        DESTINATION,
        PATH,
        "org.freedesktop.host.portal.Registry",
    )
    .await
    .map_err(|error| error.to_string())?;
    let options: HashMap<&str, Value<'_>> = HashMap::new();
    match registry
        .call::<_, _, ()>("Register", &(app_id, options))
        .await
    {
        Ok(()) => Ok(()),
        // Older portals derive the identity without this interface.
        Err(zbus::Error::MethodError(name, _, _))
            if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod"
                || name.as_str() == "org.freedesktop.DBus.Error.UnknownInterface" =>
        {
            Ok(())
        }
        Err(error) => Err(format!(
            "could not register Reflow's desktop identity: {error}"
        )),
    }
}

fn ensure_desktop_identity(app_id: &str) -> Result<(), String> {
    if app_id.is_empty()
        || !app_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
    {
        return Err("invalid application desktop identifier".into());
    }
    let filename = format!("{app_id}.desktop");
    let data_dir = dirs::data_dir()
        .ok_or_else(|| "could not locate your application data directory".to_owned())?;
    let mut directories = vec![data_dir.clone()];
    directories.extend(std::env::split_paths(
        &std::env::var_os("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into()),
    ));
    if directories
        .iter()
        .any(|directory| directory.join("applications").join(&filename).is_file())
    {
        return Ok(());
    }
    let applications = data_dir.join("applications");
    std::fs::create_dir_all(&applications).map_err(|error| error.to_string())?;
    let executable = crate::platform::linux_desktop::current_launcher_executable()?;
    let exec = crate::platform::linux_desktop::desktop_exec(&executable)?;
    let content = format!("[Desktop Entry]\nType=Application\nName=Reflow\nComment=Local speech dictation\nExec={exec}\nTerminal=false\nNoDisplay=true\nCategories=Utility;AudioVideo;\n");
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(applications.join(filename))
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(content.as_bytes())
                .map_err(|error| error.to_string())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(format!("could not save Reflow's desktop identity: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        closed_session_error, dictation_trigger, launch_settings, preferred_trigger, PortalBinding,
        ReleaseOnDisconnect,
    };
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use zbus::zvariant::{OwnedValue, Value};

    #[test]
    fn hold_release_finishes_and_never_cancels_even_if_mode_changes() {
        let calls = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let callback = |name| {
            let calls = Arc::clone(&calls);
            Arc::new(move |_| calls.lock().push(name)) as crate::hotkey::hook::platform::ComboFn
        };
        let binding = PortalBinding {
            action: "dictate".into(),
            shortcut: "Ctrl+Shift+Space".into(),
            press: callback("press"),
            release: callback("release"),
            toggle: callback("toggle"),
            cancel: callback("cancel"),
        };
        let mut held = HashMap::new();
        let at = std::time::Instant::now();
        // Native event contract, including repeats and a duplicate release.
        for _ in 0..30 {
            super::dispatch_event(&binding, true, true, &mut held, at);
            super::dispatch_event(&binding, true, true, &mut held, at);
            super::dispatch_event(&binding, false, false, &mut held, at);
            super::dispatch_event(&binding, false, false, &mut held, at);
        }
        assert_eq!(*calls.lock(), ["press", "release"].repeat(30));
        assert!(held.is_empty());
        calls.lock().clear();
        for _ in 0..2 {
            super::dispatch_event(&binding, true, false, &mut held, at);
            super::dispatch_event(&binding, true, false, &mut held, at);
            super::dispatch_event(&binding, false, true, &mut held, at);
        }
        assert_eq!(*calls.lock(), ["toggle", "toggle"]);
    }

    #[tokio::test]
    async fn settings_launcher_reports_missing_programs_and_early_failures() {
        let missing = launch_settings("/reflow-test-missing-shortcut-settings", &[])
            .await
            .unwrap_err();
        assert!(missing.contains("Could not launch"));
        let failed = launch_settings("sh", &["-c", "exit 7"]).await.unwrap_err();
        assert!(failed.contains("exited before opening"));
        assert!(failed.contains('7'));
    }

    #[test]
    fn shortcut_triggers_use_xdg_keysyms_and_modifier_names() {
        assert_eq!(preferred_trigger("Control+Shift+Space"), "CTRL+SHIFT+space");
        assert_eq!(preferred_trigger("Alt+Enter"), "ALT+Return");
        assert_eq!(preferred_trigger("Super+KeyK"), "LOGO+k");
        assert_eq!(preferred_trigger("Control+Digit1"), "CTRL+1");
        assert_eq!(preferred_trigger("Control+F9"), "CTRL+F9");
    }

    #[test]
    fn a_bound_shortcut_without_an_assigned_key_is_not_ready() {
        let properties = |trigger: &str| {
            HashMap::from([(
                "trigger_description".into(),
                OwnedValue::try_from(Value::from(trigger)).unwrap(),
            )])
        };
        assert!(dictation_trigger(&vec![("dictate".into(), properties(""))]).is_none());
        assert!(
            dictation_trigger(&vec![("command".into(), properties("Ctrl+Alt+Space"))]).is_none()
        );
        assert_eq!(
            dictation_trigger(&vec![("dictate".into(), properties("Ctrl+Shift+Space"))]),
            Some("Ctrl+Shift+Space")
        );
    }

    #[test]
    fn closed_sessions_release_recordings_owned_by_each_binding() {
        let released = Arc::new(AtomicUsize::new(0));
        let bindings = ["dictate", "command"]
            .into_iter()
            .map(|action| {
                let released = Arc::clone(&released);
                PortalBinding {
                    action: action.into(),
                    shortcut: "Ctrl+Shift+Space".into(),
                    press: Arc::new(|_| {}),
                    toggle: Arc::new(|_| {}),
                    release: Arc::new(|_| {}),
                    cancel: Arc::new(move |_| {
                        released.fetch_add(1, Ordering::AcqRel);
                    }),
                }
            })
            .collect::<Vec<_>>();
        assert!(closed_session_error().contains("revoked or closed"));
        drop(ReleaseOnDisconnect(&bindings));
        assert_eq!(released.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn aborting_a_shortcut_listener_releases_its_owned_recording() {
        let released = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&released);
        let (ready, started) = tokio::sync::oneshot::channel();
        let listener = tokio::spawn(async move {
            let bindings = vec![PortalBinding {
                action: "dictate".into(),
                shortcut: "Control+Shift+Space".into(),
                press: Arc::new(|_| {}),
                toggle: Arc::new(|_| {}),
                release: Arc::new(|_| {}),
                cancel: Arc::new(move |_| {
                    count.fetch_add(1, Ordering::AcqRel);
                }),
            }];
            let _guard = ReleaseOnDisconnect(&bindings);
            ready.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        started.await.unwrap();
        listener.abort();
        assert!(listener.await.unwrap_err().is_cancelled());
        assert_eq!(released.load(Ordering::Acquire), 1);
    }
}
