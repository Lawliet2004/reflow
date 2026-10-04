//! Opt-in real model test: no microphone access or downloads.
use reflow_lib::{
    asr::{ASREngine, Qwen3AsrSidecar},
    audio::resampler::AudioResampler,
};
use std::time::{Duration, Instant};

#[test]
#[ignore = "requires installed Phonon weights, Python dependencies and REFLOW_PHONON_AUDIO"]
fn phonon_real_sidecar_cpu_and_cuda() {
    let audio =
        std::env::var("REFLOW_PHONON_AUDIO").expect("set REFLOW_PHONON_AUDIO to a PCM16 WAV");
    let mut wav = hound::WavReader::open(audio).unwrap();
    let spec = wav.spec();
    assert_eq!(spec.bits_per_sample, 16);
    let input: Vec<f32> = wav
        .samples::<i16>()
        .map(|s| s.unwrap() as f32 / 32768.0)
        .collect();
    let samples = AudioResampler::new(spec.sample_rate, spec.channels).resample_f32(&input);
    let manager = reflow_lib::model::manager::ModelManager::new(
        reflow_lib::platform::PlatformSys::get_models_dir(),
    );
    assert!(manager.is_installed("phonon-2"));
    let directory = manager.get_model_dir("phonon-2");
    let reference = std::env::var("REFLOW_PHONON_REFERENCE")
        .expect("set REFLOW_PHONON_REFERENCE to the exact spoken words");
    let normalize = |s: &str| {
        s.split_whitespace()
            .map(|w| {
                w.chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect::<String>()
                    .to_lowercase()
            })
            .collect::<Vec<_>>()
    };
    for device in ["cpu", "cuda"] {
        let mut engine = Qwen3AsrSidecar::new();
        engine.initialize().unwrap();
        engine
            .load_model_with_precision(&directory.to_string_lossy(), device, "auto")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(240);
        loop {
            let status = engine.engine_status();
            assert!(status.error.is_none(), "{status:?}");
            if status.loaded && !status.is_loading {
                assert_eq!(status.device, device);
                assert!(status.backend.contains("Phonon-2"));
                assert_eq!(
                    status.precision,
                    if device == "cpu" { "int8" } else { "fp32" }
                );
                break;
            }
            assert!(Instant::now() < deadline, "load timed out: {status:?}");
            std::thread::sleep(Duration::from_millis(500));
        }
        assert!(engine
            .start_stream("hi", &[])
            .unwrap_err()
            .contains("English"));
        engine.start_stream("en", &[]).unwrap();
        for chunk in samples.chunks(16000) {
            engine.push_audio(chunk).unwrap();
        }
        let started = Instant::now();
        let text = engine.stop_stream().unwrap();
        println!("{device}: {:?}: {text}", started.elapsed());
        assert_eq!(normalize(&text), normalize(&reference));
        assert_eq!(engine.get_detected_language(), "en");
        engine.unload_model().unwrap();
    }
}
