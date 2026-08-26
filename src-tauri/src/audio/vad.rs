use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct VadConfig {
    pub energy_threshold: f32,
    pub silence_timeout_ms: u64,
    pub pre_roll_ms: u64,
    pub post_roll_ms: u64,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            energy_threshold: 0.015,
            silence_timeout_ms: 1500,
            pre_roll_ms: 300,
            post_roll_ms: 500,
        }
    }
}

pub struct VoiceActivityDetector {
    config: VadConfig,
    pre_roll_buffer: VecDeque<f32>,
    max_pre_roll_samples: usize,
    post_roll_samples_remaining: usize,
    max_post_roll_samples: usize,
    consecutive_silence_samples: usize,
    silence_timeout_samples: usize,
    is_speaking: bool,
    noise_floor: f32,
}

impl VoiceActivityDetector {
    pub fn new(config: VadConfig, sample_rate: u32) -> Self {
        let max_pre_roll_samples = ((config.pre_roll_ms as f32 / 1000.0) * sample_rate as f32) as usize;
        let max_post_roll_samples = ((config.post_roll_ms as f32 / 1000.0) * sample_rate as f32) as usize;
        let silence_timeout_samples = ((config.silence_timeout_ms as f32 / 1000.0) * sample_rate as f32) as usize;

        Self {
            config,
            pre_roll_buffer: VecDeque::with_capacity(max_pre_roll_samples),
            max_pre_roll_samples,
            post_roll_samples_remaining: 0,
            max_post_roll_samples,
            consecutive_silence_samples: 0,
            silence_timeout_samples,
            is_speaking: false,
            noise_floor: 0.005,
        }
    }

    pub fn set_sensitivity(&mut self, sensitivity: f32) {
        // Sensitivity 0.0 -> threshold 0.04 (less sensitive, requires louder voice)
        // Sensitivity 1.0 -> threshold 0.005 (more sensitive)
        let clamped = sensitivity.clamp(0.0, 1.0);
        self.config.energy_threshold = 0.04 - clamped * 0.035;
    }

    pub fn set_silence_timeout_ms(&mut self, timeout_ms: u64, sample_rate: u32) {
        self.config.silence_timeout_ms = timeout_ms;
        self.silence_timeout_samples = ((timeout_ms as f32 / 1000.0) * sample_rate as f32) as usize;
    }

    /// Process a chunk of 16kHz mono audio samples.
    /// Returns: (is_voice_active, should_auto_stop, rms_level)
    pub fn process_chunk(&mut self, chunk: &[f32]) -> (bool, bool, f32) {
        if chunk.is_empty() {
            return (false, false, 0.0);
        }

        // Calculate Root Mean Square (RMS) energy
        let sum_sq: f32 = chunk.iter().map(|&s| s * s).sum();
        let rms = (sum_sq / chunk.len() as f32).sqrt();

        // Update adaptive noise floor slowly during low volume
        if rms < self.config.energy_threshold * 0.8 {
            self.noise_floor = 0.95 * self.noise_floor + 0.05 * rms;
        }

        let threshold = (self.config.energy_threshold).max(self.noise_floor * 2.0);
        let frame_is_speech = rms > threshold;

        let mut auto_stop = false;

        if frame_is_speech {
            self.is_speaking = true;
            self.consecutive_silence_samples = 0;
            self.post_roll_samples_remaining = self.max_post_roll_samples;
        } else {
            self.consecutive_silence_samples += chunk.len();

            if self.post_roll_samples_remaining > 0 {
                self.post_roll_samples_remaining = self
                    .post_roll_samples_remaining
                    .saturating_sub(chunk.len());
            } else if self.is_speaking {
                self.is_speaking = false;
            }

            // A zero timeout means auto-stop is disabled (push-to-talk).
            if self.silence_timeout_samples > 0
                && self.consecutive_silence_samples >= self.silence_timeout_samples
            {
                auto_stop = true;
            }
        }

        // Maintain pre-roll buffer
        for &s in chunk {
            if self.pre_roll_buffer.len() >= self.max_pre_roll_samples {
                self.pre_roll_buffer.pop_front();
            }
            self.pre_roll_buffer.push_back(s);
        }

        let active = frame_is_speech || self.post_roll_samples_remaining > 0;
        (active, auto_stop, rms)
    }

    /// Extract pre-roll audio buffer on speech onset
    pub fn drain_pre_roll(&mut self) -> Vec<f32> {
        self.pre_roll_buffer.drain(..).collect()
    }

    pub fn reset(&mut self) {
        self.pre_roll_buffer.clear();
        self.consecutive_silence_samples = 0;
        self.post_roll_samples_remaining = 0;
        self.is_speaking = false;
    }
}
