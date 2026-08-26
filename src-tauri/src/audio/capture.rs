use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Host, SampleFormat, Stream, StreamConfig, SupportedStreamConfig};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::resampler::AudioResampler;
use super::vad::{VadConfig, VoiceActivityDetector};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub sample_rate: u32,
    pub channels: u16,
}

pub struct AudioCaptureEngine {
    current_stream: Option<Stream>,
    is_recording: Arc<AtomicBool>,
    audio_level: Arc<Mutex<f32>>,
    last_device_name: Arc<Mutex<String>>,
}

unsafe impl Send for AudioCaptureEngine {}
unsafe impl Sync for AudioCaptureEngine {}

/// Rank Windows capture endpoints. Bluetooth Hands-Free / Stereo Mix / FxSound
/// often become the OS default and yield all-zero WASAPI buffers.
pub(crate) fn score_input_device_name(name: &str) -> i32 {
    let n = name.to_lowercase();
    if n.contains("stereo mix")
        || n.contains("what u hear")
        || n.contains("wave out mix")
        || n.contains("loopback")
    {
        return -200;
    }
    if n.contains("fxsound")
        || n.contains("voicemeeter")
        || n.contains("vb-audio")
        || n.contains("cable input")
        || n.contains("cable output")
        || n.contains("virtual")
    {
        return -120;
    }
    if n.contains("hands-free") || n.contains("handsfree") {
        return -80;
    }
    // Bluetooth "Headset" capture pins are often silent unless a call is active.
    if n.contains("headset") && !n.contains("microphone") {
        return -40;
    }
    if n.contains("headphones") || n.contains("speakers") || n.contains("mapper") {
        return -90;
    }
    if n.contains("microphone array") {
        return 120;
    }
    if n.contains("microphone") {
        return 90;
    }
    if n.split_whitespace().any(|w| w == "mic") {
        return 70;
    }
    10
}

fn resolve_input_device(host: &Host, device_id: Option<&str>) -> Result<(Device, String), String> {
    if let Some(id) = device_id {
        if id != "default" {
            if let Some(dev) = host
                .input_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().map(|n| n == id).unwrap_or(false))
            {
                let name = dev.name().unwrap_or_else(|_| id.to_string());
                return Ok((dev, name));
            }
        }
    }

    let default = host.default_input_device();
    let default_name = default
        .as_ref()
        .and_then(|d| d.name().ok())
        .unwrap_or_default();
    if !default_name.is_empty() && score_input_device_name(&default_name) >= 0 {
        if let Some(dev) = default {
            return Ok((dev, default_name));
        }
    }

    let mut ranked: Vec<(i32, String, Device)> = Vec::new();
    if let Ok(inputs) = host.input_devices() {
        for dev in inputs {
            if let Ok(name) = dev.name() {
                ranked.push((score_input_device_name(&name), name, dev));
            }
        }
    }
    ranked.sort_by(|a, b| b.0.cmp(&a.0));
    if let Some((score, name, dev)) = ranked.into_iter().next() {
        if score < 0 {
            return Err(format!(
                "No usable microphone found (best candidate '{name}' is a virtual/Hands-Free endpoint)."
            ));
        }
        log::info!("Default capture '{default_name}' looks unusable; using '{name}' (score {score})");
        return Ok((dev, name));
    }

    host.default_input_device()
        .map(|d| {
            let name = d.name().unwrap_or_else(|_| "default".into());
            (d, name)
        })
        .ok_or_else(|| "No audio input device found on the system.".to_string())
}

/// Communications-class endpoints (16 kHz mono) are silent on some Windows 11
/// WASAPI builds. Prefer 44.1/48 kHz when the device supports it.
fn pick_input_config(device: &Device) -> Result<SupportedStreamConfig, String> {
    let default = device
        .default_input_config()
        .map_err(|e| format!("Failed to read device default input configuration: {e}"))?;

    // Shared-mode WASAPI is silent if we request a format other than the
    // device mix. Stick to default_input_config (GetMixFormat).
    let _ = device;
    Ok(default)
}

impl AudioCaptureEngine {
    pub fn new() -> Self {
        Self {
            current_stream: None,
            is_recording: Arc::new(AtomicBool::new(false)),
            audio_level: Arc::new(Mutex::new(0.0)),
            last_device_name: Arc::new(Mutex::new(String::new())),
        }
    }

    pub fn get_current_audio_level(&self) -> f32 {
        *self.audio_level.lock()
    }

    pub fn last_device_name(&self) -> String {
        self.last_device_name.lock().clone()
    }

    /// List all available input audio devices
    pub fn list_input_devices() -> Vec<AudioDeviceInfo> {
        let host = cpal::default_host();
        let default_device_name = host
            .default_input_device()
            .and_then(|d| d.name().ok())
            .unwrap_or_default();

        let mut devices = Vec::new();

        if let Ok(input_devices) = host.input_devices() {
            for dev in input_devices {
                if let Ok(name) = dev.name() {
                    let default_config = dev.default_input_config();
                    let (sample_rate, channels) = match default_config {
                        Ok(cfg) => (cfg.sample_rate().0, cfg.channels()),
                        Err(_) => (48000, 2),
                    };

                    devices.push(AudioDeviceInfo {
                        id: name.clone(),
                        name: name.clone(),
                        is_default: name == default_device_name,
                        sample_rate,
                        channels,
                    });
                }
            }
        }

        devices.sort_by(|a, b| {
            score_input_device_name(&b.name).cmp(&score_input_device_name(&a.name))
        });
        devices
    }

    /// Starts capturing audio and streaming 16kHz mono f32 samples to the provided sender channel
    pub fn start_capture(
        &mut self,
        device_id: Option<String>,
        gain: f32,
        vad_sensitivity: f32,
        silence_timeout_ms: u64,
        sample_sender: mpsc::UnboundedSender<Vec<f32>>,
        auto_stop_notify: mpsc::Sender<()>,
    ) -> Result<(), String> {
        self.stop_capture();

        let host = cpal::default_host();
        let (device, device_name) = resolve_input_device(&host, device_id.as_deref())?;
        let supported = pick_input_config(&device)?;

        let sample_rate = supported.sample_rate().0;
        let channels = supported.channels();
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();

        log::info!(
            "Mic capture: '{device_name}' {sample_rate} Hz {channels} ch {sample_format:?} gain={gain:.2}"
        );
        *self.last_device_name.lock() = device_name;

        let resampler = Arc::new(Mutex::new(AudioResampler::new(sample_rate, channels)));

        let mut vad = VoiceActivityDetector::new(
            VadConfig {
                silence_timeout_ms,
                ..Default::default()
            },
            16000,
        );
        vad.set_sensitivity(vad_sensitivity);
        let vad_mutex = Arc::new(Mutex::new(vad));

        self.is_recording.store(true, Ordering::SeqCst);
        let is_recording_flag = Arc::clone(&self.is_recording);
        let audio_level_ref = Arc::clone(&self.audio_level);

        let err_fn = |err| {
            log::error!("Audio stream error occurred: {:?}", err);
        };

        let stream = match sample_format {
            SampleFormat::F32 => {
                let resampler_clone = Arc::clone(&resampler);
                let vad_clone = Arc::clone(&vad_mutex);
                let sender_clone = sample_sender.clone();
                let auto_stop_clone = auto_stop_notify.clone();

                device.build_input_stream(
                    &config,
                    move |data: &[f32], _: &_| {
                        if !is_recording_flag.load(Ordering::Relaxed) {
                            return;
                        }
                        let gained: Vec<f32> = if (gain - 1.0).abs() > 0.01 {
                            data.iter().map(|&s| s * gain).collect()
                        } else {
                            data.to_vec()
                        };
                        dispatch_chunk(
                            &gained,
                            &resampler_clone,
                            &vad_clone,
                            &audio_level_ref,
                            &sender_clone,
                            &auto_stop_clone,
                        );
                    },
                    err_fn,
                    None,
                )
            }
            SampleFormat::I16 => {
                let resampler_clone = Arc::clone(&resampler);
                let vad_clone = Arc::clone(&vad_mutex);
                let sender_clone = sample_sender.clone();
                let auto_stop_clone = auto_stop_notify.clone();

                device.build_input_stream(
                    &config,
                    move |data: &[i16], _: &_| {
                        if !is_recording_flag.load(Ordering::Relaxed) {
                            return;
                        }
                        let f32_samples: Vec<f32> = data
                            .iter()
                            .map(|&s| (s as f32 / 32768.0) * gain)
                            .collect();
                        dispatch_chunk(
                            &f32_samples,
                            &resampler_clone,
                            &vad_clone,
                            &audio_level_ref,
                            &sender_clone,
                            &auto_stop_clone,
                        );
                    },
                    err_fn,
                    None,
                )
            }
            SampleFormat::I32 => {
                let resampler_clone = Arc::clone(&resampler);
                let vad_clone = Arc::clone(&vad_mutex);
                let sender_clone = sample_sender.clone();
                let auto_stop_clone = auto_stop_notify.clone();

                device.build_input_stream(
                    &config,
                    move |data: &[i32], _: &_| {
                        if !is_recording_flag.load(Ordering::Relaxed) {
                            return;
                        }
                        let f32_samples: Vec<f32> = data
                            .iter()
                            .map(|&s| (s as f32 / 2147483648.0) * gain)
                            .collect();
                        dispatch_chunk(
                            &f32_samples,
                            &resampler_clone,
                            &vad_clone,
                            &audio_level_ref,
                            &sender_clone,
                            &auto_stop_clone,
                        );
                    },
                    err_fn,
                    None,
                )
            }
            SampleFormat::U16 => {
                let resampler_clone = Arc::clone(&resampler);
                let vad_clone = Arc::clone(&vad_mutex);
                let sender_clone = sample_sender.clone();
                let auto_stop_clone = auto_stop_notify.clone();

                device.build_input_stream(
                    &config,
                    move |data: &[u16], _: &_| {
                        if !is_recording_flag.load(Ordering::Relaxed) {
                            return;
                        }
                        let f32_samples: Vec<f32> = data
                            .iter()
                            .map(|&s| ((s as f32 - 32768.0) / 32768.0) * gain)
                            .collect();
                        dispatch_chunk(
                            &f32_samples,
                            &resampler_clone,
                            &vad_clone,
                            &audio_level_ref,
                            &sender_clone,
                            &auto_stop_clone,
                        );
                    },
                    err_fn,
                    None,
                )
            }
            other => {
                return Err(format!("Unsupported audio sample format: {other:?}"));
            }
        }
        .map_err(|e| format!("Failed to build audio stream: {e}"))?;

        stream
            .play()
            .map_err(|e| format!("Failed to start audio stream: {e}"))?;

        self.current_stream = Some(stream);
        Ok(())
    }

    pub fn stop_capture(&mut self) {
        self.is_recording.store(false, Ordering::SeqCst);
        if let Some(stream) = self.current_stream.take() {
            let _ = stream.pause();
        }
        *self.audio_level.lock() = 0.0;
    }
}

fn dispatch_chunk(
    native: &[f32],
    resampler: &Mutex<AudioResampler>,
    vad: &Mutex<VoiceActivityDetector>,
    audio_level: &Mutex<f32>,
    sender: &mpsc::UnboundedSender<Vec<f32>>,
    auto_stop: &mpsc::Sender<()>,
) {
    let mono_16k = resampler.lock().resample_f32(native);
    if mono_16k.is_empty() {
        return;
    }

    let mut vad_guard = vad.lock();
    let (_is_speech, should_auto_stop, rms) = vad_guard.process_chunk(&mono_16k);
    drop(vad_guard);

    *audio_level.lock() = (rms * 6.0).min(1.0);
    let _ = sender.send(mono_16k);
    if should_auto_stop {
        let _ = auto_stop.try_send(());
    }
}

#[cfg(test)]
mod tests {
    use super::score_input_device_name;

    #[test]
    fn prefers_real_mic_over_handsfree_and_virtual() {
        let amd = score_input_device_name("Microphone Array (AMD Audio Device)");
        let hfp = score_input_device_name("Headset (SA-HP P10 Hands-Free)");
        let headset = score_input_device_name("Headset (SA-HP P10)");
        let mix = score_input_device_name("Stereo Mix (Realtek(R) Audio)");
        let fx = score_input_device_name("FxSound Speakers (FxSound Audio Enhancer)");
        assert!(amd > hfp);
        assert!(amd > headset);
        assert!(amd > mix);
        assert!(amd > fx);
        assert!(hfp < 0);
        assert!(headset < 0);
        assert!(mix < 0);
    }
}
