use super::*;
use crate::file_jobs::TranscriptSegment;
#[derive(Clone, Debug)]
pub(super) struct Window {
    pub track: usize,
    pub start: u64,
    pub frames: usize,
}
fn asr_key(m: &Meeting, c: &Window) -> String {
    format!(
        "asr/v2/{}/{}/{}/{}/{}",
        store::digest(m.asr_model.as_deref().unwrap_or("unknown").as_bytes()),
        m.language,
        c.track,
        c.start,
        c.frames
    )
}
pub(super) fn read_window(
    s: &store::Store,
    id: &str,
    chunks: &[Chunk],
    w: &Window,
) -> Result<Vec<f32>, String> {
    let mut output = Vec::with_capacity(w.frames);
    for c in chunks.iter().filter(|c| {
        c.track == w.track
            && c.start < w.start + w.frames as u64
            && c.start + c.frames as u64 > w.start
    }) {
        let audio = s.audio(id, c)?;
        let start = w.start.saturating_sub(c.start) as usize;
        let end = (w.start + w.frames as u64)
            .saturating_sub(c.start)
            .min(c.frames as u64) as usize;
        output.extend_from_slice(&audio[start..end]);
    }
    if output.len() != w.frames {
        return Err("ASR window contains missing audio; input preserved".into());
    }
    Ok(output)
}
pub(super) fn windows(s: &store::Store, id: &str, chunks: &[Chunk]) -> Result<Vec<Window>, String> {
    let mut windows = Vec::new();
    for track in 0..2 {
        let mut runs: Vec<(u64, u64)> = Vec::new();
        for c in chunks.iter().filter(|c| c.track == track) {
            if let Some(last) = runs.last_mut().filter(|r| r.1 == c.start) {
                last.1 += c.frames as u64;
            } else {
                runs.push((c.start, c.start + c.frames as u64));
            }
        }
        for (mut start, end) in runs {
            while start < end {
                let mut w = Window {
                    track,
                    start,
                    frames: (end - start).min(30 * 16_000) as usize,
                };
                if end - start > 30 * 16_000 {
                    let audio = read_window(s, id, chunks, &w)?;
                    // Quietest 100ms boundary in the final five seconds. No
                    // text deduplication: repeated words are never discarded.
                    w.frames = (25 * 16_000..30 * 16_000)
                        .step_by(1600)
                        .min_by(|&a, &b| {
                            let energy = |i| audio[i..i + 1600].iter().map(|x| x * x).sum::<f32>();
                            energy(a).total_cmp(&energy(b))
                        })
                        .unwrap();
                }
                start += w.frames as u64;
                windows.push(w);
            }
        }
    }
    windows.sort_by_key(|w| (w.start, w.track));
    Ok(windows)
}
async fn recognize(ctx: &AppContext, audio: &[f32], language: &str) -> Result<String, String> {
    let session = ctx.asr_handle.next_session_id();
    let result=tokio::time::timeout(Duration::from_secs(120),async {
        ctx.asr_handle.start_stream(session,language,&crate::session::session_vocabulary(&ctx.settings_store.get())).await?;
        for (seq,part) in audio.chunks(16_000).enumerate(){if ctx.meetings.cancel.load(Ordering::Acquire){return Err("Transcription cancelled".into());}ctx.asr_handle.push_audio(session,seq as u32,part.to_vec()).await?;}
        let stopped=ctx.asr_handle.stop_stream(session);tokio::pin!(stopped);
        loop {tokio::select! {out=&mut stopped=>break out,_=tokio::time::sleep(Duration::from_millis(50))=>{if ctx.meetings.cancel.load(Ordering::Acquire){return Err("Transcription cancelled".into());}}}}
    }).await;
    match result {
        Ok(Ok(text)) => {
            if let Some(w) = ctx.asr_handle.take_last_warning() {
                Err(format!("Recognition incomplete: {w}"))
            } else {
                Ok(text)
            }
        }
        other => {
            // Cancellation signals the actor's owned inference worker out of
            // band, then waits for its acknowledgement before releasing ownership.
            ctx.asr_handle.cancel_stream(session).await?;
            match other {
                Ok(Err(e)) => Err(e),
                _ => Err("Recognition window timed out; checkpoint preserved".into()),
            }
        }
    }
}
pub fn segments(s: &store::Store, m: &Meeting) -> Result<Vec<TranscriptSegment>, String> {
    // Index survives audio-retention deletion; each segment is checkpointed.
    Ok(s.work(&m.id, "transcript")?
        .map(|v| serde_json::from_str(&v))
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default())
}
pub async fn start(ctx: AppContext, id: String, _manual: bool) -> Result<(), String> {
    let guard = ctx
        .session_operation
        .clone()
        .try_lock_owned()
        .map_err(|_| "Speech resources are busy. Retry processing after the current task.")?;
    if ctx.meetings.processing.swap(true, Ordering::AcqRel) {
        return Err("A meeting is already processing".into());
    }
    ctx.meetings.cancel.store(false, Ordering::Release);
    tauri::async_runtime::spawn(async move {
        let handle = ctx.asr_handle.clone();
        let restore = tokio::task::spawn_blocking(move || handle.loaded_model_request_blocking())
            .await
            .ok()
            .and_then(Result::ok)
            .flatten();
        let result = run(&ctx, &id).await;
        if let Err(e) = result {
            if let Ok(s) = store::Store::open(
                &ctx.meetings.path,
                ctx.settings_store.get().history_encryption,
            ) {
                if let Ok(mut m) = s.get(&id) {
                    if m.processing == "transcribing" {
                        m.transcription_status = if ctx.meetings.cancel.load(Ordering::Acquire) {
                            "cancelled"
                        } else {
                            "failed"
                        }
                        .into();
                    }
                    if m.processing == "summarizing" {
                        m.summary_status = if ctx.meetings.cancel.load(Ordering::Acquire) {
                            "cancelled"
                        } else {
                            "failed"
                        }
                        .into();
                    }
                    m.processing = if ctx.meetings.cancel.load(Ordering::Acquire) {
                        "cancelled"
                    } else {
                        "failed"
                    }
                    .into();
                    m.error = Some(e);
                    let _ = s.save(&m);
                }
            }
        }
        if ctx.settings_store.get().asr.keep_loaded && !ctx.asr_handle.is_model_loaded() {
            if let Some(request) = restore {
                if let Err(e) = ctx
                    .asr_handle
                    .load_model_with_precision(
                        &request.model_dir,
                        &request.backend,
                        &request.precision,
                    )
                    .await
                {
                    log::warn!("Could not restore speech model after meeting: {e}");
                }
            }
        }
        ctx.meetings.processing.store(false, Ordering::Release);
        drop(guard);
    });
    Ok(())
}
async fn run(ctx: &AppContext, id: &str) -> Result<(), String> {
    let s = store::Store::open(
        &ctx.meetings.path,
        ctx.settings_store.get().history_encryption,
    )?;
    let mut m = s.get(id)?;
    if matches!(m.capture.as_str(), "recording" | "saving") {
        return Err("Stop recording before processing".into());
    }
    m.error = None;
    let chunks = s.chunks(id)?;
    let settings = ctx.settings_store.get();
    if m.saved_frames.iter().sum::<u64>() == 0 {
        return Err("No durable audio is available in this meeting".into());
    }
    if !m.audio_removed && m.processed_frames < m.saved_frames.iter().sum::<u64>() {
        let status = ctx.asr_handle.refresh_status().await?;
        if !status.loaded || status.is_loading {
            m.transcription_status = "waiting_for_model".into();
            s.save(&m)?;
            return Err(
                "Audio saved. Load the selected speech model in Settings, then Retry processing."
                    .into(),
            );
        }
        let identity = serde_json::to_string(&(
            settings.asr.model.clone(),
            settings.asr.runtime.clone(),
            settings.asr.device.clone(),
            settings.asr.precision.clone(),
            crate::session::session_vocabulary(&settings),
        ))
        .map_err(|e| e.to_string())?;
        if m.asr_model.as_ref().is_some_and(|v| v != &identity) && m.processed_frames > 0 {
            return Err("The speech configuration changed. Restore the original speech settings to resume this transcript.".into());
        }
        m.asr_model = Some(identity);
        m.processing = "transcribing".into();
        m.transcription_status = "running".into();
        m.processed_frames = 0;
        s.save(&m)?;
        let mut transcript = segments(&s, &m)?;
        let ranges = windows(&s, id, &chunks)?;
        for c in &ranges {
            if ctx.meetings.cancel.load(Ordering::Acquire) {
                return Err("Processing cancelled. Audio and completed segments are saved.".into());
            }
            let key = asr_key(&m, c);
            let text = if let Some(text) = s.work(id, &key)? {
                text
            } else {
                let audio = read_window(&s, id, &chunks, c)?;
                let mut outcome = Err("Recognition did not finish".to_string());
                for attempt in 0..3 {
                    s.put(
                        id,
                        &format!("job/{key}"),
                        &format!("running attempt {}", attempt + 1),
                    )?;
                    outcome = recognize(ctx, &audio, &m.language).await;
                    if outcome.is_ok() || ctx.meetings.cancel.load(Ordering::Acquire) {
                        break;
                    }
                    s.put(
                        id,
                        &format!("job/{key}"),
                        &format!("failed attempt {}", attempt + 1),
                    )?;
                    tokio::time::sleep(Duration::from_millis(250 * (attempt + 1))).await;
                }
                let text = outcome?;
                s.put(id, &key, &text)?;
                s.put(id, &format!("job/{key}"), "completed")?;
                text
            };
            let label = if c.track == 0 {
                "[Microphone]"
            } else {
                "[System playback]"
            };
            transcript.retain(|s| !(s.offset_ms == c.start / 16 && s.text.starts_with(label)));
            transcript.push(TranscriptSegment {
                text: format!(
                    "[{}] {}",
                    if c.track == 0 {
                        "Microphone"
                    } else {
                        "System playback"
                    },
                    text
                ),
                offset_ms: c.start / 16,
                end_ms: (c.start + c.frames as u64) / 16,
                speakers: None,
            });
            transcript.sort_by_key(|s| (s.offset_ms, s.end_ms));
            s.put(
                id,
                "transcript",
                &serde_json::to_string(&transcript).map_err(|e| e.to_string())?,
            )?;
            m.processed_frames += c.frames as u64;
            s.save(&m)?;
        }
        m.processing = "transcribed".into();
        m.transcription_status = "completed".into();
        s.save(&m)?;
    }
    if let Some(model) = m.config.summary_model.clone() {
        if !crate::rewrite::server::flow_gguf_path(&model).is_file() {
            return Err("Selected summary model is no longer installed. Select an installed model and retry.".into());
        }
        // The session guard is held for the entire job. Other dictation/file jobs
        // cannot own ASR while it is unloaded for a single summary-model launch.
        ctx.asr_handle.unload_model().await?;
        if ctx.asr_handle.refresh_status().await?.loaded {
            return Err("Speech model did not unload; summary deferred to preserve memory".into());
        }
        m.processing = "summarizing".into();
        m.summary_status = "running".into();
        s.save(&m)?;
        let context = ctx.clone();
        let meeting = m.clone();
        tokio::task::spawn_blocking(move || {
            let settings = context.settings_store.get();
            context.flow_runtime.ensure(
                &model,
                &settings.refinement.device,
                if settings.refinement.gpu_layers < 0 {
                    None
                } else {
                    Some(settings.refinement.gpu_layers as u32)
                },
                settings.memory_policy.vram_reserve_mb,
                settings.refinement.context_size,
            )?;
            let mut client = context.flow_runtime.client.read().clone();
            client.timeout = Duration::from_secs(90);
            client.max_completion_tokens = 768;
            let s = store::Store::open(&context.meetings.path, meeting.encrypted)?;
            let result = summary::run(&s, &meeting, &client, &context.meetings.cancel);
            if !settings.refinement.keep_warm {
                context.flow_runtime.shutdown();
            }
            result
        })
        .await
        .map_err(|e| e.to_string())??;
    }
    m = s.get(id)?;
    m.processing = "ready".into();
    m.summary_status = if m.config.summary_model.is_some() {
        "completed"
    } else {
        "not_requested"
    }
    .into();
    m.error = None;
    s.save(&m)?;
    // Disabled retention still permits temporary durable input. Never purge
    // unfinished/failed work. Transcript/summary stay until explicit deletion.
    if settings.audio_retention == "disabled" {
        s.remove_audio(id)?;
        m.audio_removed = true;
        s.save(&m)?;
    }
    if !settings.asr.keep_loaded {
        let _ = ctx.asr_handle.unload_model().await;
    }
    Ok(())
}
