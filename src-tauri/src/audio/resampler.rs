/// Linear interpolation resampler for converting arbitrary sample rates and channel counts to mono 16000 Hz.
///
/// Streaming-safe: leftover samples and fractional phase are carried across
/// `resample_f32` calls so 44.1 kHz (and other non-integer ratios) don't click
/// or drop samples at chunk boundaries.
pub struct AudioResampler {
    input_sample_rate: u32,
    input_channels: u16,
    target_sample_rate: u32,
    leftover: Vec<f32>,
    phase: f64,
}

impl AudioResampler {
    pub fn new(input_sample_rate: u32, input_channels: u16) -> Self {
        Self {
            input_sample_rate,
            input_channels,
            target_sample_rate: 16000,
            leftover: Vec::new(),
            phase: 0.0,
        }
    }

    fn downmix(&self, input: &[f32]) -> Vec<f32> {
        if self.input_channels <= 1 {
            input.to_vec()
        } else {
            let channels = self.input_channels as usize;
            input
                .chunks_exact(channels)
                .map(|chunk| chunk.iter().sum::<f32>() / channels as f32)
                .collect()
        }
    }

    /// Resample multi-channel f32 input into 16kHz mono f32 samples
    pub fn resample_f32(&mut self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() && self.leftover.is_empty() {
            return Vec::new();
        }

        let mut src = std::mem::take(&mut self.leftover);
        if !input.is_empty() {
            src.extend(self.downmix(input));
        }

        if self.input_sample_rate == self.target_sample_rate {
            return src;
        }

        let step = self.input_sample_rate as f64 / self.target_sample_rate as f64;
        let mut pos = self.phase;
        let mut output = Vec::with_capacity(((src.len() as f64) / step) as usize + 1);

        while pos + 1.0 < src.len() as f64 {
            let idx0 = pos.floor() as usize;
            let frac = (pos - idx0 as f64) as f32;
            let s0 = src[idx0];
            let s1 = src[idx0 + 1];
            output.push(s0 + frac * (s1 - s0));
            pos += step;
        }

        let keep = pos.floor() as usize;
        self.leftover = if keep < src.len() {
            src[keep..].to_vec()
        } else {
            Vec::new()
        };
        self.phase = pos - keep as f64;
        output
    }

    /// Converts little-endian 16-bit PCM bytes to f32 samples in [-1, 1].
    pub fn pcm16_bytes_to_f32(bytes: &[u8]) -> Vec<f32> {
        bytes
            .chunks_exact(2)
            .map(|chunk| {
                let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                sample as f32 / 32768.0
            })
            .collect()
    }

    /// Converts 16kHz mono f32 samples to 16-bit PCM bytes (Little Endian)
    pub fn f32_to_pcm16_bytes(samples: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for &s in samples {
            let clamped = s.clamp(-1.0, 1.0);
            let sample_i16 = (clamped * 32767.0) as i16;
            bytes.extend_from_slice(&sample_i16.to_le_bytes());
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resampling_48k_to_16k() {
        let mut resampler = AudioResampler::new(48000, 2);
        // 48000 stereo samples (24000 frames = 0.5s)
        let stereo_samples = vec![0.5f32; 48000];
        let resampled = resampler.resample_f32(&stereo_samples);
        // 0.5s at 16000Hz should produce ~8000 samples (streaming interpolator
        // holds 1 source frame of lookahead, so it may be 1 sample short).
        assert!((resampled.len() as i32 - 8000).abs() <= 2);
        for &s in &resampled {
            assert!((s - 0.5).abs() < 1e-4);
        }
    }

    #[test]
    fn streaming_44100_matches_offline_length() {
        let src: Vec<f32> = (0..44100)
            .map(|i| ((i as f32) * 440.0 * 2.0 * std::f32::consts::PI / 44100.0).sin())
            .collect();
        let mut offline = AudioResampler::new(44100, 1);
        let whole = offline.resample_f32(&src);

        let mut streamed = AudioResampler::new(44100, 1);
        let mut parts = Vec::new();
        for chunk in src.chunks(512) {
            parts.extend(streamed.resample_f32(chunk));
        }
        // Streaming carries a 1-sample interpolator tail; lengths stay within ~1 ms.
        assert!(
            (whole.len() as i32 - parts.len() as i32).abs() <= 32,
            "offline {} vs streamed {}",
            whole.len(),
            parts.len()
        );
        assert!(parts.len() > 15000 && parts.len() < 17000);
    }

    #[test]
    fn pcm16_roundtrip_sign() {
        let samples = vec![-1.0f32, 0.0, 0.5];
        let bytes = AudioResampler::f32_to_pcm16_bytes(&samples);
        let back = AudioResampler::pcm16_bytes_to_f32(&bytes);
        assert_eq!(back.len(), 3);
        assert!(back[0] < -0.9);
        assert!(back[1].abs() < 0.01);
        assert!((back[2] - 0.5).abs() < 0.01);
    }
}
