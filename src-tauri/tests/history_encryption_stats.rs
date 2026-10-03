use reflow_lib::expansion_commands::note_entry;
use reflow_lib::file_jobs::TranscriptSegment;
use reflow_lib::history::db::HistoryQuery;
use reflow_lib::history::{encryption::HistoryKeyProvider, HistoryEntry, HistoryStore};
use std::sync::Arc;

struct TestKey(Option<[u8; 32]>);
impl HistoryKeyProvider for TestKey {
    fn load_key(&self, _: bool) -> Result<[u8; 32], String> {
        self.0.ok_or("Test history key is missing".into())
    }
}
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("reflow-encrypted-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir.join("history.db"))
    }
    fn open(&self) -> HistoryStore {
        HistoryStore::new_with_key_provider(self.0.clone(), Arc::new(TestKey(Some([7; 32]))))
            .unwrap()
    }
    fn connection(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(&self.0).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}
fn entry(id: &str, text: &str) -> HistoryEntry {
    let mut entry = note_entry(text.into());
    entry.id = id.into();
    entry.source = "dictation".into();
    entry.kind = "dictation".into();
    entry.duration_ms = 60_000;
    entry.application_process = "editor.exe".into();
    entry.application_name = "Editor".into();
    entry
}

#[test]
fn encryption_migrates_every_private_surface_and_reopens_with_existing_key() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let secret = "ultra_private_transcript_独特";
    let mut row = entry("private", secret);
    row.command_input = Some("ultra_private_selection_独特".into());
    store.insert_entry(&row).unwrap();
    store.set_audio_retention("forever").unwrap();
    let samples = vec![0.25, -0.25, 0.5];
    store.save_audio("private", &samples).unwrap();
    store
        .save_file_segments(
            "private",
            &[TranscriptSegment {
                text: "ultra_private_subtitle_独特".into(),
                offset_ms: 0,
                end_ms: 1000,
                speakers: None,
            }],
        )
        .unwrap();
    store.set_encryption(true).unwrap();

    let conn = fixture.connection();
    let (raw, smart, final_text, command): (String, String, String, String) = conn
        .query_row(
            "SELECT raw_transcript,smart_transcript,final_transcript,command_input FROM history",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert!(!raw.contains(secret));
    assert_ne!(raw, smart, "each field needs a fresh nonce");
    assert_ne!(smart, final_text);
    assert!(!command.contains("ultra_private_selection"));
    let segments: String = conn
        .query_row("SELECT segments FROM file_segments", [], |r| r.get(0))
        .unwrap();
    assert!(!segments.contains("ultra_private_subtitle"));
    let pcm: Vec<u8> = conn
        .query_row("SELECT pcm FROM history_audio", [], |r| r.get(0))
        .unwrap();
    let plain_pcm: Vec<_> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    assert_ne!(pcm, plain_pcm);
    assert!(pcm.len() >= plain_pcm.len() + 28);
    drop(conn);
    drop(store);

    let bytes = std::fs::read(&fixture.0).unwrap();
    for secret in [
        secret,
        "ultra_private_selection_独特",
        "ultra_private_subtitle_独特",
    ] {
        assert!(
            !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
            "plaintext remained in SQLite: {secret}"
        );
    }
    let reopened = fixture.open();
    assert!(reopened.encryption_enabled());
    assert_eq!(
        reopened
            .get_entry("private")
            .unwrap()
            .unwrap()
            .final_transcript,
        row.final_transcript
    );
    assert_eq!(
        reopened
            .get_entry("private")
            .unwrap()
            .unwrap()
            .command_input,
        row.command_input
    );
    assert_eq!(reopened.load_audio("private").unwrap(), samples);
    assert_eq!(
        reopened.file_segments("private").unwrap()[0].text,
        "ultra_private_subtitle_独特"
    );
    reopened.set_encryption(false).unwrap();
    drop(reopened);
    assert!(!fixture.open().encryption_enabled());
}

#[test]
fn enabling_encryption_removes_old_plaintext_wal_frames() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let conn = fixture.connection();
    conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    let secret = "private_plaintext_in_old_wal_frames";
    store.insert_entry(&entry("wal", secret)).unwrap();
    let wal = fixture.0.with_file_name("history.db-wal");
    let bytes = std::fs::read(&wal).unwrap();
    assert!(bytes
        .windows(secret.len())
        .any(|value| value == secret.as_bytes()));
    drop(conn);

    store.set_encryption(true).unwrap();
    for path in [&fixture.0, &wal] {
        if let Ok(bytes) = std::fs::read(path) {
            assert!(
                !bytes
                    .windows(secret.len())
                    .any(|value| value == secret.as_bytes()),
                "plaintext survived encryption in {}",
                path.display()
            );
        }
    }
}

#[test]
fn missing_or_wrong_key_preserves_encrypted_original_byte_for_byte() {
    let fixture = Fixture::new();
    let store = fixture.open();
    store
        .insert_entry(&entry("private", "Do not lose this transcript"))
        .unwrap();
    store.set_encryption(true).unwrap();
    drop(store);
    let original = std::fs::read(&fixture.0).unwrap();
    for key in [None, Some([8; 32])] {
        let opened = HistoryStore::new_with_key_provider(fixture.0.clone(), Arc::new(TestKey(key)));
        assert!(opened.is_err());
        assert_eq!(std::fs::read(&fixture.0).unwrap(), original);
    }
    assert_eq!(
        fixture
            .open()
            .get_entry("private")
            .unwrap()
            .unwrap()
            .final_transcript,
        "Do not lose this transcript"
    );
}

#[test]
fn failed_decryption_migration_rolls_back_transcripts_and_manifest() {
    let fixture = Fixture::new();
    let store = fixture.open();
    store
        .insert_entry(&entry("private", "Preserve the encrypted transcript"))
        .unwrap();
    store
        .save_file_segments(
            "private",
            &[TranscriptSegment {
                text: "Subtitle".into(),
                offset_ms: 0,
                end_ms: 1,
                speakers: None,
            }],
        )
        .unwrap();
    store.set_encryption(true).unwrap();
    let conn = fixture.connection();
    let raw: String = conn
        .query_row("SELECT raw_transcript FROM history", [], |r| r.get(0))
        .unwrap();
    conn.execute("UPDATE file_segments SET segments='damaged ciphertext'", [])
        .unwrap();
    assert!(store.set_encryption(false).is_err());
    assert!(store.encryption_enabled());
    let still_encrypted: String = conn
        .query_row("SELECT raw_transcript FROM history", [], |r| r.get(0))
        .unwrap();
    assert_eq!(still_encrypted, raw);
    let flag: String = conn
        .query_row(
            "SELECT value FROM history_settings WHERE key='encryption'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(flag, "1");
}

#[test]
fn authenticated_values_cannot_be_swapped_between_transcript_columns() {
    let fixture = Fixture::new();
    let store = fixture.open();
    store.set_encryption(true).unwrap();
    store
        .insert_entry(&entry("private", "Text protected from swapping"))
        .unwrap();
    fixture
        .connection()
        .execute("UPDATE history SET final_transcript=raw_transcript", [])
        .unwrap();
    assert!(store.get_entry("private").is_err());
}

#[test]
fn failed_enabling_is_transactional_and_keeps_plaintext_state() {
    let fixture = Fixture::new();
    let store = fixture.open();
    store
        .insert_entry(&entry("private", "Keep the original on migration failure"))
        .unwrap();
    fixture.connection().execute_batch("CREATE TRIGGER reject_encryption BEFORE UPDATE ON history BEGIN SELECT RAISE(ABORT, 'injected migration failure'); END;").unwrap();
    assert!(store.set_encryption(true).is_err());
    assert!(!store.encryption_enabled());
    assert_eq!(
        store
            .get_entry("private")
            .unwrap()
            .unwrap()
            .final_transcript,
        "Keep the original on migration failure"
    );
    drop(store);
    assert!(!fixture.open().encryption_enabled());
}

#[test]
fn missing_key_recovery_never_replaces_the_original_or_creates_a_key() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct MissingKey(Arc<AtomicUsize>);
    impl HistoryKeyProvider for MissingKey {
        fn load_key(&self, create: bool) -> Result<[u8; 32], String> {
            if create {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
            Err("Original history key missing".into())
        }
    }
    let fixture = Fixture::new();
    let store = fixture.open();
    store
        .insert_entry(&entry("private", "Keep the original encrypted database"))
        .unwrap();
    store.set_encryption(true).unwrap();
    drop(store);
    let original = std::fs::read(&fixture.0).unwrap();
    let creates = Arc::new(AtomicUsize::new(0));
    let recovered = HistoryStore::open_recovering_with_key_provider(
        fixture.0.clone(),
        Arc::new(MissingKey(creates.clone())),
    );
    assert!(recovered.recovery_notice().is_some());
    assert!(recovered.set_encryption(true).is_err());
    assert_eq!(creates.load(Ordering::SeqCst), 0);
    recovered
        .insert_entry(&entry("new", "New separate recovery transcript"))
        .unwrap();
    assert_eq!(std::fs::read(&fixture.0).unwrap(), original);
    assert!(fixture.open().get_entry("new").unwrap().is_none());
}

#[test]
fn encrypted_edits_retries_search_and_pagination_return_plaintext_to_callers() {
    let fixture = Fixture::new();
    let store = fixture.open();
    store.set_encryption(true).unwrap();
    for id in ["one", "two"] {
        store.insert_entry(&entry(id, "original")).unwrap();
        store
            .replace_transcript(id, "RAW", "SMART", "edited café 100%_literal", true, "fr")
            .unwrap();
    }
    store
        .update_transcript("two", "SMARTER", "edited café 100%_literal twice", false)
        .unwrap();
    let query = HistoryQuery {
        query: "100%_literal".into(),
        limit: Some(1),
        ..Default::default()
    };
    let page = store.query_entries(&query).unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.next_offset, Some(1));
    let second = store
        .query_entries(&HistoryQuery {
            offset: page.next_offset,
            ..query
        })
        .unwrap();
    assert_ne!(page.entries[0].id, second.entries[0].id);
    assert_eq!(store.search_entries("CAFÉ").unwrap().len(), 2);
    let empty_search = store.search_entries("").unwrap();
    assert!(empty_search
        .iter()
        .all(|row| row.final_transcript.starts_with("edited café")));
    assert_eq!(store.get_entries(10, 0).unwrap().len(), 2);
    let value: String = fixture
        .connection()
        .query_row(
            "SELECT final_transcript FROM history WHERE id='two'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!value.contains("edited café"));
}

#[test]
fn usage_stats_exclude_manual_notes_and_files_and_fill_thirty_day_series() {
    let fixture = Fixture::new();
    let store = fixture.open();
    for (id, date, words) in [
        ("today", "2026-10-02T12:00:00Z", 80),
        ("yesterday", "2026-10-01T12:00:00Z", 40),
        ("older", "2026-09-29T12:00:00Z", 20),
    ] {
        let mut row = entry(id, "dictated words");
        row.created_at = date.into();
        row.word_count = words;
        row.character_count = words * 5;
        store.insert_entry(&row).unwrap();
    }
    let manual = note_entry("manual note".into());
    store.insert_entry(&manual).unwrap();
    let mut imported = entry("import", "imported file");
    imported.source = "file".into();
    imported.kind = "file".into();
    store.insert_entry(&imported).unwrap();
    let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
    let stats = store.usage_stats_at(today).unwrap();
    assert_eq!(stats.total_words, 140);
    assert_eq!(stats.total_characters, 700);
    assert_eq!(stats.dictations, 3);
    assert_eq!(stats.minutes_spoken, 3.0);
    assert_eq!(stats.time_saved_minutes, 3.5);
    assert_eq!(stats.streak_days, 2);
    assert_eq!(stats.per_app.len(), 1);
    assert_eq!(stats.per_app[0].process, "editor.exe");
    assert_eq!(stats.per_day.len(), 30);
    assert_eq!(stats.per_day.last().unwrap().words, 80);
    store.set_encryption(true).unwrap();
    assert_eq!(store.usage_stats_at(today).unwrap().total_words, 140);
}

#[test]
fn requested_encryption_blocks_private_writes_in_missing_key_recovery() {
    let fixture = Fixture::new();
    {
        let store = fixture.open();
        store.set_encryption(true).unwrap();
        store
            .insert_entry(&entry("old", "original secret"))
            .unwrap();
    }
    let recovered =
        HistoryStore::open_recovering_with_key_provider(fixture.0.clone(), Arc::new(TestKey(None)));
    recovered.require_encryption(true);
    assert!(recovered
        .insert_entry(&entry("new", "new secret"))
        .unwrap_err()
        .contains("not saved"));
    assert_eq!(recovered.get_entries(10, 0).unwrap().len(), 0);
    recovered.require_encryption(false);
    recovered
        .insert_entry(&entry("new", "explicitly readable"))
        .unwrap();
    assert_eq!(recovered.get_entries(10, 0).unwrap().len(), 1);
}

#[test]
fn failed_encryption_setup_also_blocks_retranscription_of_existing_plaintext() {
    let fixture = Fixture::new();
    let store =
        HistoryStore::new_with_key_provider(fixture.0.clone(), Arc::new(TestKey(None))).unwrap();
    store
        .insert_entry(&entry("existing", "original text"))
        .unwrap();
    store.require_encryption(true);
    assert!(store.set_encryption(true).is_err());
    assert!(store
        .replace_transcript(
            "existing",
            "new private raw",
            "new private smart",
            "new private final",
            true,
            "en"
        )
        .is_err());
    assert_eq!(
        store
            .get_entry("existing")
            .unwrap()
            .unwrap()
            .final_transcript,
        "original text"
    );
}
