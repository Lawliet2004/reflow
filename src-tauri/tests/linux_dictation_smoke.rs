//! Opt-in Linux smoke checks using the installed speech model and audio devices.
//! REFLOW_SMOKE_WAV points to a known English WAV; REFLOW_SMOKE_MODEL to the
//! downloaded Zipformer directory. Run with `--ignored --nocapture`.
#![cfg(target_os = "linux")]

use reflow_lib::asr::AsrHandle;
use reflow_lib::audio::{file_decode::decode_file, preflight::capture_sample};
use reflow_lib::settings::AppSettings;
use std::time::Duration;

fn thread_count() -> usize {
    std::fs::read_dir("/proc/self/task").unwrap().count()
}

fn child_count() -> usize {
    let parent = std::process::id().to_string();
    std::fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter(|entry| {
            std::fs::read_to_string(entry.path().join("status")).is_ok_and(|status| {
                status
                    .lines()
                    .any(|line| line.strip_prefix("PPid:").is_some_and(|pid| pid.trim() == parent))
            })
        })
        .count()
}

async fn loaded_model() -> AsrHandle {
    let model = std::env::var("REFLOW_SMOKE_MODEL").expect("set REFLOW_SMOKE_MODEL");
    let engine = AsrHandle::new_sidecar();
    engine
        .load_model_with_precision(&model, "cpu", "int8")
        .await
        .expect("load installed Zipformer");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let status = engine.refresh_status().await.expect("read model status");
        assert!(status.error.is_none(), "model error: {:?}", status.error);
        if status.loaded && !status.is_loading {
            println!("Model ready: {}", status.backend);
            return engine;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "model load timed out"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test]
#[ignore = "requires installed Zipformer and REFLOW_SMOKE_WAV English fixture"]
async fn rust_actor_transcribes_english_and_recovers_after_cancel() {
    let wav = std::env::var("REFLOW_SMOKE_WAV").expect("set REFLOW_SMOKE_WAV");
    let audio = decode_file(std::path::Path::new(&wav), false).expect("decode speech fixture");
    let engine = loaded_model().await;
    let expected = "after early nightfall the yellow lamps would light up here and there";
    let mut settled_workers = None;
    for cycle in 0..30 {
        let session = engine.next_session_id();
        engine.start_stream(session, "en", &[]).await.unwrap();
        for (index, chunk) in audio.chunks(16_000).enumerate() {
            engine
                .push_audio(session, index as u32, chunk.to_vec())
                .await
                .unwrap();
        }
        let text = engine.stop_stream(session).await.unwrap();
        println!("Cycle {}: {text}", cycle + 1);
        assert!(
            text.to_lowercase().contains(expected),
            "unexpected transcript: {text}"
        );
        let cancelled = engine.next_session_id();
        engine.start_stream(cancelled, "en", &[]).await.unwrap();
        engine.cancel_stream(cancelled).await.unwrap();
        let threads = thread_count();
        let children = child_count();
        if let Some((baseline_threads, baseline_children)) = settled_workers {
            assert!(
                threads <= baseline_threads,
                "ASR threads grew in cycle {cycle}"
            );
            assert_eq!(
                children, baseline_children,
                "ASR children grew in cycle {cycle}"
            );
        } else {
            settled_workers = Some((threads, children));
        }
    }
    engine.unload_model().await.unwrap();
}

#[tokio::test]
#[ignore = "opens the configured Linux microphone for three seconds"]
async fn linux_microphone_delivers_audio_to_capture_pipeline() {
    let settings = AppSettings {
        microphone_device_id: std::env::var("REFLOW_SMOKE_MIC").ok(),
        ..AppSettings::default()
    };
    let (health, samples) = capture_sample(&settings, 3, None)
        .await
        .expect("open Linux microphone");
    println!("Microphone health: {health:?}");
    assert!(
        samples.len() >= 16_000,
        "microphone delivered too little audio"
    );
    assert_eq!(health.dropped_chunks, 0, "microphone dropped frames");
    assert!(
        samples.iter().all(|value| value.is_finite()),
        "invalid samples"
    );
}

#[tokio::test]
#[ignore = "opens REFLOW_SMOKE_MIC for 30 one-second capture/cancel cycles"]
async fn repeated_linux_capture_reaps_workers_without_dropped_chunks() {
    let settings = AppSettings {
        microphone_device_id: std::env::var("REFLOW_SMOKE_MIC").ok(),
        ..AppSettings::default()
    };
    let before_children = child_count();
    let mut settled_threads = None;
    for cycle in 0..30 {
        let (health, samples) = capture_sample(&settings, 1, None).await.unwrap();
        assert!(!samples.is_empty(), "cycle {cycle} received no audio");
        assert_eq!(health.dropped_chunks, 0, "cycle {cycle} dropped audio");
        assert!(samples.iter().all(|value| value.is_finite()));
        assert_eq!(
            child_count(),
            before_children,
            "cycle {cycle} leaked a capture process"
        );
        if let Some(count) = settled_threads {
            assert!(thread_count() <= count, "cycle {cycle} leaked threads");
        } else {
            settled_threads = Some(thread_count());
        }
    }
}

#[tokio::test]
#[ignore = "requires an isolated PipeWire loopback, REFLOW_SMOKE_PLAYBACK_SINK, and speech fixture"]
async fn pipewire_capture_resampling_and_recognition_work_together() {
    let sink = std::env::var("REFLOW_SMOKE_PLAYBACK_SINK").expect("set isolated playback sink");
    let wav = std::env::var("REFLOW_SMOKE_WAV").expect("set speech fixture");
    let input = format!("pipewire:{sink}.monitor");
    let engine = loaded_model().await;
    let playback = tokio::task::spawn_blocking(move || {
        std::thread::sleep(Duration::from_secs(1));
        std::process::Command::new("paplay")
            .args(["--device", &sink, &wav])
            .status()
            .expect("play fixture into isolated loopback")
    });
    let settings = AppSettings {
        microphone_device_id: Some(input),
        ..AppSettings::default()
    };
    let (health, samples) = capture_sample(&settings, 10, None)
        .await
        .expect("capture PipeWire fixture");
    assert!(playback.await.unwrap().success());
    println!("Loopback capture health: {health:?}");
    assert!(health.peak > 0.01, "fixture did not reach capture stream");
    assert_eq!(health.dropped_chunks, 0);
    let session = engine.next_session_id();
    engine.start_stream(session, "en", &[]).await.unwrap();
    for (index, chunk) in samples.chunks(16_000).enumerate() {
        engine
            .push_audio(session, index as u32, chunk.to_vec())
            .await
            .unwrap();
    }
    let text = engine.stop_stream(session).await.unwrap();
    println!("Captured transcript: {text}");
    assert!(text
        .to_lowercase()
        .contains("after early nightfall the yellow lamps"));
    engine.unload_model().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an isolated REFLOW_SMOKE_PLAYBACK_SINK and speech fixture"]
async fn meeting_capture_mixes_two_pipewire_streams_and_reaps_them() {
    let sink = std::env::var("REFLOW_SMOKE_PLAYBACK_SINK").unwrap();
    let wav = std::env::var("REFLOW_SMOKE_WAV").unwrap();
    let source = format!("pipewire:{sink}.monitor");
    let (sender, mut receiver) = tokio::sync::mpsc::channel(256);
    let (stop, _) = tokio::sync::mpsc::channel(1);
    let mut engine = reflow_lib::audio::AudioCaptureEngine::new();
    engine
        .start_meeting_capture(
            std::env::var("REFLOW_SMOKE_MIC").ok(),
            Some(source),
            1.0,
            sender,
            stop,
        )
        .unwrap();
    let playback = tokio::task::spawn_blocking(move || {
        std::thread::sleep(Duration::from_secs(1));
        std::process::Command::new("paplay")
            .args(["--device", &sink, &wav])
            .status()
            .unwrap()
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(9);
    let mut samples = Vec::new();
    while let Ok(Some(chunk)) = tokio::time::timeout_at(deadline, receiver.recv()).await {
        samples.extend(chunk);
    }
    engine.stop_capture();
    assert!(playback.await.unwrap().success());
    assert!(
        engine.stream_error().is_none(),
        "{:?}",
        engine.stream_error()
    );
    assert_eq!(engine.dropped_chunks(), 0);
    assert!(samples.len() >= 16_000);
    assert!(samples.iter().all(|value| value.is_finite()));
    assert!(samples.iter().any(|value| value.abs() > 0.01));
}
