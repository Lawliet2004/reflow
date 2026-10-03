use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MicrophoneHealth {
    pub device: String,
    pub duration_ms: u64,
    pub peak: f32,
    pub rms: f32,
    pub clipped_pct: f32,
    pub dropped_chunks: u64,
    pub assessment: String,
}

impl MicrophoneHealth {
    pub fn from_samples(device: String, samples: &[f32], dropped_chunks: u64) -> Self {
        let peak = samples
            .iter()
            .fold(0.0f32, |peak, value| peak.max(value.abs()));
        let rms = (samples.iter().map(|v| (*v as f64).powi(2)).sum::<f64>()
            / samples.len().max(1) as f64)
            .sqrt() as f32;
        let clipped_pct = samples.iter().filter(|v| v.abs() >= 0.99).count() as f32 * 100.0
            / samples.len().max(1) as f32;
        let assessment = if samples.is_empty() {
            "No audio received. Check microphone permissions and connection."
        } else if dropped_chunks > 0 {
            "Audio frames were dropped. Stop other intensive tasks and try again."
        } else if peak < 0.001 {
            "No signal detected. Check mute, permissions, and the selected microphone."
        } else if clipped_pct > 0.5 {
            "Audio is clipping. Lower input gain or move away from the microphone."
        } else if rms < 0.005 {
            "Signal is quiet. Move closer or increase input gain slightly."
        } else {
            "Audio signal looks healthy. This checks input levels, not recognition accuracy."
        };
        Self {
            device,
            duration_ms: samples.len() as u64 * 1000 / 16_000,
            peak,
            rms,
            clipped_pct,
            dropped_chunks,
            assessment: assessment.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measures_real_signal_and_identifies_missing_clipped_and_dropped_audio() {
        let missing = MicrophoneHealth::from_samples("mic".into(), &[0.0; 16000], 0);
        assert_eq!(missing.duration_ms, 1000);
        assert!(missing.assessment.contains("No signal"));
        let clipped = MicrophoneHealth::from_samples("mic".into(), &[1.0; 100], 0);
        assert_eq!(clipped.clipped_pct, 100.0);
        assert!(clipped.assessment.contains("clipping"));
        let healthy = MicrophoneHealth::from_samples("mic".into(), &[0.1; 100], 0);
        assert!((healthy.rms - 0.1).abs() < 0.0001);
        assert!(healthy.assessment.contains("healthy"));
        assert!(MicrophoneHealth::from_samples("mic".into(), &[0.1; 100], 2)
            .assessment
            .contains("dropped"));
    }
}
