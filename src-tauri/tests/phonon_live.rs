//! Opt-in real model test: no microphone access or downloads.
use reflow_lib::{
    asr::{ASREngine, Qwen3AsrSidecar},
    audio::resampler::AudioResampler,
};
use std::time::{Duration, Instant};

#[test]
#[ignore = "requires installed Phonon weights, Python dependencies and REFLOW_PHONON_AUDIO"]
fn phonon_real_sidecar_selected_devices() {
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
    let directory = if let Some(path) = std::env::var_os("REFLOW_PHONON_MODEL_DIR") {
        std::path::PathBuf::from(path)
    } else {
        let manager = reflow_lib::model::manager::ModelManager::new(
            reflow_lib::platform::PlatformSys::get_models_dir(),
        );
        assert!(manager.is_installed("phonon-2"));
        manager.get_model_dir("phonon-2")
    };
    assert!(directory.join("phonon-2.bps.tar.zst").is_file());
    let requested = std::env::var("REFLOW_PHONON_DEVICES").unwrap_or_else(|_| "cpu,cuda".into());
    let devices = requested.split(',').map(str::trim).collect::<Vec<_>>();
    assert!(
        !devices.is_empty()
            && devices
                .iter()
                .all(|device| matches!(*device, "cpu" | "cuda"))
    );
    println!("Requested devices: {devices:?}");
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
    let mut engine = Qwen3AsrSidecar::new();
    engine.initialize().unwrap();
    for device in devices {
        let loading = Instant::now();
        engine
            .load_model_with_precision(&directory.to_string_lossy(), device, "auto")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(240);
        let loaded = loop {
            let status = engine.engine_status();
            assert!(status.error.is_none(), "{status:?}");
            if status.loaded && !status.is_loading {
                assert_eq!(status.device, device);
                assert!(status.backend.contains("Phonon-2"));
                if device == "cpu" {
                    assert!(matches!(status.precision.as_str(), "int8" | "fp32"));
                } else {
                    assert_eq!(status.precision, "fp32");
                }
                if let Ok(expected) = std::env::var("REFLOW_PHONON_EXPECT_PRECISION") {
                    assert_eq!(status.precision, expected);
                }
                break status;
            }
            assert!(Instant::now() < deadline, "load timed out: {status:?}");
            std::thread::sleep(Duration::from_millis(500));
        };
        let load_ms = loading.elapsed().as_millis();
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
        println!(
            "REFLOW_PHONON_ACCEPTANCE:{}",
            serde_json::json!({
                "device":device,"precision":loaded.precision,"backend":loaded.backend,
                "load_ms":load_ms,"stop_and_decode_ms":started.elapsed().as_millis(),
                "text":text,"reference":reference,"reference_words_match":true,
                "audio_seconds":samples.len() as f64 / 16000.0,"other_language_rejected":true,
                "model_downloads":0,"native_desktop_acceptance":false
            })
        );
        engine.unload_model().unwrap();
    }
}
