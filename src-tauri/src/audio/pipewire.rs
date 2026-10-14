//! PipeWire sources listed through `pactl` and recorded through `pw-record`.
//! cpal's ALSA host exposes only generic PCMs, so selected Linux inputs and
//! monitors are captured by a subprocess that targets the PipeWire node.
use super::capture::AudioDeviceInfo;
use parking_lot::Mutex;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, ErrorKind, Read};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub const ID_PREFIX: &str = "pipewire:";
pub const AUTO_ID: &str = "pipewire:@auto";
pub const NO_MICROPHONE: &str =
    "No microphone is available. Connect one or choose another input in Audio settings.";
const AUTO_NAME: &str = "Automatic (headset when connected, otherwise built-in mic)";
const APP_NAME: &str = "Reflow";
const RATE: u32 = 16_000;
const CHUNK_BYTES: usize = 2560;
const PACTL_TIMEOUT: Duration = Duration::from_secs(2);
const UNAVAILABLE: &str =
    "The selected microphone is unavailable. Reconnect it or choose another input in Audio settings.";
pub(crate) const MISSING_TOOL: &str = "Recording from a selected Linux microphone needs pw-record (package pipewire-bin). Install it or choose System default.";

#[derive(Deserialize)]
struct PactlSource {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    sample_specification: String,
    #[serde(default)]
    properties: HashMap<String, serde_json::Value>,
    #[serde(default)]
    active_port: Option<String>,
    #[serde(default)]
    ports: Vec<PactlPort>,
}

#[derive(Deserialize)]
struct PactlPort {
    #[serde(default)]
    name: String,
    #[serde(default)]
    availability: String,
}

impl PactlSource {
    fn is_available(&self) -> bool {
        if self.name.ends_with(".monitor") {
            return true;
        }
        // An unplugged analog microphone jack still appears as an ALSA
        // source. It must not be mistaken for the laptop's internal mic.
        if let Some(port) = self
            .ports
            .iter()
            .find(|port| Some(port.name.as_str()) == self.active_port.as_deref())
        {
            return port.availability != "not available";
        }
        self.ports.is_empty()
            || self
                .ports
                .iter()
                .any(|port| port.availability != "not available")
    }

    fn property(&self, key: &str) -> String {
        match self.properties.get(key) {
            Some(serde_json::Value::String(value)) => value.clone(),
            Some(serde_json::Value::Number(value)) => value.to_string(),
            _ => String::new(),
        }
    }

    fn is_webcam(&self) -> bool {
        self.property("device.form_factor") == "webcam"
    }

    fn is_headset(&self) -> bool {
        !self.is_webcam()
            && (matches!(self.property("device.bus").as_str(), "bluetooth" | "usb")
                || matches!(
                    self.active_port.as_deref().unwrap_or_default(),
                    "analog-input-mic"
                        | "analog-input-headset-mic"
                        | "analog-input-front-mic"
                        | "analog-input-rear-mic"
                )
                || matches!(
                    self.property("device.form_factor").as_str(),
                    "headset" | "headphone" | "handsfree" | "hands-free"
                ))
    }

    fn is_builtin(&self) -> bool {
        !self.is_webcam()
            && (self.property("device.bus") == "pci"
                || self.property("device.form_factor") == "internal")
    }

    fn priority(&self) -> i64 {
        self.property("priority.session").parse().unwrap_or(0)
    }
}

fn parse_spec(spec: &str) -> (u32, u16) {
    let mut sample_rate: u32 = 0;
    let mut channels: u16 = 0;
    for token in spec.split_whitespace() {
        if let Some(value) = token.strip_suffix("Hz") {
            sample_rate = value.parse().unwrap_or(0);
        } else if let Some(value) = token.strip_suffix("ch") {
            channels = value.parse().unwrap_or(0);
        }
    }
    (sample_rate, channels)
}

pub(crate) fn parse_sources(
    json: &str,
    default_source: &str,
) -> Result<Vec<AudioDeviceInfo>, String> {
    let sources: Vec<PactlSource> =
        serde_json::from_str(json).map_err(|e| format!("Could not read PipeWire sources: {e}"))?;
    let mut devices: Vec<AudioDeviceInfo> = sources
        .into_iter()
        .map(|source| {
            let (sample_rate, channels) = parse_spec(&source.sample_specification);
            let id = format!("{ID_PREFIX}{}", source.name);
            let is_default = source.name == default_source;
            let is_monitor = source.name.ends_with(".monitor");
            // Unplugged jacks are listed too — the pickers show them disabled
            // so a user can see the hardware exists, but they can never be
            // selected for capture.
            let is_available = source.is_available();
            let name = if source.description.is_empty() {
                source.name
            } else {
                source.description
            };
            AudioDeviceInfo {
                id,
                name,
                is_default,
                is_monitor,
                is_available,
                sample_rate,
                channels,
            }
        })
        .collect();
    devices.sort_by_key(|device| {
        if device.is_default {
            0
        } else if device.is_monitor {
            2
        } else {
            1
        }
    });
    Ok(devices)
}

fn pick_in_tier<'a>(
    candidates: &[&'a PactlSource],
    default_source: &str,
) -> Option<&'a PactlSource> {
    if let Some(default) = candidates.iter().find(|s| s.name == default_source) {
        return Some(default);
    }
    // Highest session priority wins; the first candidate keeps ties.
    candidates.iter().copied().reduce(|best, source| {
        if source.priority() > best.priority() {
            source
        } else {
            best
        }
    })
}

pub(crate) fn pick_automatic(json: &str, default_source: &str) -> Option<(String, String)> {
    let sources: Vec<PactlSource> = serde_json::from_str(json).ok()?;
    let inputs: Vec<&PactlSource> = sources
        .iter()
        .filter(|source| !source.name.ends_with(".monitor") && source.is_available())
        .collect();
    let headsets: Vec<&PactlSource> = inputs
        .iter()
        .copied()
        .filter(|source| source.is_headset())
        .collect();
    let builtins: Vec<&PactlSource> = inputs
        .iter()
        .copied()
        .filter(|source| source.is_builtin())
        .collect();
    pick_in_tier(&headsets, default_source)
        .or_else(|| pick_in_tier(&builtins, default_source))
        .or_else(|| inputs.iter().copied().find(|s| s.name == default_source))
        .map(|source| {
            let description = if source.description.is_empty() {
                source.name.clone()
            } else {
                source.description.clone()
            };
            (source.name.clone(), description)
        })
}

fn with_automatic(mut devices: Vec<AudioDeviceInfo>) -> Vec<AudioDeviceInfo> {
    devices.insert(
        0,
        AudioDeviceInfo {
            id: AUTO_ID.to_string(),
            name: AUTO_NAME.to_string(),
            is_default: false,
            is_monitor: false,
            is_available: true,
            sample_rate: 0,
            channels: 0,
        },
    );
    devices
}

/// `true` when `pactl` talks to a PipeWire (pipewire-pulse) server. A pure
/// PulseAudio daemon answers `pactl` too — listing works, but `pw-record`
/// cannot connect — so discovery and resolution must fall back to cpal.
/// Text form — the label is localized, so callers run pactl under LC_ALL=C.
pub(crate) fn is_pipewire_server(pactl_info: &str) -> bool {
    pactl_info
        .lines()
        .any(|line| line.starts_with("Server Name:") && line.contains("PipeWire"))
}

/// JSON form of the same check — `pactl -f json info` is locale-independent.
pub(crate) fn server_name_json_is_pipewire(info_json: &str) -> Option<bool> {
    let value: serde_json::Value = serde_json::from_str(info_json).ok()?;
    value
        .get("server_name")
        .and_then(|v| v.as_str())
        .map(|name| name.contains("PipeWire"))
}

fn probe_pipewire_server() -> bool {
    if let Some(json) = run_pactl(&["-f", "json", "info"]) {
        if let Some(result) = server_name_json_is_pipewire(&json) {
            return result;
        }
    }
    // Older pactl without JSON output — force English for the text parse.
    if let Some(text) = run_pactl_env(&["info"], &[("LC_ALL", "C")]) {
        return is_pipewire_server(&text);
    }
    false
}

/// A positive answer is cached forever (a server never turns into pure
/// PulseAudio); a failed or absent probe retries after 5s so a daemon that
/// starts after Reflow becomes visible again.
fn pipewire_server_present() -> bool {
    probe_rate_limited(&SERVER_PROBE, probe_pipewire_server)
}

static SERVER_PROBE: Mutex<Option<(bool, std::time::Instant)>> = Mutex::new(None);

fn probe_rate_limited(
    slot: &'static Mutex<Option<(bool, std::time::Instant)>>,
    probe: impl Fn() -> bool,
) -> bool {
    {
        let cache = slot.lock();
        match *cache {
            Some((true, _)) => return true,
            Some((false, at)) if at.elapsed() < Duration::from_secs(5) => {
                return false;
            }
            _ => {}
        }
    }
    let present = probe();
    *slot.lock() = Some((present, std::time::Instant::now()));
    present
}

/// `true` when a Pulse/PipeWire server answers and its default source would
/// capture playback or nothing — an ALSA `default` PCM routed through the
/// server is then not a usable microphone.
pub fn server_default_is_unusable() -> bool {
    let Some(json) = run_pactl(&["-f", "json", "list", "sources"]) else {
        return false; // No server: a plain-ALSA `default` is real hardware.
    };
    let default = default_source_name().unwrap_or_default();
    default_source_unusable(&json, &default)
}

/// A server-routed `default` captures silence when the server's default is a
/// monitor or no usable capture source exists.
fn default_source_unusable(json: &str, default: &str) -> bool {
    if default.ends_with(".monitor") {
        return true;
    }
    pick_automatic(json, default).is_none() && pick_system_microphone(json, &default).is_none()
}

fn run_pactl(args: &[&str]) -> Option<String> {
    run_pactl_env(args, &[])
}

fn run_pactl_env(args: &[&str], env: &[(&str, &str)]) -> Option<String> {
    crate::platform::process::output_env("pactl", args, PACTL_TIMEOUT, 4 * 1024 * 1024, env).ok()
}

fn default_source_name() -> Option<String> {
    run_pactl(&["get-default-source"])
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

pub fn default_sink() -> Option<String> {
    run_pactl(&["get-default-sink"])
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

pub fn list_sources() -> Option<Vec<AudioDeviceInfo>> {
    if !pipewire_server_present() {
        return None;
    }
    let json = run_pactl(&["-f", "json", "list", "sources"])?;
    let default = default_source_name().unwrap_or_default();
    let devices = parse_sources(&json, &default).ok()?;
    (!devices.is_empty()).then(|| with_automatic(devices))
}

pub fn resolve_automatic() -> Option<(String, String)> {
    let json = run_pactl(&["-f", "json", "list", "sources"])?;
    let default = default_source_name().unwrap_or_default();
    pick_automatic(&json, &default)
}

/// Automatic resolution where "no pick" is a real error: the caller already
/// committed to the PipeWire path (a vanished saved node, or the explicit
/// Automatic entry) and cpal would silently capture a monitor or nothing.
fn resolve_automatic_or_error() -> Result<Option<(String, String)>, String> {
    if !pipewire_server_present() {
        return Ok(None); // No PipeWire server at all — let the cpal path try.
    }
    let Some(json) = run_pactl(&["-f", "json", "list", "sources"]) else {
        return Ok(None); // Preserve ALSA-only systems without a Pulse/PipeWire service.
    };
    let default = default_source_name().unwrap_or_default();
    match pick_automatic(&json, &default) {
        Some(pick) => Ok(Some(pick)),
        // The automatic pick found nothing usable — falling through to cpal
        // would open the ALSA "default" PCM, which on PipeWire systems is the
        // speaker monitor: silence or loopback, and no obvious error.
        None => Err(no_microphone_message(&json, &default)),
    }
}

/// The message a failed pick should produce. An unplugged headset jack is
/// the common "there IS a mic port but nothing is in it" case; a speaker
/// monitor as the only source means there is no capture endpoint at all.
fn no_microphone_message(json: &str, default_source: &str) -> String {
    let Ok(sources) = serde_json::from_str::<Vec<PactlSource>>(json) else {
        return NO_MICROPHONE.to_string();
    };
    let non_monitor: Vec<&PactlSource> = sources
        .iter()
        .filter(|s| !s.name.ends_with(".monitor"))
        .collect();
    if non_monitor.iter().any(|s| !s.is_available()) {
        return concat!(
            "Only a headset jack was found, and nothing is plugged into it. ",
            "Plug in a headset, or if this computer has a built-in microphone, ",
            "its Linux audio driver may not be loaded."
        )
        .into();
    }
    if non_monitor.is_empty()
        && sources
            .iter()
            .any(|s| s.name == default_source && s.name.ends_with(".monitor"))
    {
        return concat!(
            "No microphone is available — the only source is a speaker monitor, ",
            "which captures playback, not your voice. Connect a microphone or ",
            "choose another input in Audio settings."
        )
        .into();
    }
    NO_MICROPHONE.to_string()
}

pub(crate) fn pick_system_microphone(json: &str, default_source: &str) -> Option<(String, String)> {
    let sources: Vec<PactlSource> = serde_json::from_str(json).ok()?;
    if let Some(source) = sources.iter().find(|source| {
        source.name == default_source && !source.name.ends_with(".monitor") && source.is_available()
    }) {
        return Some((
            source.name.clone(),
            if source.description.is_empty() {
                source.name.clone()
            } else {
                source.description.clone()
            },
        ));
    }
    // A playback monitor is not a system-default microphone. This occurs on
    // KDE with Pro Audio profiles; CPAL's generic ALSA "default" then captures
    // speaker output or silence. Explicitly selected monitors remain supported.
    pick_automatic(json, default_source)
}

pub fn resolve_system_microphone() -> Result<Option<(String, String)>, String> {
    if !pipewire_server_present() {
        return Ok(None);
    }
    let Some(json) = run_pactl(&["-f", "json", "list", "sources"]) else {
        return Ok(None); // Preserve ALSA-only systems without a Pulse/PipeWire service.
    };
    let default = default_source_name().unwrap_or_default();
    let selected = pick_system_microphone(&json, &default)
        .ok_or_else(|| no_microphone_message(&json, &default))?;
    if selected.0 != default {
        log::warn!("System default input is not a microphone; using '{}' for this session. Choose an explicit input in Audio settings to override it.", selected.1);
    }
    Ok(Some(selected))
}

/// Resolve a selected device id to a PipeWire node name and its description.
/// `Ok(None)` when the id does not select a PipeWire source at all (the
/// caller then falls back to cpal's ALSA host).
pub fn resolve_selected(id: &str) -> Result<Option<(String, String)>, String> {
    if id == AUTO_ID {
        if let Some((node, description)) = resolve_automatic_or_error()? {
            log::info!("Automatic microphone: '{description}' ({node})");
            return Ok(Some((node, description)));
        }
        return Ok(None);
    }
    let Some(node) = id.strip_prefix(ID_PREFIX) else {
        return Ok(None);
    };
    if let Some(description) = source_description(node) {
        return Ok(Some((node.to_string(), description)));
    }
    // The saved PipeWire node is gone (device unplugged or renamed). Resolve
    // to whatever automatic would pick for this session rather than failing —
    // but if nothing usable exists, that is an error, not an excuse to let
    // cpal capture the speaker monitor.
    log::warn!(
        "Selected microphone '{id}' is unavailable; using the automatic input for this session."
    );
    resolve_automatic_or_error()
}

pub fn default_source_description() -> Option<String> {
    list_sources()?
        .into_iter()
        .find(|device| device.is_default)
        .map(|device| device.name)
}

/// Whether a node exists and can capture right now (not an unplugged jack).
fn node_capturable(node: &str) -> Option<bool> {
    let json = run_pactl(&["-f", "json", "list", "sources"])?;
    let sources: Vec<PactlSource> = serde_json::from_str(&json).ok()?;
    sources
        .iter()
        .find(|s| s.name == node)
        .map(PactlSource::is_available)
}

pub fn source_description(node: &str) -> Option<String> {
    let id = format!("{ID_PREFIX}{node}");
    list_sources()?
        .into_iter()
        .find(|device| device.id == id)
        .map(|device| device.name)
}

pub(crate) fn record_args(node: &str) -> Vec<String> {
    let (target, properties) = match node.strip_suffix(".monitor") {
        Some(sink) => (
            sink,
            format!("{{ stream.capture.sink=true node.dont-fallback=true application.name={APP_NAME} }}"),
        ),
        // Bluetooth inputs are unavailable until WirePlumber switches the card
        // to headset mode, so a strict target would never link.
        None if node.starts_with("bluez_input.") => (
            node,
            format!("{{ application.name={APP_NAME} }}"),
        ),
        None => (
            node,
            format!("{{ node.dont-fallback=true application.name={APP_NAME} }}"),
        ),
    };
    vec![
        "-P".into(),
        properties,
        "--target".into(),
        target.into(),
        "--rate".into(),
        RATE.to_string(),
        "--channels".into(),
        "1".into(),
        "--format".into(),
        "f32".into(),
        "--raw".into(),
        "-".into(),
    ]
}

pub(crate) fn decode_f32le(carry: &mut Vec<u8>, bytes: &[u8]) -> Vec<f32> {
    carry.extend_from_slice(bytes);
    let whole = carry.len() / 4 * 4;
    let samples = (0..whole)
        .step_by(4)
        .map(|i| f32::from_le_bytes([carry[i], carry[i + 1], carry[i + 2], carry[i + 3]]))
        .collect();
    carry.drain(..whole);
    samples
}

pub struct PipeWireCapture {
    child: Child,
    reader: Option<JoinHandle<()>>,
    stopped: Arc<AtomicBool>,
}

impl PipeWireCapture {
    pub fn spawn(
        node: &str,
        mut on_samples: impl FnMut(&[f32]) + Send + 'static,
        on_error: impl FnOnce(String) + Send + 'static,
    ) -> Result<Self, String> {
        // Unknown or unplugged sources are refused here rather than
        // streamed as silence.
        if node_capturable(node) != Some(true) {
            return Err(UNAVAILABLE.into());
        }
        let mut command = Command::new("pw-record");
        command
            .args(record_args(node))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::platform::process::own_child(&mut command);
        let mut child = command.spawn().map_err(|error| match error.kind() {
            ErrorKind::NotFound => MISSING_TOOL.to_string(),
            _ => format!("Cannot start PipeWire recording: {error}"),
        })?;
        let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Cannot read PipeWire recording output.".into());
        };

        // pw-record exits at once when the server is not actually PipeWire or
        // refuses the connection — report that instead of pretending a silent
        // capture is running.
        std::thread::sleep(Duration::from_millis(300));
        if let Ok(Some(_status)) = child.try_wait() {
            let detail = BufReader::new(stderr)
                .lines()
                .map_while(Result::ok)
                .filter(|line| !line.trim().is_empty())
                .last()
                .unwrap_or_else(|| "pw-record exited immediately".to_string());
            let _ = child.wait();
            return Err(format!("Cannot start PipeWire recording: {detail}"));
        }

        let last_line = Arc::new(Mutex::new(String::new()));
        let stderr_line = Arc::clone(&last_line);
        let stderr_reader = thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let line = line.trim();
                if !line.is_empty() {
                    *stderr_line.lock() = line.to_string();
                }
            }
        });

        let stopped = Arc::new(AtomicBool::new(false));
        let reader_stopped = Arc::clone(&stopped);
        let reader = thread::spawn(move || {
            let mut stdout = stdout;
            let mut carry = Vec::new();
            let mut buffer = vec![0u8; CHUNK_BYTES];
            loop {
                match stdout.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        let samples = decode_f32le(&mut carry, &buffer[..read]);
                        if !samples.is_empty() {
                            on_samples(&samples);
                        }
                    }
                    Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            let _ = stderr_reader.join();
            if !reader_stopped.load(Ordering::SeqCst) {
                let detail = last_line.lock().clone();
                let detail = if detail.is_empty() {
                    "PipeWire recording ended".to_string()
                } else {
                    detail
                };
                on_error(format!(
                    "Microphone stopped: {detail}. Reconnect it or select another input."
                ));
            }
        });

        Ok(Self {
            child,
            reader: Some(reader),
            stopped,
        })
    }

    pub fn stop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

impl Drop for PipeWireCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_headset_jack_is_not_a_builtin_microphone() {
        let json = r#"[
            {"name":"alsa_input.analog-stereo","description":"Headset jack",
             "active_port":"analog-input-mic",
             "ports":[{"name":"analog-input-mic","availability":"not available"}],
             "properties":{"device.bus":"pci"}},
            {"name":"speaker.monitor"}
        ]"#;
        assert_eq!(pick_automatic(json, "alsa_input.analog-stereo"), None);
        assert_eq!(
            pick_system_microphone(json, "alsa_input.analog-stereo"),
            None
        );
        // The unplugged jack stays in the list, flagged unavailable — the
        // pickers show it disabled so the hardware is visible but unrecordable.
        let devices = parse_sources(json, "alsa_input.analog-stereo").unwrap();
        assert_eq!(devices.len(), 2);
        let jack = devices
            .iter()
            .find(|d| d.id == "pipewire:alsa_input.analog-stereo")
            .unwrap();
        assert!(!jack.is_available);
        let monitor = devices.iter().find(|d| d.is_monitor).unwrap();
        assert!(monitor.is_available);

        // The diagnostic message names the real problem — an empty jack —
        // instead of a generic "no microphone".
        let message = no_microphone_message(json, "speaker.monitor");
        assert!(
            message.contains("headset jack"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn monitor_only_default_reports_no_microphone_endpoint() {
        let json = r#"[{"name":"alsa_output.analog-stereo.monitor"}]"#;
        let message = no_microphone_message(json, "alsa_output.analog-stereo.monitor");
        assert!(message.contains("speaker monitor"), "got: {message}");
    }

    #[test]
    fn pipewire_server_detection_distinguishes_pulseaudio() {
        assert!(is_pipewire_server(
            "Server String: /run/user/1000/pulse/native
Server Name: PulseAudio (on PipeWire 1.4.8)
"
        ));
        assert!(!is_pipewire_server(
            "Server String: /run/user/1000/pulse/native
Server Name: PulseAudio
"
        ));
        assert!(!is_pipewire_server(""));
        // The JSON `server_name` field is locale-independent — the preferred
        // probe path.
        assert_eq!(
            server_name_json_is_pipewire(r#"{"server_name":"PulseAudio (on PipeWire 1.6.2)"}"#),
            Some(true)
        );
        assert_eq!(
            server_name_json_is_pipewire(r#"{"server_name":"PulseAudio"}"#),
            Some(false)
        );
        assert_eq!(server_name_json_is_pipewire("not json"), None);
    }

    #[test]
    fn failed_server_probe_retries_after_the_rate_limit() {
        static CACHE: Mutex<Option<(bool, std::time::Instant)>> = Mutex::new(None);
        let calls = std::cell::Cell::new(0u32);
        let probe = || {
            calls.set(calls.get() + 1);
            false
        };
        assert!(!probe_rate_limited(&CACHE, probe));
        // Inside the 5s window the cached failure stands — no re-probe.
        assert!(!probe_rate_limited(&CACHE, probe));
        assert_eq!(calls.get(), 1);
        // Past the window a failure is re-probed — a daemon that starts after
        // Reflow becomes visible without a restart.
        *CACHE.lock() = Some((false, std::time::Instant::now() - Duration::from_secs(10)));
        assert!(!probe_rate_limited(&CACHE, probe));
        assert_eq!(calls.get(), 2);

        static CACHE_OK: Mutex<Option<(bool, std::time::Instant)>> = Mutex::new(None);
        assert!(probe_rate_limited(&CACHE_OK, || true));
        assert!(probe_rate_limited(&CACHE_OK, || {
            panic!("a detected PipeWire server is never re-probed")
        }));
    }

    #[test]
    fn default_source_unusable_only_when_server_has_no_mic() {
        let monitor_default = r#"[{"name":"speaker.monitor"}]"#;
        assert!(default_source_unusable(monitor_default, "speaker.monitor"));

        let jack_only = r#"[
            {"name":"speaker.monitor"},
            {"name":"alsa_input.jack","ports":[{"name":"m","availability":"not available"}]}
        ]"#;
        assert!(default_source_unusable(jack_only, "speaker.monitor"));
        assert!(default_source_unusable(jack_only, "alsa_input.jack"));

        // A working microphone behind a non-monitor default keeps `default`.
        let real_mic = r#"[
            {"name":"alsa_input.mic","ports":[{"name":"m","availability":"available"}]}
        ]"#;
        assert!(!default_source_unusable(real_mic, "alsa_input.mic"));
    }

    #[test]
    fn unavailable_active_port_does_not_hide_another_working_microphone() {
        let json = r#"[
            {"name":"headset-jack","active_port":"jack",
             "ports":[{"name":"jack","availability":"not available"},
                      {"name":"other","availability":"available"}],
             "properties":{"device.bus":"pci","priority.session":3000}},
            {"name":"internal-mic","description":"Internal microphone",
             "active_port":"internal",
             "ports":[{"name":"internal","availability":"availability unknown"}],
             "properties":{"device.bus":"pci","priority.session":1000}}
        ]"#;
        assert_eq!(
            pick_automatic(json, "headset-jack").unwrap().0,
            "internal-mic"
        );
        assert_eq!(
            pick_system_microphone(json, "headset-jack").unwrap().0,
            "internal-mic"
        );
    }

    #[test]
    fn connected_headset_jack_remains_available() {
        let json = r#"[{"name":"headset-jack","active_port":"jack",
            "ports":[{"name":"jack","availability":"available"}],
            "properties":{"device.bus":"pci"}}]"#;
        assert_eq!(
            pick_automatic(json, "headset-jack").unwrap().0,
            "headset-jack"
        );
        assert_eq!(parse_sources(json, "headset-jack").unwrap().len(), 1);
    }

    #[test]
    fn automatic_prefers_a_plugged_headset_jack_over_the_default_internal_mic() {
        let json = r#"[
            {"name":"headset-jack","active_port":"analog-input-headset-mic",
             "ports":[{"name":"analog-input-headset-mic","availability":"available"}],
             "properties":{"device.bus":"pci","priority.session":1000}},
            {"name":"internal-mic",
             "properties":{"device.form_factor":"internal","priority.session":2000}}
        ]"#;
        assert_eq!(
            pick_automatic(json, "internal-mic").unwrap().0,
            "headset-jack"
        );
    }

    #[test]
    fn bluetooth_sources_with_null_active_port_remain_selectable() {
        let json = r#"[{"name":"bluez_input.headset","active_port":null,"ports":[],
            "properties":{"device.bus":"bluetooth","device.form_factor":"headset"}}]"#;
        assert_eq!(pick_automatic(json, "").unwrap().0, "bluez_input.headset");
        assert_eq!(parse_sources(json, "").unwrap().len(), 1);
    }

    #[test]
    fn system_default_monitor_does_not_capture_speaker_audio_as_a_microphone() {
        let json = sources(&[
            source("speaker.monitor", "pci", "", "3000"),
            source("built-in", "pci", "internal", "1000"),
            source("usb-headset", "usb", "headset", "2000"),
        ]);
        assert_eq!(
            pick_system_microphone(&json, "speaker.monitor").unwrap().0,
            "usb-headset"
        );
        assert_eq!(
            pick_system_microphone(&json, "built-in").unwrap().0,
            "built-in"
        );
        assert_eq!(
            pick_system_microphone(
                &sources(&[source("speaker.monitor", "pci", "", "3000")]),
                "speaker.monitor"
            ),
            None
        );
    }

    const SOURCES: &str = r#"[
        {"index":40,"state":"SUSPENDED","name":"alsa_output.pci-0000_06_00.6.analog-stereo.monitor","description":"Monitor of Ryzen HD Audio Controller Analog Stereo","driver":"PipeWire","sample_specification":"s32le 2ch 48000Hz","monitor_source":"alsa_output.pci-0000_06_00.6.analog-stereo","properties":{"device.class":"monitor","media.class":"Audio/Source"}},
        {"index":41,"state":"SUSPENDED","name":"alsa_input.pci-0000_06_00.6.analog-stereo","description":"Ryzen HD Audio Controller Analog Stereo","driver":"PipeWire","sample_specification":"s32le 2ch 48000Hz","properties":{"device.class":"sound","device.bus":"pci","priority.session":"2009","media.class":"Audio/Source"}},
        {"index":42,"state":"SUSPENDED","name":"alsa_input.usb-Webcam-02.analog-stereo","description":"USB Webcam Analog Stereo","driver":"PipeWire","sample_specification":"s16le 2ch 48000Hz","properties":{"device.class":"sound","device.bus":"usb","device.form_factor":"webcam","priority.session":"2008","media.class":"Audio/Source"}},
        {"index":88,"state":"RUNNING","name":"bluez_input.84:AC:60:3D:83:64","description":"SA-TW P10","driver":"PipeWire","sample_specification":"float32le 1ch 48000Hz","properties":{"device.class":"sound","device.bus":"bluetooth","device.form_factor":"headset","priority.session":"2010","media.class":"Audio/Source"}},
        {"index":89,"state":"RUNNING","name":"bluez_output.84:AC:60:3D:83:64.monitor","description":"Monitor of SA-TW P10","driver":"PipeWire","sample_specification":"float32le 2ch 48000Hz","properties":{"device.class":"monitor","device.bus":"bluetooth","device.form_factor":"headset","media.class":"Audio/Source"}}
    ]"#;

    const BLUEZ: &str = "bluez_input.84:AC:60:3D:83:64";
    const ANALOG: &str = "alsa_input.pci-0000_06_00.6.analog-stereo";
    const WEBCAM: &str = "alsa_input.usb-Webcam-02.analog-stereo";

    fn source(name: &str, bus: &str, form: &str, priority: &str) -> String {
        format!(
            r#"{{"name":"{name}","description":"{name} desc","sample_specification":"s16le 2ch 48000Hz","properties":{{"device.bus":"{bus}","device.form_factor":"{form}","priority.session":"{priority}"}}}}"#
        )
    }

    fn sources(entries: &[String]) -> String {
        format!("[{}]", entries.join(","))
    }

    #[test]
    fn parses_sources_with_default_first_then_inputs_then_monitors() {
        let devices = parse_sources(SOURCES, BLUEZ).unwrap();
        let ids: Vec<&str> = devices.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "pipewire:bluez_input.84:AC:60:3D:83:64",
                "pipewire:alsa_input.pci-0000_06_00.6.analog-stereo",
                "pipewire:alsa_input.usb-Webcam-02.analog-stereo",
                "pipewire:alsa_output.pci-0000_06_00.6.analog-stereo.monitor",
                "pipewire:bluez_output.84:AC:60:3D:83:64.monitor",
            ]
        );
        assert_eq!(devices[0].name, "SA-TW P10");
        assert!(devices[0].is_default && !devices[0].is_monitor);
        assert_eq!(devices[1].name, "Ryzen HD Audio Controller Analog Stereo");
        assert!(!devices[1].is_default && !devices[1].is_monitor);
        assert!(devices[3].is_monitor && !devices[3].is_default);
        assert!(devices[4].is_monitor);
        assert_eq!((devices[0].sample_rate, devices[0].channels), (48000, 1));
        assert_eq!((devices[3].sample_rate, devices[3].channels), (48000, 2));
    }

    #[test]
    fn automatic_prefers_connected_headset_and_ignores_webcam() {
        assert_eq!(
            pick_automatic(SOURCES, ANALOG),
            Some((BLUEZ.to_string(), "SA-TW P10".to_string()))
        );
    }

    #[test]
    fn automatic_falls_back_to_builtin_when_headset_removed() {
        let json = sources(&[
            source(ANALOG, "pci", "", "2009"),
            source(WEBCAM, "usb", "webcam", "2008"),
        ]);
        assert_eq!(
            pick_automatic(&json, BLUEZ).map(|(node, _)| node),
            Some(ANALOG.to_string())
        );
    }

    #[test]
    fn automatic_picks_default_headset_then_highest_priority() {
        let json = sources(&[
            source(BLUEZ, "bluetooth", "headset", "2010"),
            source("alsa_input.usb-Headset", "usb", "headset", "2005"),
        ]);
        assert_eq!(
            pick_automatic(&json, "alsa_input.usb-Headset").map(|(node, _)| node),
            Some("alsa_input.usb-Headset".to_string())
        );
        assert_eq!(
            pick_automatic(&json, ANALOG).map(|(node, _)| node),
            Some(BLUEZ.to_string())
        );
    }

    #[test]
    fn automatic_accepts_numeric_priority_and_missing_priority() {
        let json = r#"[
            {"name":"a","description":"A","properties":{"device.bus":"usb","device.form_factor":"headset"}},
            {"name":"b","description":"B","properties":{"device.bus":"usb","device.form_factor":"headset","priority.session":3000}}
        ]"#;
        assert_eq!(
            pick_automatic(json, "").map(|(node, _)| node),
            Some("b".to_string())
        );
    }

    #[test]
    fn automatic_ignores_monitors_and_returns_none_without_inputs() {
        let json = sources(&[source(
            "alsa_output.pci.analog-stereo.monitor",
            "pci",
            "",
            "2009",
        )]);
        assert_eq!(pick_automatic(&json, ""), None);
    }

    #[test]
    fn automatic_uses_default_source_when_no_tier_matches() {
        let json = sources(&[source("odd_input", "", "", "0")]);
        assert_eq!(
            pick_automatic(&json, "odd_input"),
            Some(("odd_input".to_string(), "odd_input desc".to_string()))
        );
    }

    #[test]
    fn automatic_entry_is_listed_first() {
        let devices = with_automatic(parse_sources(SOURCES, BLUEZ).unwrap());
        assert_eq!(devices[0].id, AUTO_ID);
        assert!(!devices[0].is_default && !devices[0].is_monitor);
        assert_eq!(devices.len(), 6);
    }

    #[test]
    #[ignore = "requires PipeWire and an input device"]
    fn resolves_automatic_microphone_now() {
        let resolved = resolve_automatic();
        println!("automatic microphone: {resolved:?}");
        assert!(resolved.is_some());
    }

    #[test]
    fn malformed_sample_specification_yields_zero_format() {
        let json = r#"[{"name":"x","description":"","sample_specification":"garbage"}]"#;
        let devices = parse_sources(json, "").unwrap();
        assert_eq!(devices[0].name, "x");
        assert_eq!((devices[0].sample_rate, devices[0].channels), (0, 0));
    }

    #[test]
    fn rejects_invalid_source_json() {
        assert!(parse_sources("not json", "").is_err());
    }

    #[test]
    fn record_args_target_source_directly() {
        let args = record_args("alsa_input.pci-0000_06_00.6.analog-stereo");
        assert_eq!(
            args,
            [
                "-P",
                "{ node.dont-fallback=true application.name=Reflow }",
                "--target",
                "alsa_input.pci-0000_06_00.6.analog-stereo",
                "--rate",
                "16000",
                "--channels",
                "1",
                "--format",
                "f32",
                "--raw",
                "-",
            ]
        );
    }

    #[test]
    fn record_args_allow_bluetooth_fallback_for_headset_switch() {
        let args = record_args("bluez_input.84:AC:60:3D:83:64");
        assert_eq!(args[1], "{ application.name=Reflow }");
        assert_eq!(args[3], "bluez_input.84:AC:60:3D:83:64");
    }

    #[test]
    fn record_args_capture_monitor_as_sink() {
        let args = record_args("bluez_output.84:AC:60:3D:83:64.monitor");
        assert_eq!(
            args[1],
            "{ stream.capture.sink=true node.dont-fallback=true application.name=Reflow }"
        );
        assert_eq!(args[3], "bluez_output.84:AC:60:3D:83:64");
    }

    #[test]
    fn decodes_f32le_split_across_reads() {
        let mut carry = Vec::new();
        let bytes: Vec<u8> = [0.25f32, -0.5f32]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        assert!(decode_f32le(&mut carry, &bytes[..2]).is_empty());
        assert_eq!(decode_f32le(&mut carry, &bytes[2..5]), [0.25]);
        assert_eq!(carry, [0x00u8]);
        assert_eq!(decode_f32le(&mut carry, &bytes[5..]), [-0.5]);
        assert!(carry.is_empty());
    }

    #[test]
    #[ignore = "requires PipeWire and the analog input"]
    fn records_selected_source_for_one_second() {
        let samples = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&samples);
        let mut capture = PipeWireCapture::spawn(
            "alsa_input.pci-0000_06_00.6.analog-stereo",
            move |chunk| {
                counter.fetch_add(chunk.len(), Ordering::SeqCst);
            },
            |_| {},
        )
        .expect("analog input should be recordable");
        thread::sleep(Duration::from_secs(1));
        capture.stop();
        assert!(samples.load(Ordering::SeqCst) >= 12_000);
    }

    #[test]
    #[ignore = "requires PipeWire"]
    fn rejects_missing_node_before_recording() {
        let result = PipeWireCapture::spawn("reflow.missing.node", |_| {}, |_| {});
        assert!(result.is_err());
    }

    /// Runs against the host's real PipeWire/Pulse server — prints what the
    /// picker list and automatic resolution would produce on this machine.
    #[test]
    #[ignore]
    fn live_pactl_resolution_report() {
        eprintln!("server is pipewire: {}", pipewire_server_present());
        let devices = list_sources();
        eprintln!("devices: {devices:#?}");
        match resolve_selected(AUTO_ID) {
            Ok(Some((node, desc))) => eprintln!("automatic → {desc} ({node})"),
            Ok(None) => eprintln!("automatic → cpal fallback"),
            Err(e) => eprintln!("automatic error → {e}"),
        }
        match resolve_system_microphone() {
            Ok(Some((node, desc))) => eprintln!("system default → {desc} ({node})"),
            Ok(None) => eprintln!("system default → cpal fallback"),
            Err(e) => eprintln!("system default error → {e}"),
        }
    }
}
