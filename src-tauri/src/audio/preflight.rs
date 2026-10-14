use super::{health::MicrophoneHealth, AudioCaptureEngine};
use crate::settings::AppSettings;
use parking_lot::RwLock;
use std::sync::Arc;

/// `level_sink`, when given, is updated with the live capture level so the
/// `get_current_audio_level` command can drive a meter during the test.
pub async fn capture_sample(
    settings: &AppSettings,
    seconds: u64,
    level_sink: Option<Arc<RwLock<f32>>>,
) -> Result<(MicrophoneHealth, Vec<f32>), String> {
    let seconds = seconds.clamp(1, 10);
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let (stop_tx, _stop_rx) = tokio::sync::mpsc::channel(1);
    let device = settings.microphone_device_id.clone();
    let gain = settings.input_gain;
    let mut capture = tokio::task::spawn_blocking(move || {
        let mut capture = AudioCaptureEngine::new();
        capture.start_capture(device, gain, 0.5, 0, tx, stop_tx)?;
        Ok::<_, String>(capture)
    })
    .await
    .map_err(|error| error.to_string())??;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(seconds);
    let limit = seconds as usize * 16_000;
    let mut samples = Vec::with_capacity(limit);
    while let Ok(Some(chunk)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        if let Some(sink) = &level_sink {
            *sink.write() = capture.get_current_audio_level();
        }
        let remaining = limit.saturating_sub(samples.len());
        samples.extend(chunk.into_iter().take(remaining));
        if samples.len() == limit {
            break;
        }
    }
    capture.stop_capture();
    if let Some(sink) = &level_sink {
        *sink.write() = 0.0;
    }
    if let Some(error) = capture.stream_error() {
        return Err(error);
    }
    Ok((
        MicrophoneHealth::from_samples(
            capture.last_device_name(),
            &samples,
            capture.dropped_chunks(),
        ),
        samples,
    ))
}

#[derive(serde::Serialize)]
pub struct RecognitionTest {
    pub text: String,
    pub language: String,
    pub health: MicrophoneHealth,
}
