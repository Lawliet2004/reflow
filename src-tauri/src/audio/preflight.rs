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
    let (health, samples, _) = capture_sample_inner(settings, seconds, level_sink, None).await?;
    Ok((health, samples))
}

pub async fn capture_sample_with_admission(
    settings: &AppSettings,
    seconds: u64,
    level_sink: Option<Arc<RwLock<f32>>>,
    admission: tokio::sync::OwnedMutexGuard<()>,
) -> Result<(MicrophoneHealth, Vec<f32>, tokio::sync::OwnedMutexGuard<()>), String> {
    let (health, samples, admission) =
        capture_sample_inner(settings, seconds, level_sink, Some(admission)).await?;
    Ok((
        health,
        samples,
        admission.expect("owned admission is retained"),
    ))
}

async fn bounded_start<T: Send + 'static>(
    start: impl FnOnce() -> Result<T, String> + Send + 'static,
    admission: Option<tokio::sync::OwnedMutexGuard<()>>,
    timeout: std::time::Duration,
) -> Result<(T, Option<tokio::sync::OwnedMutexGuard<()>>), String> {
    // A wedged driver cannot be killed safely in process. Keep admission with
    // its worker until it returns; retries cannot stack overlapping captures.
    tokio::time::timeout(timeout, tokio::task::spawn_blocking(move || {
        start().map(|capture| (capture, admission))
    })).await.map_err(|_| "The microphone did not open within 5 seconds. Reconnect it and retry; restart Reflow if the audio driver remains busy.".to_string())?
        .map_err(|error| error.to_string())?
}

async fn capture_sample_inner(
    settings: &AppSettings,
    seconds: u64,
    level_sink: Option<Arc<RwLock<f32>>>,
    admission: Option<tokio::sync::OwnedMutexGuard<()>>,
) -> Result<
    (
        MicrophoneHealth,
        Vec<f32>,
        Option<tokio::sync::OwnedMutexGuard<()>>,
    ),
    String,
> {
    let seconds = seconds.clamp(1, 10);
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let (stop_tx, _stop_rx) = tokio::sync::mpsc::channel(1);
    let device = settings.microphone_device_id.clone();
    let gain = settings.input_gain;
    let (mut capture, admission) = bounded_start(
        move || {
            let mut capture = AudioCaptureEngine::new();
            capture.start_capture(device, gain, 0.5, 0, tx, stop_tx)?;
            Ok::<_, String>(capture)
        },
        admission,
        std::time::Duration::from_secs(5),
    )
    .await?;
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
        admission,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn opening_timeout_retains_admission_until_the_driver_returns() {
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let admission = lock.clone().try_lock_owned().unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        let result = bounded_start(
            move || {
                wait.recv().unwrap();
                Ok(())
            },
            Some(admission),
            std::time::Duration::from_millis(30),
        )
        .await;
        assert!(result.unwrap_err().contains("microphone did not open"));
        assert!(
            lock.try_lock().is_err(),
            "timed-out worker still owns capture admission"
        );
        release.send(()).unwrap();
        let _admitted = tokio::time::timeout(std::time::Duration::from_secs(1), lock.lock())
            .await
            .unwrap();
    }
}

#[derive(serde::Serialize)]
pub struct RecognitionTest {
    pub text: String,
    pub language: String,
    pub health: MicrophoneHealth,
}
