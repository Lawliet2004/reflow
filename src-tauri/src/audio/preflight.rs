use super::{health::MicrophoneHealth, AudioCaptureEngine};
use crate::settings::AppSettings;

pub async fn capture_sample(
    settings: &AppSettings,
    seconds: u64,
) -> Result<(MicrophoneHealth, Vec<f32>), String> {
    let seconds = seconds.clamp(1, 10);
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let (stop_tx, _stop_rx) = tokio::sync::mpsc::channel(1);
    let mut capture = AudioCaptureEngine::new();
    capture.start_capture(
        settings.microphone_device_id.clone(),
        settings.input_gain,
        0.5,
        0,
        tx,
        stop_tx,
    )?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(seconds);
    let limit = seconds as usize * 16_000;
    let mut samples = Vec::with_capacity(limit);
    while let Ok(Some(chunk)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        let remaining = limit.saturating_sub(samples.len());
        samples.extend(chunk.into_iter().take(remaining));
        if samples.len() == limit {
            break;
        }
    }
    capture.stop_capture();
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
