//! Two native capture streams share a bounded 16 kHz wall-clock timeline.
//! Missing loopback callbacks represent silence, rather than delayed audio.
use super::{
    capture::AudioCaptureEngine, resampler::AudioResampler, VadConfig, VoiceActivityDetector,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use std::{
    collections::VecDeque,
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

const RATE: u64 = 16_000;
const BUFFER: usize = 16_000;

#[derive(Default)]
pub struct CaptureTimeline {
    next_frame: Option<u64>,
}
impl CaptureTimeline {
    /// Capture timestamps place buffers; callback time is only a first-buffer
    /// fallback. A sample cursor prevents rounding overlaps from losing audio.
    pub fn place(&mut self, capture_frame: Option<u64>, callback_frame: u64, frames: usize) -> u64 {
        let start = capture_frame
            .unwrap_or_else(|| {
                self.next_frame
                    .unwrap_or_else(|| callback_frame.saturating_sub(frames as u64))
            })
            .max(self.next_frame.unwrap_or(0));
        self.next_frame = Some(start.saturating_add(frames as u64));
        start
    }
}

#[derive(Default)]
pub struct MeetingMixer {
    mic: VecDeque<(u64, f32)>,
    system: VecDeque<(u64, f32)>,
    cursor: u64,
}
impl MeetingMixer {
    pub fn push_at(&mut self, system: bool, offset: u64, samples: &[f32]) {
        let queue = if system {
            &mut self.system
        } else {
            &mut self.mic
        };
        for (index, sample) in samples.iter().enumerate() {
            let frame = offset.saturating_add(index as u64);
            if frame < self.cursor || queue.back().is_some_and(|(last, _)| *last >= frame) {
                continue;
            }
            queue.push_back((
                frame,
                if sample.is_finite() {
                    sample.clamp(-1.0, 1.0)
                } else {
                    0.0
                },
            ));
            if queue.len() > BUFFER {
                queue.pop_front();
            }
        }
    }
    pub fn render(&mut self, frames: usize) -> Vec<f32> {
        let take = |queue: &mut VecDeque<(u64, f32)>, frame: u64| {
            while queue.front().is_some_and(|(at, _)| *at < frame) {
                queue.pop_front();
            }
            if queue.front().is_some_and(|(at, _)| *at == frame) {
                queue.pop_front().unwrap().1
            } else {
                0.0
            }
        };
        (0..frames)
            .map(|_| {
                let value =
                    (take(&mut self.mic, self.cursor) + take(&mut self.system, self.cursor)) / 2.0;
                self.cursor += 1;
                value
            })
            .collect()
    }
    pub fn buffered_samples(&self) -> usize {
        self.mic.len() + self.system.len()
    }
}

fn system_device(
    host: &cpal::Host,
    monitor: Option<&str>,
) -> Result<(cpal::Device, cpal::SupportedStreamConfig), String> {
    #[cfg(target_os = "windows")]
    {
        let _ = monitor;
        // CPAL 0.15.3 WASAPI device.rs sets LOOPBACK for eRender input streams.
        let device = host
            .default_output_device()
            .ok_or("No system playback device is available")?;
        let config = device.default_output_config().map_err(|e| e.to_string())?;
        Ok((device, config))
    }
    #[cfg(target_os = "linux")]
    {
        let mut devices = host
            .input_devices()
            .map_err(|e| e.to_string())?
            .filter(|d| {
                d.name().is_ok_and(|name| {
                    name.to_lowercase().contains("monitor") && monitor.is_none_or(|id| name == id)
                })
            });
        let device = devices.next().ok_or("No monitor capture source is exposed. Configure a PipeWire/PulseAudio monitor through ALSA, then select it in Advanced settings.")?;
        if devices.next().is_some() {
            return Err(
                "Several monitor sources are available. Select one in Advanced settings.".into(),
            );
        }
        let config = device.default_input_config().map_err(|e| e.to_string())?;
        Ok((device, config))
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = (host, monitor);
        Err("Meeting audio is available on Windows and Linux. Use file transcription on this platform.".into())
    }
}

impl AudioCaptureEngine {
    pub fn start_meeting_capture(
        &mut self,
        device_id: Option<String>,
        monitor_id: Option<String>,
        gain: f32,
        sender: mpsc::Sender<Vec<f32>>,
        auto_stop: mpsc::Sender<()>,
    ) -> Result<(), String> {
        self.stop_capture();
        *self.stream_error.lock() = None;
        self.dropped_chunks.store(0, Ordering::Relaxed);
        if !gain.is_finite() || !(0.5..=3.0).contains(&gain) {
            return Err("Input gain must be between 0.5 and 3.0".into());
        }
        let host = cpal::default_host();
        let (mic, mic_name) = super::capture::resolve_input_device(&host, device_id.as_deref())?;
        if mic_name.to_lowercase().contains("monitor") {
            return Err(
                "Select a microphone as the primary input, rather than a monitor source".into(),
            );
        }
        let mic_config = mic.default_input_config().map_err(|e| e.to_string())?;
        let (system, system_config) = system_device(&host, monitor_id.as_deref())?;
        *self.last_device_name.lock() = format!("{mic_name} + system audio");
        let mixer = Arc::new(Mutex::new(MeetingMixer::default()));
        let epoch = Arc::new(std::sync::OnceLock::<Instant>::new());
        let build = |device: &cpal::Device,
                     config: cpal::SupportedStreamConfig,
                     is_system: bool| {
            let epoch = epoch.clone();
            let format = config.sample_format();
            let mut resampler = AudioResampler::new(config.sample_rate().0, config.channels());
            let mut origin: Option<(cpal::StreamInstant, u64)> = None;
            let mut timeline = CaptureTimeline::default();
            let mixer = mixer.clone();
            let recording = self.is_recording.clone();
            let error_recording = recording.clone();
            let error = self.stream_error.clone();
            let auto_stop = auto_stop.clone();
            device.build_input_stream_raw(&config.into(), format, move |data, info| {
                if !recording.load(Ordering::Relaxed) { return; }
                let samples: Vec<f32> = match format {
                    cpal::SampleFormat::F32 => data.as_slice::<f32>().unwrap_or_default().to_vec(),
                    cpal::SampleFormat::I16 => data.as_slice::<i16>().unwrap_or_default().iter().map(|s| *s as f32 / 32768.0).collect(),
                    cpal::SampleFormat::I32 => data.as_slice::<i32>().unwrap_or_default().iter().map(|s| *s as f32 / 2147483648.0).collect(),
                    cpal::SampleFormat::U16 => data.as_slice::<u16>().unwrap_or_default().iter().map(|s| (*s as f32 - 32768.0) / 32768.0).collect(),
                    _ => return,
                };
                let mut mono = resampler.resample_f32(&samples);
                if !is_system { for sample in &mut mono { *sample *= gain; } }
                let Some(epoch) = epoch.get() else { return; };
                let observed = (epoch.elapsed().as_secs_f64() * RATE as f64) as u64;
                let timestamp = info.timestamp();
                let (first, base) = origin.get_or_insert_with(|| {
                    let lag = timestamp.callback.duration_since(&timestamp.capture)
                        .map(|delay| (delay.as_secs_f64() * RATE as f64) as u64)
                        .unwrap_or(mono.len() as u64);
                    (timestamp.capture, observed.saturating_sub(lag))
                });
                let capture_frame = timestamp.capture.duration_since(first)
                    .map(|offset| base.saturating_add((offset.as_secs_f64() * RATE as f64) as u64));
                let start = timeline.place(capture_frame, observed, mono.len());
                mixer.lock().push_at(is_system, start, &mono);
            }, move |err| {
                *error.lock() = Some(format!("Meeting capture stopped: {err}. Check your microphone and playback device."));
                error_recording.store(false, Ordering::SeqCst);
                let _ = auto_stop.try_send(());
            }, None).map_err(|e| format!("Cannot open meeting audio: {e}"))
        };
        for config in [&mic_config, &system_config] {
            if !matches!(
                config.sample_format(),
                cpal::SampleFormat::F32
                    | cpal::SampleFormat::I16
                    | cpal::SampleFormat::I32
                    | cpal::SampleFormat::U16
            ) {
                return Err(format!(
                    "Unsupported meeting sample format: {:?}",
                    config.sample_format()
                ));
            }
        }
        let mic_stream = build(&mic, mic_config, false)?;
        let system_stream = build(&system, system_config, true)?;
        let started = Instant::now();
        let _ = epoch.set(started);
        self.is_recording.store(true, Ordering::SeqCst);
        if let Err(error) = system_stream.play().and_then(|_| mic_stream.play()) {
            self.is_recording.store(false, Ordering::SeqCst);
            return Err(format!("Cannot start meeting audio: {error}"));
        }
        let recording = self.is_recording.clone();
        let level = self.audio_level.clone();
        let dropped = self.dropped_chunks.clone();
        // Eighty milliseconds of headroom admits callbacks from both devices.
        // Wall-clock positions preserve quiet loopback periods and differing rates.
        let worker = std::thread::Builder::new()
            .name("meeting-mix".into())
            .spawn(move || {
                let mut vad = VoiceActivityDetector::new(
                    VadConfig {
                        silence_timeout_ms: 0,
                        ..Default::default()
                    },
                    RATE as u32,
                );
                loop {
                    let live = recording.load(Ordering::SeqCst);
                    let end = (started.elapsed().as_secs_f64() * RATE as f64) as u64;
                    let until = if live { end.saturating_sub(1280) } else { end };
                    loop {
                        let mut mixer = mixer.lock();
                        let frames = until.saturating_sub(mixer.cursor).min(320) as usize;
                        if frames == 0 {
                            break;
                        }
                        let pcm = mixer.render(frames);
                        drop(mixer);
                        let (_, _, rms) = vad.process_chunk(&pcm);
                        *level.lock() = (rms * 6.0).min(1.0);
                        if matches!(
                            sender.try_send(pcm),
                            Err(mpsc::error::TrySendError::Full(_))
                        ) {
                            dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    if !live {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            })
            .map_err(|e| {
                self.is_recording.store(false, Ordering::SeqCst);
                e.to_string()
            })?;
        self.current_stream = Some(mic_stream);
        self.system_stream = Some(system_stream);
        self.meeting_thread = Some(worker);
        Ok(())
    }
}
