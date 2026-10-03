use reflow_lib::asr::{ASREngine, MockASREngine};
use reflow_lib::audio::{VadConfig, VoiceActivityDetector};
use reflow_lib::formatting::{format_transcript, CustomReplacements};
use reflow_lib::history::{HistoryEntry, HistoryStore};
use reflow_lib::platform::PlatformSys;
use std::fs;

#[test]
fn test_100_consecutive_dictation_cycles_stress_and_leak_detection() {
    let test_dir = std::env::temp_dir().join(format!("reflow_stress_{}", uuid::Uuid::new_v4()));
    let db_path = test_dir.join("stress_history.db");
    let store = HistoryStore::new(db_path).expect("Failed to create history store for stress test");

    let mut engine = MockASREngine::new();
    engine
        .initialize()
        .expect("Failed to initialize mock ASR engine");

    let initial_metrics = PlatformSys::get_system_metrics();
    let initial_app_ram = initial_metrics.app_ram_mb;

    let custom = CustomReplacements::new(vec![]);

    for cycle in 0..100 {
        // 1. Audio VAD simulation
        let mut vad = VoiceActivityDetector::new(
            VadConfig {
                energy_threshold: 0.02,
                silence_timeout_ms: 300,
                post_roll_ms: 100,
            },
            16000,
        );

        let audio_chunk = vec![0.08f32; 1600]; // 100ms
        let (is_speech, _, _) = vad.process_chunk(&audio_chunk);
        assert!(is_speech);

        // 2. ASR streaming cycle
        engine.start_stream("en", &[]).expect("Start stream failed");
        let partial = engine.push_audio(&audio_chunk).expect("Push audio failed");
        assert!(partial.is_some());
        let raw_transcript = engine.stop_stream().expect("Stop stream failed");
        assert!(!raw_transcript.is_empty());

        // 3. Formatting
        let formatted = format_transcript(&raw_transcript, "smart", "normal", true, false, &custom);
        assert!(!formatted.is_empty());

        // 4. History persistence
        let entry = HistoryEntry {
            audio_available: false,
            audio_expires_at: None,
            command_input: None,
            kind: "dictation".into(),
            source: "dictation".into(),
            pinned: false,
            tags: String::new(),
            id: format!("stress-{}", cycle),
            created_at: chrono::Utc::now().to_rfc3339(),
            duration_ms: 1200,
            language: "en".into(),
            raw_transcript: raw_transcript.clone(),
            final_transcript: formatted.clone(),
            application_name: "StressTest".into(),
            application_process: "stress.exe".into(),
            word_count: 5,
            character_count: formatted.len(),
            model_version: "0.6B-test".into(),
            processing_mode: "smart".into(),
            smart_transcript: formatted,
            rewriter_used: false,
        };

        store
            .insert_entry(&entry)
            .expect("Insert entry failed in stress loop");
    }

    // Verify all 100 entries persisted correctly
    let entries = store.get_entries(150, 0).expect("Get entries failed");
    assert_eq!(entries.len(), 100);

    // Measure memory drift
    let final_metrics = PlatformSys::get_system_metrics();
    let final_app_ram = final_metrics.app_ram_mb;

    let drift_mb = (final_app_ram as i64) - (initial_app_ram as i64);
    println!(
        "Stress test completed: Initial RAM = {} MB, Final RAM = {} MB, Drift = {} MB",
        initial_app_ram, final_app_ram, drift_mb
    );

    // Drift must be < 50MB across 100 continuous dictations
    assert!(
        drift_mb < 50,
        "Memory leak detected: RSS drifted by {} MB (threshold 50 MB)",
        drift_mb
    );

    let _ = fs::remove_dir_all(&test_dir);
}
