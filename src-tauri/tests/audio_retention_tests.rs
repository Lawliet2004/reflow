use reflow_lib::history::{HistoryEntry, HistoryStore};

fn entry(id: &str, created_at: chrono::DateTime<chrono::Utc>) -> HistoryEntry {
    HistoryEntry {
        id: id.into(),
        created_at: created_at.to_rfc3339(),
        duration_ms: 1000,
        language: "en".into(),
        raw_transcript: "Original".into(),
        smart_transcript: "Original".into(),
        final_transcript: "Cleaned".into(),
        rewriter_used: true,
        application_name: String::new(),
        application_process: String::new(),
        word_count: 1,
        character_count: 7,
        model_version: "test".into(),
        processing_mode: "light".into(),
        audio_available: false,
        audio_expires_at: None,
        command_input: None,
        kind: "dictation".into(),
        source: "dictation".into(),
        pinned: false,
        tags: String::new(),
    }
}

#[test]
fn audio_expiry_keeps_transcripts_and_cannot_be_reversed() {
    let path = std::env::temp_dir().join(format!("reflow-audio-{}", uuid::Uuid::new_v4()));
    let store = HistoryStore::new(path.join("history.db")).unwrap();
    let now = chrono::Utc::now();
    store.set_audio_retention("forever").unwrap();
    store
        .insert_entry(&entry("old", now - chrono::Duration::days(7)))
        .unwrap();
    store
        .insert_entry(&entry("recent", now - chrono::Duration::hours(23)))
        .unwrap();
    store.save_audio("old", &[0.25, -0.25]).unwrap();
    store.save_audio("recent", &[0.5, -0.5]).unwrap();
    assert!(store.get_entry("old").unwrap().unwrap().audio_available);
    store.set_audio_retention("7_days").unwrap();
    assert!(store.load_audio("old").is_err());
    let old = store.get_entry("old").unwrap().unwrap();
    assert!(!old.audio_available);
    assert_eq!(old.raw_transcript, "Original");
    assert_eq!(old.final_transcript, "Cleaned");
    assert_eq!(store.load_audio("recent").unwrap(), vec![0.5, -0.5]);
    store.set_audio_retention("forever").unwrap();
    assert!(store.load_audio("old").is_err());
    store.set_audio_retention("disabled").unwrap();
    assert!(store.load_audio("recent").is_err());
    assert_eq!(store.get_entries(10, 0).unwrap().len(), 2);
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn never_save_is_default_and_deleting_history_removes_audio() {
    let path = std::env::temp_dir().join(format!("reflow-audio-{}", uuid::Uuid::new_v4()));
    let store = HistoryStore::new(path.join("history.db")).unwrap();
    store
        .insert_entry(&entry("one", chrono::Utc::now()))
        .unwrap();
    store.save_audio("one", &[0.5]).unwrap();
    assert!(store.load_audio("one").is_err());
    for policy in ["1_day", "7_days", "30_days", "forever"] {
        store.set_audio_retention(policy).unwrap();
        store.save_audio("one", &[0.5]).unwrap();
        let saved = store.get_entry("one").unwrap().unwrap();
        assert!(saved.audio_available);
        assert_eq!(saved.audio_expires_at.is_none(), policy == "forever");
    }
    assert!(store.set_audio_retention("invalid").is_err());
    store.delete_entry("one").unwrap();
    assert!(store.load_audio("one").is_err());
    assert!(store.save_audio("one", &[0.5]).is_err());
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn expiry_is_enforced_after_restart_and_before_background_cleanup() {
    let path = std::env::temp_dir().join(format!("reflow-audio-{}", uuid::Uuid::new_v4()));
    let store = HistoryStore::new(path.join("history.db")).unwrap();
    store.set_audio_retention("1_day").unwrap();
    store
        .insert_entry(&entry("one", chrono::Utc::now()))
        .unwrap();
    store.save_audio("one", &[0.5]).unwrap();
    drop(store);
    let conn = rusqlite::Connection::open(path.join("history.db")).unwrap();
    conn.execute(
        "UPDATE history_audio SET expires_at_ms = ?1",
        [chrono::Utc::now().timestamp_millis()],
    )
    .unwrap();
    let store = HistoryStore::new(path.join("history.db")).unwrap();
    assert!(!store.get_entry("one").unwrap().unwrap().audio_available);
    assert!(store.load_audio("one").is_err());
    let count: i64 = conn
        .query_row("SELECT count(*) FROM history_audio", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        store.get_entry("one").unwrap().unwrap().final_transcript,
        "Cleaned"
    );
    drop(store);
    drop(conn);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn retry_recognizes_audio_without_replacing_history_or_injecting_text() {
    let path = std::env::temp_dir().join(format!("reflow-audio-{}", uuid::Uuid::new_v4()));
    let ctx = reflow_lib::context::AppContext::bootstrap_test(path.clone());
    ctx.history_store.set_audio_retention("7_days").unwrap();
    ctx.history_store
        .insert_entry(&entry("one", chrono::Utc::now()))
        .unwrap();
    ctx.history_store.save_audio("one", &[0.5; 32_000]).unwrap();
    let _operation = ctx.session_operation.lock().await;
    let result = reflow_lib::session::transcribe_saved_audio(&ctx, "one")
        .await
        .unwrap();
    assert!(result.starts_with("I need to finish this API"));
    assert_ne!(result, "Original");
    assert_eq!(ctx.history_store.get_entries(10, 0).unwrap().len(), 1);
    assert_eq!(
        ctx.history_store
            .get_entry("one")
            .unwrap()
            .unwrap()
            .final_transcript,
        "Cleaned"
    );
    ctx.history_store.set_audio_retention("disabled").unwrap();
    assert!(reflow_lib::session::transcribe_saved_audio(&ctx, "one")
        .await
        .is_err());
    drop(_operation);
    drop(ctx);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn extracted_wav_is_playable_and_existing_exports_are_preserved() {
    let path = std::env::temp_dir().join(format!("reflow-audio-{}", uuid::Uuid::new_v4()));
    let store = HistoryStore::new(path.join("history.db")).unwrap();
    store.set_audio_retention("7_days").unwrap();
    store
        .insert_entry(&entry("one", chrono::Utc::now()))
        .unwrap();
    store.save_audio("one", &[0.25, -0.5, 0.0]).unwrap();
    let output = path.join("audio.wav");
    store.export_audio("one", &output).unwrap();
    let mut reader = hound::WavReader::open(&output).unwrap();
    assert_eq!(reader.spec().sample_rate, 16_000);
    assert_eq!(reader.spec().channels, 1);
    assert_eq!(
        reader
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
        vec![8192, -16384, 0]
    );
    drop(reader);
    let original = std::fs::read(&output).unwrap();
    assert!(store.export_audio("one", &output).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), original);
    store.set_audio_retention("disabled").unwrap();
    assert!(store
        .export_audio("one", &path.join("expired.wav"))
        .is_err());
    assert!(!path.join("expired.wav").exists());
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn audio_retention_settings_survive_restart_and_invalid_values_are_rejected() {
    let path = std::env::temp_dir().join(format!("reflow-audio-settings-{}", uuid::Uuid::new_v4()));
    let store = reflow_lib::settings::SettingsStore::new(path.join("settings.json"));
    assert_eq!(store.get().audio_retention, "disabled");
    for policy in ["1_day", "7_days", "30_days", "forever", "disabled"] {
        store
            .merge_update(serde_json::json!({ "audio_retention": policy }))
            .unwrap();
        assert_eq!(
            reflow_lib::settings::SettingsStore::new(path.join("settings.json"))
                .get()
                .audio_retention,
            policy
        );
    }
    assert!(store
        .merge_update(serde_json::json!({ "audio_retention": "bad" }))
        .is_err());
    assert_eq!(store.get().audio_retention, "disabled");
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn session_keeps_the_whole_recording_beyond_the_previous_sixty_second_buffer() {
    let path = std::env::temp_dir().join(format!("reflow-audio-session-{}", uuid::Uuid::new_v4()));
    let ctx = reflow_lib::context::AppContext::bootstrap_test(path.clone());
    ctx.settings_store
        .merge_update(
            serde_json::json!({ "audio_retention": "7_days", "intelligence_tier": "raw_verbatim", "max_duration_sec": 0 }),
        )
        .unwrap();
    ctx.history_store.set_audio_retention("7_days").unwrap();
    reflow_lib::session::start_external(&ctx, Some("en".into()))
        .await
        .unwrap();
    // Feed 61 seconds without recording from a real device or injecting text.
    let sender = ctx.recording_sample_sender.read().clone().unwrap();
    for _ in 0..61 {
        sender.send(vec![0.25; 16_000]).await.unwrap();
    }
    drop(sender);
    reflow_lib::session::stop(&ctx, false).await.unwrap();
    let entries = ctx.history_store.get_entries(10, 0).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].audio_available);
    let samples = ctx.history_store.load_audio(&entries[0].id).unwrap();
    assert_eq!(samples.len(), 61 * 16_000);
    assert!(samples.iter().all(|sample| *sample == 0.25));
    drop(ctx);
    std::fs::remove_dir_all(path).unwrap();
}
