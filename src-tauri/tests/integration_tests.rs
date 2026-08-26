use std::fs;
use reflow_lib::asr::{ASREngine, MockASREngine};
use reflow_lib::audio::{VadConfig, VoiceActivityDetector};
use reflow_lib::formatting::{format_transcript, CustomReplacements, ReplacementRule};
use reflow_lib::history::{HistoryEntry, HistoryStore};
use reflow_lib::settings::SettingsStore;

#[test]
fn test_sqlite_history_store_full_lifecycle() {
    let test_dir = std::env::temp_dir().join(format!("reflow_test_{}", uuid::Uuid::new_v4()));
    let db_path = test_dir.join("test_history.db");

    let store = HistoryStore::new(db_path.clone()).expect("Failed to create history store");

    // 1. Insert multiple entries
    let entry1 = HistoryEntry {
        id: "id-1".into(),
        created_at: "2026-08-21T10:00:00Z".into(),
        duration_ms: 3200,
        language: "en".into(),
        raw_transcript: "hello world".into(),
        final_transcript: "Hello world.".into(),
        application_name: "Visual Studio Code".into(),
        application_process: "Code.exe".into(),
        word_count: 2,
        character_count: 12,
        model_version: "0.6B-v1".into(),
        processing_mode: "smart".into(),
        smart_transcript: "Hello world.".into(),
        rewriter_used: false,
    };

    let entry2 = HistoryEntry {
        id: "id-2".into(),
        created_at: "2026-08-21T11:00:00Z".into(),
        duration_ms: 4500,
        language: "hi".into(),
        raw_transcript: "aaj humein deployment karna hai".into(),
        final_transcript: "Aaj humein deployment karna hai.".into(),
        application_name: "Slack".into(),
        application_process: "slack.exe".into(),
        word_count: 5,
        character_count: 32,
        model_version: "0.6B-v1".into(),
        processing_mode: "smart".into(),
        smart_transcript: "Aaj humein deployment karna hai.".into(),
        rewriter_used: true,
    };

    store.insert_entry(&entry1).expect("Insert entry 1 failed");
    store.insert_entry(&entry2).expect("Insert entry 2 failed");

    // 2. Query all entries
    let entries = store.get_entries(10, 0).expect("Get entries failed");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].id, "id-2"); // Latest first
    assert!(entries[0].rewriter_used);
    assert_eq!(entries[0].smart_transcript, "Aaj humein deployment karna hai.");
    assert!(!entries[1].rewriter_used);

    // 3. Search query
    let search_res = store.search_entries("deployment").expect("Search failed");
    assert_eq!(search_res.len(), 1);
    assert_eq!(search_res[0].id, "id-2");

    // 4. Delete single entry
    let del_res = store.delete_entry("id-1").expect("Delete item failed");
    assert!(del_res);
    let remaining = store.get_entries(10, 0).expect("Get entries failed");
    assert_eq!(remaining.len(), 1);

    // 5. Clear all
    let cleared = store.clear_all().expect("Clear all failed");
    assert_eq!(cleared, 1);
    assert_eq!(store.get_entries(10, 0).unwrap().len(), 0);

    // Clean up
    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_settings_store_persistence() {
    let test_dir = std::env::temp_dir().join(format!("reflow_test_cfg_{}", uuid::Uuid::new_v4()));
    let cfg_path = test_dir.join("test_settings.json");

    let store = SettingsStore::new(cfg_path.clone());
    let mut current = store.get();
    assert_eq!(current.hotkey, reflow_lib::platform::default_hotkey());

    current.hotkey = "Alt+Space".into();
    current.language = "bn".into();
    current.push_to_talk = false;

    store.update(current.clone()).expect("Update failed");

    // Re-instantiate from disk
    let store2 = SettingsStore::new(cfg_path);
    let loaded = store2.get();
    assert_eq!(loaded.hotkey, "Alt+Space");
    assert_eq!(loaded.language, "bn");
    assert!(!loaded.push_to_talk);
    assert!(!loaded.api_enabled);
    assert_eq!(loaded.api_port, 7840);

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_legacy_settings_json_defaults_api_fields() {
    let mut value = serde_json::to_value(reflow_lib::settings::AppSettings::default()).unwrap();
    let obj = value.as_object_mut().unwrap();
    obj.remove("api_enabled");
    obj.remove("api_bind");
    obj.remove("api_port");
    obj.remove("api_mdns");
    obj.remove("api_inject_default");
    let loaded: reflow_lib::settings::AppSettings = serde_json::from_value(value).unwrap();
    assert!(!loaded.api_enabled);
    assert_eq!(loaded.api_port, 7840);
    assert_eq!(loaded.api_bind, "lan");
    assert_eq!(loaded.cleanup_level, "light");
    assert!(!loaded.developer_mode);
}

#[test]
fn test_default_settings_are_tray_first() {
    let s = reflow_lib::settings::AppSettings::default();
    assert!(!s.developer_mode);
    assert_eq!(s.cleanup_level, "light");
    assert_eq!(s.style, "neutral");
    assert_eq!(s.resolved_cleanup_level(), "light");
}

#[test]
fn test_vad_speech_and_silence_detection() {
    let mut vad = VoiceActivityDetector::new(
        VadConfig {
            energy_threshold: 0.02,
            silence_timeout_ms: 300, // 300ms = 4800 samples at 16k
            pre_roll_ms: 100,
            post_roll_ms: 100,
        },
        16000,
    );

    // Feed silence
    let silence = vec![0.001f32; 1600]; // 100ms
    let (is_speech, auto_stop, _) = vad.process_chunk(&silence);
    assert!(!is_speech);
    assert!(!auto_stop);

    // Feed loud speech
    let speech = vec![0.1f32; 1600]; // 100ms
    let (is_speech_2, auto_stop_2, rms) = vad.process_chunk(&speech);
    assert!(is_speech_2);
    assert!(!auto_stop_2);
    assert!(rms > 0.05);

    // Feed prolonged silence to trigger auto_stop
    let long_silence = vec![0.0001f32; 6000]; // ~375ms
    let (_, auto_stop_3, _) = vad.process_chunk(&long_silence);
    assert!(auto_stop_3);
}

#[test]
fn test_mock_asr_streaming_simulation() {
    let mut engine = MockASREngine::new();
    engine.initialize().unwrap();
    assert!(engine.is_model_loaded());

    engine.start_stream("auto", &[]).unwrap();
    let audio_chunk = vec![0.05f32; 3200]; // 200ms

    let partial_1 = engine.push_audio(&audio_chunk).unwrap();
    assert!(partial_1.is_some());

    let final_res = engine.stop_stream().unwrap();
    assert!(!final_res.is_empty());
}

#[test]
fn test_mixed_language_and_jargon_formatting() {
    let rules = vec![
        ReplacementRule {
            id: "1".into(),
            before: "git hub".into(),
            after: "GitHub".into(),
            enabled: true,
        },
        ReplacementRule {
            id: "2".into(),
            before: "api".into(),
            after: "API".into(),
            enabled: true,
        },
    ];
    let custom = CustomReplacements::new(rules);

    // Bengali mixed with English technical vocabulary
    let raw_bn = "um ajke amader git hub api ta deploy korte hobe";
    let formatted_bn = format_transcript(raw_bn, "smart", "normal", true, false, &custom);
    assert_eq!(formatted_bn, "Ajke amader GitHub API ta deploy korte hobe.");

    // Hindi mixed with English technical vocabulary
    let raw_hi = "kya aap git hub repo check kar sakte ho";
    let formatted_hi = format_transcript(raw_hi, "smart", "normal", true, false, &custom);
    assert_eq!(formatted_hi, "Kya aap GitHub repo check kar sakte ho?");
}
