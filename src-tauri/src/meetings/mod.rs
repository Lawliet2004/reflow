//! Durable meeting sessions deliberately do not enter dictation's buffer/ASR path.
mod capture;
mod inhibit;
mod process;
pub mod store;
mod summary;
#[cfg(test)]
mod tests;
use crate::context::AppContext;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tauri::State;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingConfig {
    pub microphone: Option<String>,
    pub playback: Option<String>,
    pub limit_hours: u32,
    pub summary_model: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gap {
    pub track: usize,
    pub start_ms: u64,
    pub end_ms: u64,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meeting {
    pub id: String,
    pub created_at: String,
    pub config: MeetingConfig,
    pub encrypted: bool,
    pub capture: String,
    pub processing: String,
    #[serde(default)]
    pub transcription_status: String,
    #[serde(default)]
    pub summary_status: String,
    pub elapsed_ms: u64,
    pub saved_frames: [u64; 2],
    pub last_commit_ms: u64,
    #[serde(default)]
    pub queue_depth: usize,
    #[serde(default)]
    pub peak_queue_depth: usize,
    pub levels: [f32; 2],
    pub gaps: Vec<Gap>,
    pub warning: Option<String>,
    pub error: Option<String>,
    pub stop_reason: Option<String>,
    pub processed_frames: u64,
    pub summary_progress: usize,
    pub audio_removed: bool,
    pub asr_model: Option<String>,
    pub language: String,
}
#[derive(Clone)]
pub struct Chunk {
    pub track: usize,
    pub seq: u64,
    pub start: u64,
    pub frames: usize,
}
pub struct Active {
    pub stop: AtomicBool,
    pub finished: AtomicBool,
    pub overflow: AtomicBool,
    pub device_errors: [AtomicBool; 2],
    pub snapshot: RwLock<Option<Meeting>>,
    pub queued: AtomicUsize,
    pub peak: AtomicUsize,
    pub loss_start: [AtomicU64; 2],
    pub loss_end: [AtomicU64; 2],
}
impl Default for Active {
    fn default() -> Self {
        Self {
            stop: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            overflow: AtomicBool::new(false),
            device_errors: [AtomicBool::new(false), AtomicBool::new(false)],
            snapshot: RwLock::new(None),
            queued: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            loss_start: [AtomicU64::new(u64::MAX), AtomicU64::new(u64::MAX)],
            loss_end: [AtomicU64::new(0), AtomicU64::new(0)],
        }
    }
}
pub struct Meetings {
    pub path: PathBuf,
    active: RwLock<Option<Arc<Active>>>,
    pub processing: AtomicBool,
    pub cancel: AtomicBool,
    pub recovery_error: RwLock<Option<String>>,
}
impl Meetings {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            active: RwLock::new(None),
            processing: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            recovery_error: RwLock::new(None),
        }
    }
    pub fn recording(&self) -> bool {
        self.active
            .read()
            .as_ref()
            .is_some_and(|a| !a.finished.load(Ordering::Acquire))
    }
    pub fn busy(&self) -> bool {
        self.recording() || self.processing.load(Ordering::Acquire)
    }
    pub fn recover(&self, encrypt: bool) {
        if self.path.exists() {
            if let Err(e) = store::Store::open(&self.path, encrypt).and_then(|s| s.recover()) {
                *self.recovery_error.write() = Some(e);
            }
        }
    }
}
/// Called at startup and while the app remains in the tray. A busy session
/// defers cleanup to the next tick, never interrupts capture or processing.
pub fn apply_retention_if_idle(ctx: &AppContext) -> Result<(), String> {
    let Ok(_session) = ctx.session_operation.try_lock() else {
        return Ok(());
    };
    if ctx.meetings.busy() || !ctx.meetings.path.exists() {
        return Ok(());
    }
    let settings = ctx.settings_store.get();
    store::Store::open(&ctx.meetings.path, settings.history_encryption)?
        .retention(&settings.audio_retention, &settings.history_retention)
}

pub fn limit_reached(hours: u32, ms: u64) -> bool {
    ms >= hours.min(5) as u64 * 3_600_000
}
fn validate(c: &MeetingConfig) -> Result<(), String> {
    if ![1, 2, 5].contains(&c.limit_hours) {
        return Err("Choose a 1, 2 or 5 hour duration limit".into());
    }
    if let Some(model) = &c.summary_model {
        if !crate::rewrite::server::FLOW_MODELS
            .iter()
            .any(|m| m.id == model.as_str())
            || !crate::rewrite::server::flow_gguf_path(model).is_file()
        {
            return Err("Select an installed local summary model".into());
        }
    }
    Ok(())
}
#[tauri::command]
pub async fn start_durable_meeting(
    mut config: MeetingConfig,
    app: tauri::AppHandle,
    ctx: State<'_, AppContext>,
) -> Result<String, String> {
    if config.summary_model.as_deref() == Some("follow_writing") {
        config.summary_model = Some(ctx.settings_store.get().resolved_summary_model());
    }
    validate(&config)?;
    if !ctx.settings_store.get().meeting_mode {
        return Err("Enable meeting mode in Advanced settings first".into());
    }
    let guard = ctx
        .session_operation
        .clone()
        .try_lock_owned()
        .map_err(|_| "Finish the current session first")?;
    if ctx.meetings.busy()
        || !matches!(
            *ctx.state_enum.read(),
            crate::state::AppStateEnum::Ready | crate::state::AppStateEnum::Idle
        )
    {
        return Err("Finish the current session first".into());
    }
    let settings = ctx.settings_store.get();
    if settings.history_retention == "disabled" {
        return Err("Enable history before recording a meeting. Audio must be saved temporarily for processing.".into());
    }
    let m = Meeting {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        config,
        encrypted: settings.history_encryption,
        capture: "recording".into(),
        processing: "pending".into(),
        transcription_status: "pending".into(),
        summary_status: "pending".into(),
        elapsed_ms: 0,
        saved_frames: [0; 2],
        last_commit_ms: 0,
        queue_depth: 0,
        peak_queue_depth: 0,
        levels: [0.0; 2],
        gaps: vec![],
        warning: None,
        error: None,
        stop_reason: None,
        processed_frames: 0,
        summary_progress: 0,
        audio_removed: false,
        asr_model: None,
        language: settings.language.clone(),
    };
    let id = m.id.clone();
    let active = Arc::new(Active::default());
    *active.snapshot.write() = Some(m.clone());
    *ctx.meetings.active.write() = Some(active.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    let context = ctx.inner().clone();
    if let Err(e) = std::thread::Builder::new()
        .name("meeting-supervisor".into())
        .spawn(move || capture::run(context, m, active, tx, guard))
    {
        *ctx.meetings.active.write() = None;
        return Err(e.to_string());
    }
    tokio::task::spawn_blocking(move || rx.recv().map_err(|e| e.to_string())?)
        .await
        .map_err(|e| e.to_string())??;
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some("Reflow — Recording meeting (Stop in tray)"));
    }
    let meetings = ctx.meetings.clone();
    tauri::async_runtime::spawn(async move {
        while meetings.recording() {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_tooltip(Some("Reflow — Meeting saved"));
        }
    });
    Ok(id)
}
#[tauri::command]
pub fn list_meetings(ctx: State<'_, AppContext>) -> Result<Vec<Meeting>, String> {
    if let Some(e) = ctx.meetings.recovery_error.read().clone() {
        return Err(e);
    }
    let mut list = store::Store::open(
        &ctx.meetings.path,
        ctx.settings_store.get().history_encryption,
    )?
    .list()?;
    if let Some(a) = ctx
        .meetings
        .active
        .read()
        .as_ref()
        .filter(|a| !a.finished.load(Ordering::Acquire))
    {
        if let Some(m) = a.snapshot.read().as_ref() {
            if let Some(v) = list.iter_mut().find(|v| v.id == m.id) {
                *v = m.clone();
            } else {
                list.insert(0, m.clone());
            }
        }
    }
    Ok(list)
}
pub async fn stop(ctx: &AppContext) -> Result<(), String> {
    let active = ctx.meetings.active.read().clone();
    if let Some(a) = active {
        a.stop.store(true, Ordering::Release);
        while !a.finished.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }
    Ok(())
}
#[tauri::command]
pub async fn stop_meeting(id: Option<String>, ctx: State<'_, AppContext>) -> Result<(), String> {
    if let Some(id) = id {
        let active = ctx.meetings.active.read().clone();
        if active
            .as_ref()
            .and_then(|a| a.snapshot.read().as_ref().map(|m| m.id.clone()))
            .as_ref()
            != Some(&id)
        {
            return Ok(());
        }
    }
    stop(ctx.inner()).await
}
#[tauri::command]
pub async fn process_meeting(
    id: String,
    summary_model: Option<String>,
    ctx: State<'_, AppContext>,
) -> Result<(), String> {
    if ctx.meetings.busy() {
        return Err("Finish the current meeting operation first".into());
    }
    {
        let _guard = ctx
            .session_operation
            .try_lock()
            .map_err(|_| "Finish the current session first")?;
        let s = store::Store::open(
            &ctx.meetings.path,
            ctx.settings_store.get().history_encryption,
        )?;
        let mut m = s.get(&id)?;
        if m.config.summary_model != summary_model {
            s.put(
                &id,
                "summary/final",
                "Summary pending for the newly selected model.",
            )?;
        }
        m.summary_status = if summary_model.is_some() {
            "pending"
        } else {
            "not_requested"
        }
        .into();
        m.config.summary_model = summary_model;
        validate(&m.config)?;
        s.save(&m)?;
    }
    process::start(ctx.inner().clone(), id, true).await
}
#[tauri::command]
pub fn cancel_meeting_processing(ctx: State<'_, AppContext>) {
    ctx.meetings.cancel.store(true, Ordering::Release);
}
#[tauri::command]
pub async fn delete_meeting(id: String, ctx: State<'_, AppContext>) -> Result<(), String> {
    // Serialized with writers/jobs; never race deletion against an open input.
    let _guard = ctx
        .session_operation
        .clone()
        .try_lock_owned()
        .map_err(|_| "Stop recording or processing before deleting a meeting")?;
    store::Store::open(
        &ctx.meetings.path,
        ctx.settings_store.get().history_encryption,
    )?
    .delete(&id)
}
#[derive(Serialize)]
pub struct MeetingOutput {
    pub segments: Vec<crate::file_jobs::TranscriptSegment>,
    pub summary: Option<String>,
    pub details: Option<String>,
}
#[tauri::command]
pub fn get_meeting_output(id: String, ctx: State<'_, AppContext>) -> Result<MeetingOutput, String> {
    let s = store::Store::open(
        &ctx.meetings.path,
        ctx.settings_store.get().history_encryption,
    )?;
    let m = s.get(&id)?;
    let segments = process::segments(&s, &m)?;
    Ok(MeetingOutput {
        segments,
        summary: s.work(&id, "summary/final")?,
        details: s.work(&id, "summary/details")?,
    })
}
#[tauri::command]
pub fn export_meeting(
    id: String,
    format: String,
    ctx: State<'_, AppContext>,
) -> Result<String, String> {
    let output = get_meeting_output(id, ctx)?;
    match format.as_str() {
        "srt" | "vtt" => crate::file_jobs::subtitle_text(&output.segments, &format),
        "txt" => Ok(output
            .segments
            .iter()
            .map(|s| s.text.clone())
            .collect::<Vec<_>>()
            .join("\n")),
        "md" => Ok(format!(
            "{}\n\n{}",
            output.summary.unwrap_or_default(),
            output.details.unwrap_or_default()
        )),
        _ => Err("Choose txt, md, srt or vtt".into()),
    }
}
#[derive(Serialize)]
pub struct Sources {
    pub supported: bool,
    pub explanation: String,
    pub devices: Vec<crate::audio::AudioDeviceInfo>,
    pub playback: Vec<crate::audio::AudioDeviceInfo>,
}
#[tauri::command]
pub async fn get_meeting_sources() -> Result<Sources, String> {
    tokio::task::spawn_blocking(||{
        let devices=crate::audio::AudioCaptureEngine::list_input_devices()?;
        let mut playback=Vec::new();
        #[cfg(target_os="linux")]
        {playback.extend(devices.iter().filter(|d|d.is_monitor && d.id.starts_with("pipewire:")).cloned());}
        #[cfg(target_os="windows")]
        {use cpal::traits::{HostTrait,DeviceTrait};let host=cpal::default_host();for d in host.output_devices().map_err(|e|e.to_string())?{if let Ok(c)=d.default_output_config(){let name=d.name().map_err(|e|e.to_string())?;playback.push(crate::audio::AudioDeviceInfo{id:name.clone(),name,is_default:false,is_monitor:true,is_available:true,sample_rate:c.sample_rate().0,channels:c.channels()});}}}
        Ok(Sources{supported:!playback.is_empty(),explanation:if cfg!(target_os="macos"){ "System audio capture is not implemented on macOS. Import a recording instead." }else if playback.is_empty(){"No supported playback source found. On Linux, run PipeWire with pactl and pw-record installed."}else{"Devices stay pinned for this recording. Other applications playing through this source will also be captured."}.into(),devices:devices.into_iter().filter(|d|!d.is_monitor).collect(),playback})
    }).await.map_err(|e|e.to_string())?
}
#[tauri::command]
pub async fn probe_meeting_sources(
    config: MeetingConfig,
    ctx: State<'_, AppContext>,
) -> Result<[f32; 2], String> {
    let guard = ctx
        .session_operation
        .clone()
        .try_lock_owned()
        .map_err(|_| "Audio resources are busy")?;
    if ctx.meetings.busy() {
        return Err("Stop recording before testing sources".into());
    }
    tokio::task::spawn_blocking(move || {
        let _guard = guard;
        capture::probe(&config, Arc::new(Active::default()))
    })
    .await
    .map_err(|e| e.to_string())?
}
