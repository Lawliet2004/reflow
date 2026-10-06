use reflow_lib::context::apply_startup_history_preferences;
use reflow_lib::expansion_commands::note_entry;
use reflow_lib::file_jobs::TranscriptSegment;
use reflow_lib::history::{encryption::HistoryKeyProvider, HistoryStore};
use reflow_lib::settings::{AppSettings, SettingsStore};
use std::{path::PathBuf, sync::Arc};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("reflow-startup-recovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }
    fn open(&self, key: Option<[u8; 32]>) -> HistoryStore {
        HistoryStore::new_with_key_provider(self.0.join("history.db"), Arc::new(TestKey(key)))
            .unwrap()
    }
    fn saved_history(&self) {
        let store = self.open(Some([7; 32]));
        let mut row = note_entry("private old transcript".into());
        row.id = "old".into();
        row.kind = "dictation".into();
        row.created_at = (chrono::Utc::now() - chrono::Duration::days(100)).to_rfc3339();
        store.insert_entry(&row).unwrap();
        store.set_audio_retention("forever").unwrap();
        store.save_audio("old", &[0.25, -0.5]).unwrap();
        store
            .save_file_segments(
                "old",
                &[TranscriptSegment {
                    text: "private old subtitle".into(),
                    offset_ms: 0,
                    end_ms: 1000,
                    speakers: None,
                }],
            )
            .unwrap();
        store.set_encryption(true).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct TestKey(Option<[u8; 32]>);
impl HistoryKeyProvider for TestKey {
    fn load_key(&self, _: bool) -> Result<[u8; 32], String> {
        self.0.ok_or_else(|| "Fixture key unavailable".into())
    }
}

#[test]
fn missing_or_invalid_settings_preserve_encrypted_history_audio_and_subtitles() {
    for bad_config in [
        None,
        Some("broken JSON"),
        Some(r#"{"settings_version":1,"audio_retention":false}"#),
    ] {
        let fixture = Fixture::new();
        fixture.saved_history();
        let config_path = fixture.0.join("settings.json");
        if let Some(content) = bad_config {
            std::fs::write(&config_path, content).unwrap();
        }
        let settings = SettingsStore::new(config_path.clone());
        let store = fixture.open(Some([7; 32]));
        let active = apply_startup_history_preferences(&settings, &store, true);
        assert!(active.history_encryption);
        assert_eq!(active.history_retention, "indefinite");
        assert_eq!(active.audio_retention, "disabled");
        assert!(settings.recovery_notice().is_some());
        assert!(store.encryption_enabled());
        assert_eq!(
            store.get_entry("old").unwrap().unwrap().final_transcript,
            "private old transcript"
        );
        assert_eq!(store.load_audio("old").unwrap(), vec![0.25, -0.5]);
        assert_eq!(
            store.file_segments("old").unwrap()[0].text,
            "private old subtitle"
        );
        match bad_config {
            Some(content) => assert_eq!(std::fs::read_to_string(&config_path).unwrap(), content),
            None => assert!(!config_path.exists()),
        }

        // An unrelated save must persist the preserved privacy preferences.
        let active = settings
            .merge_update(serde_json::json!({"language":"en"}))
            .unwrap();
        assert!(active.history_encryption);
        assert_eq!(active.history_retention, "indefinite");
        assert!(
            SettingsStore::new(config_path.clone())
                .get()
                .history_encryption
        );
        store.require_encryption(active.history_encryption);
        store
            .insert_entry(&note_entry("new private note".into()))
            .unwrap();
        assert!(store.encryption_enabled());
        assert_eq!(store.load_audio("old").unwrap(), vec![0.25, -0.5]);

        // The recovered defaults persisted by that save also remain harmless
        // after a restart, when the settings document is valid again.
        drop(store);
        let settings = SettingsStore::new(config_path);
        let store = fixture.open(Some([7; 32]));
        let active = apply_startup_history_preferences(&settings, &store, true);
        assert!(active.history_encryption);
        assert_eq!(active.audio_retention, "disabled");
        assert_eq!(store.load_audio("old").unwrap(), vec![0.25, -0.5]);
        assert!(store.get_entry("old").unwrap().is_some());

        // Explicit user changes still decrypt and delete retained recordings.
        let active = settings
            .merge_update(serde_json::json!({
                "history_encryption":false,"audio_retention":"disabled"
            }))
            .unwrap();
        store.set_encryption(active.history_encryption).unwrap();
        store.require_encryption(active.history_encryption);
        store.set_audio_retention(&active.audio_retention).unwrap();
        assert!(!store.encryption_enabled());
        assert!(store.load_audio("old").is_err());
        assert!(store.get_entry("old").unwrap().is_some());
    }
}

#[test]
fn recovered_settings_cannot_allow_plaintext_writes_when_the_original_key_is_missing() {
    let fixture = Fixture::new();
    fixture.saved_history();
    let original_path = fixture.0.join("history.db");
    let original = std::fs::read(&original_path).unwrap();
    let store = HistoryStore::open_recovering_with_key_provider(
        original_path.clone(),
        Arc::new(TestKey(None)),
    );
    let settings = SettingsStore::new(fixture.0.join("settings.json"));
    let active = apply_startup_history_preferences(&settings, &store, true);
    assert!(active.history_encryption);
    assert!(store
        .insert_entry(&note_entry("must not save plaintext".into()))
        .is_err());
    assert_eq!(std::fs::read(original_path).unwrap(), original);
}

#[test]
fn recovery_with_conflicting_encryption_metadata_still_blocks_plaintext_writes() {
    let fixture = Fixture::new();
    fixture.saved_history();
    let original_path = fixture.0.join("history.db");
    let conn = rusqlite::Connection::open(&original_path).unwrap();
    conn.execute(
        "UPDATE history_settings SET value='0' WHERE key='encryption'",
        [],
    )
    .unwrap();
    drop(conn);
    let original = std::fs::read(&original_path).unwrap();
    let store = HistoryStore::open_recovering_with_key_provider(
        original_path.clone(),
        Arc::new(TestKey(Some([7; 32]))),
    );
    let settings = SettingsStore::new(fixture.0.join("settings.json"));
    let active = apply_startup_history_preferences(&settings, &store, true);
    assert!(active.history_encryption);
    assert!(store
        .insert_entry(&note_entry("must not save plaintext".into()))
        .is_err());
    assert_eq!(std::fs::read(original_path).unwrap(), original);
}

#[test]
fn a_valid_false_encryption_setting_does_not_decrypt_an_encrypted_manifest_on_startup() {
    let fixture = Fixture::new();
    fixture.saved_history();
    let config_path = fixture.0.join("settings.json");
    let config = AppSettings {
        history_encryption: false,
        history_retention: "indefinite".into(),
        audio_retention: "forever".into(),
        ..Default::default()
    };
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let settings = SettingsStore::new(config_path);
    let store = fixture.open(Some([7; 32]));
    let active = apply_startup_history_preferences(&settings, &store, true);
    assert!(active.history_encryption);
    assert!(store.encryption_enabled());
    assert_eq!(store.load_audio("old").unwrap(), vec![0.25, -0.5]);
    assert!(
        settings
            .merge_update(serde_json::json!({"language":"en"}))
            .unwrap()
            .history_encryption
    );
}

#[test]
fn valid_disabled_recording_settings_do_not_delete_existing_audio_on_startup() {
    let fixture = Fixture::new();
    fixture.saved_history();
    let config_path = fixture.0.join("settings.json");
    let config = AppSettings {
        history_encryption: true,
        history_retention: "indefinite".into(),
        audio_retention: "disabled".into(),
        ..Default::default()
    };
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let settings = SettingsStore::new(config_path);
    let store = fixture.open(Some([7; 32]));
    apply_startup_history_preferences(&settings, &store, true);
    assert_eq!(store.load_audio("old").unwrap(), vec![0.25, -0.5]);
    store.save_audio("old", &[0.75]).unwrap();
    assert_eq!(store.load_audio("old").unwrap(), vec![0.25, -0.5]);
}

#[test]
fn recovery_expires_only_recordings_whose_saved_deadline_passed() {
    let fixture = Fixture::new();
    fixture.saved_history();
    let conn = rusqlite::Connection::open(fixture.0.join("history.db")).unwrap();
    conn.execute(
        "UPDATE history_audio SET expires_at_ms=?1 WHERE history_id='old'",
        [chrono::Utc::now().timestamp_millis() - 1],
    )
    .unwrap();
    drop(conn);
    let settings = SettingsStore::new(fixture.0.join("settings.json"));
    let store = fixture.open(Some([7; 32]));
    apply_startup_history_preferences(&settings, &store, true);
    assert!(store.load_audio("old").is_err());
    assert!(store.get_entry("old").unwrap().is_some());
}

#[test]
fn a_fresh_install_keeps_normal_privacy_defaults() {
    let fixture = Fixture::new();
    let settings = SettingsStore::new(fixture.0.join("settings.json"));
    let store = fixture.open(Some([7; 32]));
    let active = apply_startup_history_preferences(&settings, &store, false);
    assert!(!active.history_encryption);
    assert_eq!(active.history_retention, "30_days");
    assert_eq!(active.audio_retention, "disabled");
    assert!(settings.recovery_notice().is_none());
}

#[test]
fn future_settings_schemas_preserve_unknown_fields_and_reject_all_saves() {
    for version in [
        reflow_lib::settings::CURRENT_SETTINGS_VERSION as u64 + 1,
        u64::MAX,
    ] {
        let fixture = Fixture::new();
        fixture.saved_history();
        let config_path = fixture.0.join("settings.json");
        let mut config = serde_json::to_value(AppSettings::default()).unwrap();
        config["settings_version"] = version.into();
        config["future_private_preference"] = serde_json::json!({"must":"survive"});
        let original = serde_json::to_vec_pretty(&config).unwrap();
        std::fs::write(&config_path, &original).unwrap();
        let settings = SettingsStore::new(config_path.clone());
        let store = fixture.open(Some([7; 32]));
        let active = apply_startup_history_preferences(&settings, &store, true);
        assert!(active.history_encryption);
        assert_eq!(active.history_retention, "indefinite");
        assert_eq!(store.load_audio("old").unwrap(), vec![0.25, -0.5]);
        let error = settings
            .merge_update(serde_json::json!({"language":"en"}))
            .unwrap_err();
        assert!(error.contains("newer Reflow"));
        assert!(settings
            .update(active)
            .unwrap_err()
            .contains("newer Reflow"));
        assert_eq!(std::fs::read(&config_path).unwrap(), original);
        assert_eq!(settings.get().language, "auto");
    }
}
