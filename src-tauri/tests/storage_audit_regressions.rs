use reflow_lib::expansion_commands::{correction_suggestions, note_entry};
use reflow_lib::file_jobs::TranscriptSegment;
use reflow_lib::history::{encryption::HistoryKeyProvider, HistoryStore, RetentionCleaner};
use reflow_lib::settings::{AppSettings, Mode, OutputAction};
use reflow_lib::transfer::{bundle_patch, make_bundle};
use std::{path::PathBuf, sync::Arc};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("reflow-storage-audit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }
    fn store(&self) -> HistoryStore {
        HistoryStore::new(self.0.join("history.db")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn storage_audit_automatic_retention_preserves_saved_notes_and_explicit_clear_removes_them() {
    for policy in ["disabled", "1_day", "7_days", "30_days", "90_days"] {
        let fixture = Fixture::new();
        let store = fixture.store();
        for (id, kind, source) in [
            ("manual-note", "note", "manual"),
            ("voice-note", "note", "dictation"),
            ("dictation", "dictation", "dictation"),
            ("import", "file", "file"),
        ] {
            let mut row = note_entry(format!("Saved {id}"));
            row.id = id.into();
            row.kind = kind.into();
            row.source = source.into();
            row.created_at = (chrono::Utc::now() - chrono::Duration::days(100)).to_rfc3339();
            store.insert_entry(&row).unwrap();
        }
        assert_eq!(
            RetentionCleaner::apply_retention(&store, policy).unwrap(),
            2,
            "{policy}"
        );
        assert!(
            store.get_entry("manual-note").unwrap().is_some(),
            "{policy}"
        );
        assert!(store.get_entry("voice-note").unwrap().is_some(), "{policy}");
        assert!(store.get_entry("dictation").unwrap().is_none(), "{policy}");
        assert_eq!(store.clear_all().unwrap(), 2);
    }
}

struct MissingKey;
impl HistoryKeyProvider for MissingKey {
    fn load_key(&self, _: bool) -> Result<[u8; 32], String> {
        Err("Fixture key unavailable".into())
    }
}

struct FixtureKey;
impl HistoryKeyProvider for FixtureKey {
    fn load_key(&self, _: bool) -> Result<[u8; 32], String> {
        Ok([7; 32])
    }
}

#[test]
fn storage_audit_recovery_reopens_saved_history_without_touching_the_original() {
    let fixture = Fixture::new();
    let original = fixture.0.join("history.db");
    std::fs::write(&original, b"damaged original fixture").unwrap();
    let store =
        HistoryStore::open_recovering_with_key_provider(original.clone(), Arc::new(MissingKey));
    store
        .insert_entry(&note_entry("Recovery note".into()))
        .unwrap();
    drop(store);
    let reopened =
        HistoryStore::open_recovering_with_key_provider(original.clone(), Arc::new(MissingKey));
    assert_eq!(
        reopened.get_entries(10, 0).unwrap()[0].final_transcript,
        "Recovery note"
    );
    assert_eq!(
        std::fs::read(original).unwrap(),
        b"damaged original fixture"
    );
}

#[test]
fn storage_audit_missing_key_recovery_reopens_explicitly_readable_notes() {
    let fixture = Fixture::new();
    let original = fixture.0.join("history.db");
    let store =
        HistoryStore::new_with_key_provider(original.clone(), Arc::new(FixtureKey)).unwrap();
    store
        .insert_entry(&note_entry("Original encrypted note".into()))
        .unwrap();
    store.set_encryption(true).unwrap();
    drop(store);
    let original_bytes = std::fs::read(&original).unwrap();
    let recovered =
        HistoryStore::open_recovering_with_key_provider(original.clone(), Arc::new(MissingKey));
    recovered.require_encryption(true);
    assert!(recovered
        .insert_entry(&note_entry("Blocked private write".into()))
        .is_err());
    recovered.require_encryption(false);
    recovered
        .insert_entry(&note_entry("Explicit recovery note".into()))
        .unwrap();
    drop(recovered);
    let reopened =
        HistoryStore::open_recovering_with_key_provider(original.clone(), Arc::new(MissingKey));
    assert_eq!(
        reopened.get_entries(10, 0).unwrap()[0].final_transcript,
        "Explicit recovery note"
    );
    assert_eq!(std::fs::read(original).unwrap(), original_bytes);
}

#[test]
fn storage_audit_recovery_resumes_a_legacy_uuid_store() {
    let fixture = Fixture::new();
    let original = fixture.0.join("history.db");
    std::fs::write(&original, b"damaged original fixture").unwrap();
    let legacy = fixture
        .0
        .join(format!("history-recovery-{}.db", uuid::Uuid::new_v4()));
    let store = HistoryStore::new(legacy).unwrap();
    store
        .insert_entry(&note_entry("Previously saved recovery note".into()))
        .unwrap();
    drop(store);
    for _ in 0..2 {
        let recovered =
            HistoryStore::open_recovering_with_key_provider(original.clone(), Arc::new(MissingKey));
        assert_eq!(
            recovered.get_entries(10, 0).unwrap()[0].final_transcript,
            "Previously saved recovery note"
        );
    }
}

#[test]
fn storage_audit_import_preserves_privileged_modes_and_cannot_activate_them() {
    for action in [
        OutputAction::RunCommand {
            template: "trusted local tool".into(),
        },
        OutputAction::AppendFile {
            path: "local-output.txt".into(),
        },
    ] {
        let mut current = AppSettings {
            mode_triggers_enabled: false,
            ..Default::default()
        };
        current.modes.push(Mode {
            id: "local-tool".into(),
            name: "Local tool".into(),
            enabled: false,
            output: action,
            ..Default::default()
        });
        let mut bundle = make_bundle(&current).unwrap();
        let imported = bundle.modes.last_mut().unwrap();
        imported.enabled = true;
        imported.hotkey = Some("Ctrl+Shift+K".into());
        imported.triggers.apps = vec!["editor".into()];
        imported.custom_instructions = "Use imported instructions".into();
        bundle.settings["default_mode_id"] = "local-tool".into();
        bundle.settings["mode_triggers_enabled"] = true.into();
        for replace in [false, true] {
            let patch = bundle_patch(&current, &bundle, replace).unwrap();
            assert_eq!(
                patch["modes"].as_array().unwrap().last().unwrap(),
                &serde_json::to_value(current.modes.last().unwrap()).unwrap()
            );
            assert_eq!(patch["default_mode_id"], current.default_mode_id);
            assert_eq!(patch["mode_triggers_enabled"], false);
        }
    }
}

#[test]
fn storage_audit_import_still_updates_safe_default_and_trigger_preferences() {
    let current = AppSettings::default();
    let mut bundle = make_bundle(&current).unwrap();
    bundle.settings["default_mode_id"] = "email".into();
    bundle.settings["mode_triggers_enabled"] = false.into();
    let patch = bundle_patch(&current, &bundle, false).unwrap();
    assert_eq!(patch["default_mode_id"], "email");
    assert_eq!(patch["mode_triggers_enabled"], false);
}

#[test]
fn storage_audit_edit_and_retry_invalidate_old_subtitle_text() {
    for (encrypted, retry) in [(false, false), (false, true), (true, false), (true, true)] {
        let fixture = Fixture::new();
        let store =
            HistoryStore::new_with_key_provider(fixture.0.join("history.db"), Arc::new(FixtureKey))
                .unwrap();
        store.set_encryption(encrypted).unwrap();
        let mut row = note_entry("Old subtitle".into());
        row.id = "file".into();
        row.kind = "file".into();
        row.source = "file".into();
        store.insert_entry(&row).unwrap();
        store
            .save_file_segments(
                "file",
                &[TranscriptSegment {
                    text: "Old subtitle".into(),
                    offset_ms: 0,
                    end_ms: 1000,
                    speakers: None,
                }],
            )
            .unwrap();
        if retry {
            store
                .replace_transcript(
                    "file",
                    "New recognition",
                    "New recognition",
                    "New translation",
                    true,
                    "fr",
                )
                .unwrap();
        } else {
            store
                .update_transcript("file", "Old subtitle", "Edited subtitle", false)
                .unwrap();
        }
        assert!(store.file_segments("file").unwrap().is_empty());
        assert_ne!(
            store.get_entry("file").unwrap().unwrap().final_transcript,
            "Old subtitle"
        );
    }
}

#[test]
fn storage_audit_failed_subtitle_invalidation_rolls_back_edits_and_retries() {
    for retry in [false, true] {
        let fixture = Fixture::new();
        let store = fixture.store();
        let mut row = note_entry("Original transcript".into());
        row.id = "file".into();
        store.insert_entry(&row).unwrap();
        store
            .save_file_segments(
                "file",
                &[TranscriptSegment {
                    text: "Original transcript".into(),
                    offset_ms: 0,
                    end_ms: 1000,
                    speakers: None,
                }],
            )
            .unwrap();
        let conn = rusqlite::Connection::open(fixture.0.join("history.db")).unwrap();
        conn.execute_batch("CREATE TRIGGER block_subtitle_invalidation BEFORE DELETE ON file_segments BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
        let result = if retry {
            store.replace_transcript(
                "file",
                "New recognition",
                "New recognition",
                "New translation",
                true,
                "fr",
            )
        } else {
            store.update_transcript("file", "Original transcript", "Edited transcript", false)
        };
        assert!(result.is_err());
        assert_eq!(
            store.get_entry("file").unwrap().unwrap().final_transcript,
            "Original transcript"
        );
        assert_eq!(
            store.file_segments("file").unwrap()[0].text,
            "Original transcript"
        );
    }
}

#[test]
fn storage_audit_correction_learning_skips_tokens_exceeding_dictionary_limits() {
    let before = "a".repeat(513);
    let after = format!("{}b", "a".repeat(512));
    assert!(correction_suggestions(&before, &after).is_empty());
    let before = "a".repeat(100_000);
    let after = format!("{}b", "a".repeat(99_999));
    assert!(correction_suggestions(&before, &after).is_empty());
    assert_eq!(correction_suggestions("recieve", "receive").len(), 1);
}
