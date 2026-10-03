//! Local file imports share the speech engine with live dictation.
use crate::{context::AppContext, dory::DoryEvent, history::HistoryEntry, state::AppStateEnum};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub text: String,
    pub offset_ms: u64,
    pub end_ms: u64,
    #[serde(default)]
    pub speakers: Option<Vec<String>>,
}
#[derive(Debug, Clone, Serialize)]
pub struct FileProgress {
    pub job_id: String,
    pub done_s: f64,
    pub total_s: f64,
    pub history_id: Option<String>,
    pub finished: bool,
    pub error: Option<String>,
}
#[derive(Clone)]
pub struct FileJob {
    pub progress: FileProgress,
    pub cancel: Arc<AtomicBool>,
}

/// Select the quietest 100 ms window in the final five seconds of each chunk.
pub fn segment_audio(samples: &[f32]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < samples.len() {
        let maximum = (start + 30 * 16_000).min(samples.len());
        let end = if maximum < samples.len() {
            (maximum - 5 * 16_000..maximum)
                .step_by(1600)
                .min_by(|&a, &b| {
                    let energy = |i: usize| {
                        samples[i..(i + 1600).min(maximum)]
                            .iter()
                            .map(|s| s * s)
                            .sum::<f32>()
                    };
                    energy(a).total_cmp(&energy(b))
                })
                .unwrap_or(maximum)
        } else {
            maximum
        };
        ranges.push((start, end));
        start = end;
    }
    ranges
}

pub async fn transcribe_samples(
    ctx: &AppContext,
    samples: &[f32],
    language: &str,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f64),
) -> Result<Vec<TranscriptSegment>, String> {
    let vocabulary = crate::session::session_vocabulary(&ctx.settings_store.get());
    let mut segments = Vec::new();
    for (start, end) in segment_audio(samples) {
        if cancel.load(Ordering::SeqCst) {
            return Err("File transcription cancelled".into());
        }
        let session = ctx.asr_handle.next_session_id();
        ctx.asr_handle
            .start_stream(session, language, &vocabulary)
            .await?;
        let result = async {
            for (sequence, chunk) in samples[start..end].chunks(16_000).enumerate() {
                if cancel.load(Ordering::SeqCst) {
                    return Err("File transcription cancelled".into());
                }
                ctx.asr_handle
                    .push_audio(session, sequence as u32, chunk.to_vec())
                    .await?;
            }
            let text = ctx.asr_handle.stop_stream(session).await?;
            if cancel.load(Ordering::SeqCst) {
                return Err("File transcription cancelled".into());
            }
            if let Some(warning) = ctx.asr_handle.take_last_warning() {
                return Err(format!("File transcription incomplete: {warning}"));
            }
            Ok(text)
        }
        .await;
        match result {
            Ok(text) => segments.push(TranscriptSegment {
                text,
                offset_ms: start as u64 / 16,
                end_ms: end as u64 / 16,
                speakers: None,
            }),
            Err(error) => {
                let _ = ctx.asr_handle.cancel_stream(session).await;
                return Err(error);
            }
        }
        progress(end as f64 / 16_000.0);
    }
    Ok(segments)
}

fn publish(ctx: &AppContext, id: &str, update: impl FnOnce(&mut FileProgress)) {
    let progress = {
        let mut jobs = ctx.file_jobs.write();
        let job = jobs.get_mut(id).expect("registered job");
        update(&mut job.progress);
        job.progress.clone()
    };
    ctx.bus.emit(DoryEvent::FileProgress(progress));
}

fn ensure_file_history_enabled(ctx: &AppContext) -> Result<(), String> {
    if ctx.settings_store.get().history_retention == "disabled" {
        return Err("Enable history in Privacy & diagnostics before importing audio. Imported transcripts need saved history.".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn transcribe_file(
    path: String,
    allow_large: Option<bool>,
    ctx: State<'_, AppContext>,
) -> Result<serde_json::Value, String> {
    ensure_file_history_enabled(ctx.inner())?;
    let guard = ctx
        .session_operation
        .clone()
        .try_lock_owned()
        .map_err(|_| "Finish the current session first")?;
    if !matches!(
        *ctx.state_enum.read(),
        AppStateEnum::Ready | AppStateEnum::Idle
    ) {
        return Err("Finish the current session first".into());
    }
    let status = ctx.asr_handle.refresh_status().await?;
    if !status.loaded || status.is_loading {
        return Err("Load a speech model before importing audio".into());
    }
    let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if metadata.len() > crate::audio::file_decode::CONFIRM_FILE_BYTES
        && !allow_large.unwrap_or(false)
    {
        return Err("Confirm files larger than 100 MB before importing".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut jobs = ctx.file_jobs.write();
        if jobs.len() >= 16 {
            jobs.retain(|_, job| !job.progress.finished);
        }
        jobs.insert(
            id.clone(),
            FileJob {
                cancel: cancel.clone(),
                progress: FileProgress {
                    job_id: id.clone(),
                    done_s: 0.0,
                    total_s: 0.0,
                    history_id: None,
                    finished: false,
                    error: None,
                },
            },
        );
    }
    *ctx.state_enum.write() = AppStateEnum::Processing;
    ctx.bus.emit(DoryEvent::State(AppStateEnum::Processing));
    let context = ctx.inner().clone();
    let job_id = id.clone();
    tauri::async_runtime::spawn(async move {
        let _guard = guard;
        let result = import(
            &context,
            &job_id,
            path,
            allow_large.unwrap_or(false),
            cancel,
        )
        .await;
        publish(&context, &job_id, |p| {
            p.finished = true;
            match result {
                Ok(id) => {
                    p.history_id = Some(id);
                    p.done_s = p.total_s;
                }
                Err(error) => p.error = Some(error),
            }
        });
        *context.state_enum.write() = AppStateEnum::Ready;
        context.bus.emit(DoryEvent::State(AppStateEnum::Ready));
    });
    Ok(serde_json::json!({"job_id": id}))
}

async fn import(
    ctx: &AppContext,
    id: &str,
    path: String,
    allow_large: bool,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    ensure_file_history_enabled(ctx)?;
    let filename = Path::new(&path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let samples = tokio::task::spawn_blocking(move || {
        crate::audio::file_decode::decode_file(Path::new(&path), allow_large)
    })
    .await
    .map_err(|e| e.to_string())??;
    publish(ctx, id, |p| p.total_s = samples.len() as f64 / 16_000.0);
    let settings = ctx.settings_store.get();
    let language = if settings.auto_detect_language {
        "auto"
    } else {
        &settings.language
    };
    let segments = transcribe_samples(ctx, &samples, language, &cancel, |done| {
        publish(ctx, id, |p| p.done_s = done)
    })
    .await?;
    if cancel.load(Ordering::SeqCst) {
        return Err("File transcription cancelled".into());
    }
    let text = segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let history_id = uuid::Uuid::new_v4().to_string();
    let entry = HistoryEntry {
        id: history_id.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        duration_ms: samples.len() as u64 / 16,
        language: ctx.asr_handle.get_detected_language(),
        raw_transcript: text.clone(),
        smart_transcript: text.clone(),
        final_transcript: text.clone(),
        rewriter_used: false,
        application_name: filename.clone(),
        application_process: format!("file:{filename}"),
        word_count: text.split_whitespace().count(),
        character_count: text.chars().count(),
        model_version: settings.asr.model,
        processing_mode: "file".into(),
        audio_available: false,
        audio_expires_at: None,
        command_input: None,
        kind: "file".into(),
        source: "file".into(),
        pinned: false,
        tags: String::new(),
    };
    // Serialize this final policy check and every write with settings changes.
    // The policy may have changed during decoding or speech recognition.
    let _settings_operation = ctx.settings_operation.lock();
    ensure_file_history_enabled(ctx)?;
    if cancel.load(Ordering::SeqCst) {
        return Err("File transcription cancelled".into());
    }
    ctx.history_store.insert_entry(&entry)?;
    ctx.history_store
        .save_file_segments(&history_id, &segments)?;
    ctx.history_store.save_audio(&history_id, &samples)?;
    Ok(history_id)
}

#[tauri::command]
pub fn cancel_file_transcription(job_id: String, ctx: State<'_, AppContext>) -> Result<(), String> {
    let jobs = ctx.file_jobs.read();
    let job = jobs.get(&job_id).ok_or("Unknown file job")?;
    if !job.progress.finished {
        job.cancel.store(true, Ordering::SeqCst);
    }
    Ok(())
}
#[tauri::command]
pub fn get_file_job(job_id: String, ctx: State<'_, AppContext>) -> Result<FileProgress, String> {
    ctx.file_jobs
        .read()
        .get(&job_id)
        .map(|job| job.progress.clone())
        .ok_or("Unknown file job".into())
}

pub fn subtitle_text(segments: &[TranscriptSegment], format: &str) -> Result<String, String> {
    if !matches!(format, "txt" | "srt" | "vtt") {
        return Err("Choose txt, srt or vtt".into());
    }
    let mut result = if format == "vtt" {
        "WEBVTT\n\n".into()
    } else {
        String::new()
    };
    let stamp = |ms: u64| {
        format!(
            "{:02}:{:02}:{:02}{}{:03}",
            ms / 3_600_000,
            ms / 60_000 % 60,
            ms / 1000 % 60,
            if format == "srt" { ',' } else { '.' },
            ms % 1000
        )
    };
    for (i, segment) in segments.iter().enumerate() {
        if format == "txt" {
            result.push_str(&format!("{}\n", segment.text));
        } else {
            result.push_str(&format!(
                "{}\n{} --> {}\n{}\n\n",
                i + 1,
                stamp(segment.offset_ms),
                stamp(segment.end_ms),
                segment.text.replace("-->", "→")
            ));
        }
    }
    Ok(result)
}

fn file_export_text(
    final_text: &str,
    segments: &[TranscriptSegment],
    format: &str,
) -> Result<String, String> {
    if !matches!(format, "txt" | "srt" | "vtt") {
        return Err("Choose txt, srt or vtt".into());
    }
    if format == "txt" {
        return Ok(final_text.to_owned());
    }
    if segments.is_empty() {
        return Err("This transcript has no current subtitle timing data. Import the audio again to regenerate subtitles.".into());
    }
    subtitle_text(segments, format)
}
#[tauri::command]
pub async fn export_file_transcript(
    id: String,
    format: String,
    ctx: State<'_, AppContext>,
) -> Result<String, String> {
    let store = ctx.history_store.clone();
    tokio::task::spawn_blocking(move || {
        let entry = store.get_entry(&id)?.ok_or("Transcript no longer exists")?;
        let segments = if format == "txt" {
            Vec::new()
        } else {
            store.file_segments(&id)?
        };
        let text = file_export_text(&entry.final_transcript, &segments, &format)?;
        let directory =
            dirs::download_dir().unwrap_or_else(crate::platform::PlatformSys::get_app_dir);
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let output = directory.join(format!("Reflow-{}.{format}", uuid::Uuid::new_v4()));
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&output)
            .map_err(|e| e.to_string())?;
        file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        Ok(output.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn select_audio_file() -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(pick_audio)
        .await
        .map_err(|e| e.to_string())?
}
#[cfg(windows)]
fn pick_audio() -> Result<Option<String>, String> {
    use windows::{
        core::PCWSTR,
        Win32::UI::Controls::Dialogs::{
            CommDlgExtendedError, GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST,
            OFN_NOCHANGEDIR, OPENFILENAMEW,
        },
    };
    let mut path = vec![0u16; 32768];
    let filter: Vec<u16> = "Audio files\0*.wav;*.mp3;*.m4a;*.aac;*.flac;*.ogg\0All files\0*.*\0\0"
        .encode_utf16()
        .collect();
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFile: windows::core::PWSTR(path.as_mut_ptr()),
        nMaxFile: path.len() as u32,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        Flags: OFN_FILEMUSTEXIST | OFN_NOCHANGEDIR | OFN_EXPLORER,
        ..Default::default()
    };
    unsafe {
        if GetOpenFileNameW(&mut dialog).as_bool() {
            let length = path.iter().position(|&c| c == 0).unwrap_or(path.len());
            Ok(Some(String::from_utf16_lossy(&path[..length])))
        } else {
            let error = CommDlgExtendedError();
            if error.0 == 0 {
                Ok(None)
            } else {
                Err(format!("File picker failed: {error:?}"))
            }
        }
    }
}
#[cfg(not(windows))]
fn pick_audio() -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    let output = std::process::Command::new("osascript")
        .args([
            "-e",
            "POSIX path of (choose file with prompt \"Choose audio\")",
        ])
        .output();
    #[cfg(not(target_os = "macos"))]
    let output = std::process::Command::new("zenity")
        .args([
            "--file-selection",
            "--title=Choose audio",
            "--file-filter=Audio | *.wav *.mp3 *.m4a *.aac *.flac *.ogg",
        ])
        .output();
    let output = output.map_err(|_| "File picker unavailable; enter an audio path instead")?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&output.stdout).trim().into()))
}

#[cfg(test)]
mod storage_audit_tests {
    use super::*;
    use crate::asr::{ASREngine, MockASREngine};
    use crate::settings::SettingsStore;

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("reflow-file-privacy-{}", uuid::Uuid::new_v4())))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn storage_audit_invalidated_subtitles_keep_txt_export_available() {
        assert_eq!(
            file_export_text("Edited transcript", &[], "txt").unwrap(),
            "Edited transcript"
        );
        for format in ["srt", "vtt"] {
            assert!(file_export_text("Edited transcript", &[], format)
                .unwrap_err()
                .contains("regenerate subtitles"));
        }
    }

    #[tokio::test]
    async fn storage_audit_disabled_history_refuses_import_before_opening_the_file() {
        let fixture = Fixture::new();
        let ctx = AppContext::bootstrap_test(fixture.0.clone());
        ctx.settings_store
            .merge_update(serde_json::json!({"history_retention": "disabled"}))
            .unwrap();
        let error = import(
            &ctx,
            "job",
            fixture.0.join("missing.wav").display().to_string(),
            false,
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap_err();
        assert!(error.contains("Enable history"), "{error}");
        assert!(ctx.history_store.get_entries(10, 0).unwrap().is_empty());
    }

    struct DisableHistoryOnStop {
        engine: MockASREngine,
        settings: Arc<SettingsStore>,
    }
    impl ASREngine for DisableHistoryOnStop {
        fn initialize(&mut self) -> Result<(), String> {
            self.engine.initialize()
        }
        fn load_model_with_precision(
            &mut self,
            path: &str,
            backend: &str,
            precision: &str,
        ) -> Result<(), String> {
            self.engine
                .load_model_with_precision(path, backend, precision)
        }
        fn unload_model(&mut self) -> Result<(), String> {
            self.engine.unload_model()
        }
        fn is_model_loaded(&self) -> bool {
            self.engine.is_model_loaded()
        }
        fn start_stream(&mut self, language: &str, vocabulary: &[String]) -> Result<(), String> {
            self.engine.start_stream(language, vocabulary)
        }
        fn push_audio(&mut self, samples: &[f32]) -> Result<Option<String>, String> {
            self.engine.push_audio(samples)
        }
        fn get_partial_transcript(&mut self) -> Result<String, String> {
            self.engine.get_partial_transcript()
        }
        fn stop_stream(&mut self) -> Result<String, String> {
            self.settings
                .merge_update(serde_json::json!({"history_retention": "disabled"}))?;
            self.engine.stop_stream()
        }
        fn cancel_stream(&mut self) -> Result<(), String> {
            self.engine.cancel_stream()
        }
        fn get_detected_language(&self) -> String {
            self.engine.get_detected_language()
        }
        fn get_backend_name(&self) -> String {
            self.engine.get_backend_name()
        }
    }

    #[tokio::test]
    async fn storage_audit_live_history_disable_prevents_transcript_subtitle_and_audio_writes() {
        let fixture = Fixture::new();
        let ctx = AppContext::bootstrap_test(fixture.0.clone());
        ctx.history_store.set_audio_retention("7_days").unwrap();
        ctx.asr_handle
            .swap_engine(Box::new(DisableHistoryOnStop {
                engine: MockASREngine::new(),
                settings: ctx.settings_store.clone(),
            }))
            .await
            .unwrap();
        let path = fixture.0.join("input.wav");
        let mut writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..1600 {
            writer.write_sample(1000i16).unwrap();
        }
        writer.finalize().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        ctx.file_jobs.write().insert(
            "job".into(),
            FileJob {
                cancel: cancel.clone(),
                progress: FileProgress {
                    job_id: "job".into(),
                    done_s: 0.0,
                    total_s: 0.0,
                    history_id: None,
                    finished: false,
                    error: None,
                },
            },
        );
        let error = import(&ctx, "job", path.display().to_string(), false, cancel)
            .await
            .unwrap_err();
        assert!(error.contains("Enable history"), "{error}");
        let conn = rusqlite::Connection::open(fixture.0.join("history.db")).unwrap();
        for table in ["history", "file_segments", "history_audio"] {
            let count: usize = conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "private writes to {table}");
        }
    }
}
