use super::encryption::{HistoryCipher, HistoryKeyProvider, OsHistoryKeyProvider};
use chrono::{Local, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    #[serde(default)]
    pub command_input: Option<String>,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default = "default_kind")]
    pub source: String,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub tags: String,
    pub id: String,
    pub created_at: String,
    pub duration_ms: u64,
    pub language: String,
    pub raw_transcript: String,
    #[serde(default)]
    pub smart_transcript: String,
    pub final_transcript: String,
    #[serde(default)]
    pub rewriter_used: bool,
    pub application_name: String,
    pub application_process: String,
    pub word_count: usize,
    pub character_count: usize,
    pub model_version: String,
    pub processing_mode: String,
    #[serde(default)]
    pub audio_available: bool,
    /// UTC expiry in milliseconds; None for recordings kept indefinitely.
    #[serde(default)]
    pub audio_expires_at: Option<i64>,
}

fn default_kind() -> String {
    "dictation".into()
}
pub const CURRENT_HISTORY_SCHEMA_VERSION: i32 = 6;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HistoryQuery {
    pub kind: Option<String>,
    pub application_process: Option<String>,
    pub pinned_only: Option<bool>,
    pub tag: Option<String>,
    #[serde(default)]
    pub query: String,
    pub from: Option<String>,
    pub until: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryPage {
    pub entries: Vec<HistoryEntry>,
    pub total: usize,
    pub next_offset: Option<usize>,
    pub recovery_notice: Option<String>,
}

pub struct HistoryStore {
    conn: Arc<Mutex<Connection>>,
    recovery_notice: Option<String>,
    audio_retention: Mutex<String>,
    cipher: Mutex<Option<HistoryCipher>>,
    key_provider: Arc<dyn HistoryKeyProvider>,
    encryption_required: std::sync::atomic::AtomicBool,
}

impl HistoryStore {
    pub fn require_encryption(&self, required: bool) {
        self.encryption_required
            .store(required, std::sync::atomic::Ordering::SeqCst);
    }
    fn ensure_private_writes(&self) -> Result<(), String> {
        if self
            .encryption_required
            .load(std::sync::atomic::Ordering::SeqCst)
            && !self.encryption_enabled()
        {
            return Err("Encrypted history is unavailable. New private history is not saved. Restore the OS encryption key or turn off Encrypt history to allow readable recovery history.".into());
        }
        Ok(())
    }
    pub fn recovery_notice(&self) -> Option<String> {
        self.recovery_notice.clone()
    }

    pub fn usage_stats(&self) -> Result<super::stats::UsageStats, String> {
        self.usage_stats_at(Local::now().date_naive())
    }

    pub fn usage_stats_at(
        &self,
        today: chrono::NaiveDate,
    ) -> Result<super::stats::UsageStats, String> {
        super::stats::aggregate(&self.conn.lock(), today)
    }

    pub fn encryption_enabled(&self) -> bool {
        self.cipher.lock().is_some()
    }

    fn decode_entry(&self, mut entry: HistoryEntry) -> Result<HistoryEntry, String> {
        if let Some(cipher) = self.cipher.lock().as_ref() {
            entry.raw_transcript =
                cipher.open_text(&entry.id, "raw_transcript", &entry.raw_transcript)?;
            entry.smart_transcript =
                cipher.open_text(&entry.id, "smart_transcript", &entry.smart_transcript)?;
            entry.final_transcript =
                cipher.open_text(&entry.id, "final_transcript", &entry.final_transcript)?;
            entry.command_input = entry
                .command_input
                .map(|text| cipher.open_text(&entry.id, "command_input", &text))
                .transpose()?;
        }
        Ok(entry)
    }

    fn encode_text(&self, id: &str, field: &str, text: &str) -> Result<String, String> {
        match self.cipher.lock().as_ref() {
            Some(cipher) => cipher.seal_text(id, field, text),
            None => Ok(text.to_owned()),
        }
    }

    /// Rewrite all sensitive columns together. Authentication or storage
    /// failures roll back the contents and the persisted encryption manifest.
    pub fn set_encryption(&self, enabled: bool) -> Result<(), String> {
        let conn = self.conn.lock();
        let mut current = self.cipher.lock();
        if current.is_some() == enabled {
            return Ok(());
        }
        if enabled && self.recovery_notice.is_some() {
            return Err("Recovery history cannot replace the original encryption key. Restore access to the original OS key first.".into());
        }
        let candidate = if enabled {
            Some(HistoryCipher::new(self.key_provider.load_key(true)?))
        } else {
            None
        };
        let cipher = candidate
            .as_ref()
            .or(current.as_ref())
            .expect("changing encryption state has a cipher");
        let transform = |id: &str, field: &str, text: String| {
            if enabled {
                cipher.seal_text(id, field, &text)
            } else {
                cipher.open_text(id, field, &text)
            }
        };
        // Old WAL frames may still contain plaintext even after every live row
        // is encrypted. Checkpoint/truncate them before rewriting, and use a
        // rollback journal that SQLite removes when this transaction commits.
        let busy: i64 = conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
            .map_err(|e| format!("Could not prepare history encryption: {e}"))?;
        if busy != 0 {
            return Err("History is in use by another database reader. Close it before changing encryption.".into());
        }
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
            .map_err(|e| format!("Could not prepare history encryption: {e}"))?;
        if !matches!(journal_mode.as_str(), "delete" | "memory") {
            return Err("Could not switch history to a safe migration journal".into());
        }
        let transaction = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let ids = transaction
            .prepare("SELECT id FROM history")
            .map_err(|e| e.to_string())?
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        for id in ids {
            let (raw, smart, final_text, command): (String, String, String, Option<String>) = transaction.query_row(
                "SELECT raw_transcript, smart_transcript, final_transcript, command_input FROM history WHERE id=?1", [&id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            ).map_err(|e| e.to_string())?;
            let command = command
                .map(|text| transform(&id, "command_input", text))
                .transpose()?;
            transaction.execute("UPDATE history SET raw_transcript=?2, smart_transcript=?3, final_transcript=?4, command_input=?5 WHERE id=?1", params![id, transform(&id, "raw_transcript", raw)?, transform(&id, "smart_transcript", smart)?, transform(&id, "final_transcript", final_text)?, command]).map_err(|e| e.to_string())?;
        }
        let ids = transaction
            .prepare("SELECT history_id FROM history_audio")
            .map_err(|e| e.to_string())?
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        for id in ids {
            let bytes: Vec<u8> = transaction
                .query_row(
                    "SELECT pcm FROM history_audio WHERE history_id=?1",
                    [&id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            let bytes = if enabled {
                cipher.seal(&id, "pcm", &bytes)?
            } else {
                cipher.open(&id, "pcm", &bytes)?
            };
            transaction
                .execute(
                    "UPDATE history_audio SET pcm=?2 WHERE history_id=?1",
                    params![id, bytes],
                )
                .map_err(|e| e.to_string())?;
        }
        let ids = transaction
            .prepare("SELECT history_id FROM file_segments")
            .map_err(|e| e.to_string())?
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        for id in ids {
            let text: String = transaction
                .query_row(
                    "SELECT segments FROM file_segments WHERE history_id=?1",
                    [&id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            transaction
                .execute(
                    "UPDATE file_segments SET segments=?2 WHERE history_id=?1",
                    params![id, transform(&id, "segments", text)?],
                )
                .map_err(|e| e.to_string())?;
        }
        transaction
            .execute(
                "UPDATE history_settings SET value=?1 WHERE key='encryption'",
                [if enabled { "1" } else { "0" }],
            )
            .map_err(|e| e.to_string())?;
        if enabled {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO history_settings VALUES ('verification', ?1)",
                    [cipher.seal_text(
                        "manifest",
                        "verification",
                        "reflow history encryption v1",
                    )?],
                )
                .map_err(|e| e.to_string())?;
        } else {
            transaction
                .execute("DELETE FROM history_settings WHERE key='verification'", [])
                .map_err(|e| e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())?;
        *current = candidate;
        Ok(())
    }

    pub fn new(db_path: PathBuf) -> Result<Self, String> {
        Self::new_with_key_provider(db_path, Arc::new(OsHistoryKeyProvider))
    }

    pub fn new_with_key_provider(
        db_path: PathBuf,
        key_provider: Arc<dyn HistoryKeyProvider>,
    ) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create database directory: {}", e))?;
        }

        let mut conn = Connection::open(&db_path)
            .map_err(|e| format!("Failed to open SQLite database: {}", e))?;

        // Read/verify the encrypted manifest before any migration or PRAGMA
        // writes. A missing/wrong key must leave the original DB untouched.
        let has_manifest: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='history_settings')", [], |row| row.get(0)).map_err(|e| e.to_string())?;
        let encrypted = if has_manifest {
            let flag = conn
                .query_row(
                    "SELECT value FROM history_settings WHERE key='encryption'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            let has_verification: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM history_settings WHERE key='verification')",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            match (flag.as_deref(), has_verification) {
                (Some("0"), false) => false,
                (Some("1"), true) => true,
                _ => return Err(
                    "History encryption metadata is damaged. The original database was preserved."
                        .into(),
                ),
            }
        } else {
            false
        };
        let cipher = if encrypted {
            let cipher = HistoryCipher::new(key_provider.load_key(false)?);
            let verifier: String = conn
                .query_row(
                    "SELECT value FROM history_settings WHERE key='verification'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| "Encrypted history verifier is missing".to_string())?;
            if cipher.open_text("manifest", "verification", &verifier)?
                != "reflow history encryption v1"
            {
                return Err("History encryption key verification failed".into());
            }
            Some(cipher)
        } else {
            None
        };
        Self::migrate_schema(&mut conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            recovery_notice: None,
            audio_retention: Mutex::new("disabled".into()),
            cipher: Mutex::new(cipher),
            key_provider,
            encryption_required: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// Preserve an unreadable/newer original and keep startup usable. Recovery
    /// writes go to a separate database; the original is never renamed/deleted.
    pub fn open_recovering(db_path: PathBuf) -> Self {
        Self::open_recovering_with_key_provider(db_path, Arc::new(OsHistoryKeyProvider))
    }

    pub fn open_recovering_with_key_provider(
        db_path: PathBuf,
        key_provider: Arc<dyn HistoryKeyProvider>,
    ) -> Self {
        match Self::new_with_key_provider(db_path.clone(), key_provider.clone()) {
            Ok(store) => store,
            Err(error) => {
                let recovery_path = recovery_database_path(&db_path);
                let (mut store, storage) = match Self::new_with_key_provider(
                    recovery_path.clone(),
                    key_provider.clone(),
                ) {
                    Ok(store) => (
                        store,
                        format!("New dictations are saved in {}.", recovery_path.display()),
                    ),
                    Err(_) => {
                        let mut conn =
                            Connection::open_in_memory().expect("SQLite in-memory database");
                        Self::migrate_schema(&mut conn).expect("fresh in-memory schema");
                        (Self { conn: Arc::new(Mutex::new(conn)), recovery_notice: None, audio_retention: Mutex::new("disabled".into()), cipher: Mutex::new(None), key_provider, encryption_required: std::sync::atomic::AtomicBool::new(false) }, "New history is temporary and will be lost when the app closes. Export it before quitting.".into())
                    }
                };
                store.recovery_notice = Some(format!("History could not be opened: {error}. The original at {} is preserved. {storage} Recovery history is unencrypted.", db_path.display()));
                log::error!("{}", store.recovery_notice.as_deref().unwrap_or_default());
                store
            }
        }
    }

    pub fn query_entries(&self, query: &HistoryQuery) -> Result<HistoryPage, String> {
        let conn = self.conn.lock();
        self.query_on(&conn, query)
    }

    fn query_on(&self, conn: &Connection, query: &HistoryQuery) -> Result<HistoryPage, String> {
        if self.encryption_enabled() && !query.query.trim().is_empty() {
            return self.query_encrypted_on(conn, query);
        }
        for date in [&query.from, &query.until].into_iter().flatten() {
            chrono::DateTime::parse_from_rfc3339(date)
                .map_err(|_| "History dates must use RFC3339 with a timezone.".to_string())?;
        }
        if query.query.len() > 4096 {
            return Err("History search is too long.".into());
        }
        let offset = query.offset.unwrap_or(0);
        if offset > 1_000_000 {
            return Err("History offset exceeds the supported range.".into());
        }
        let limit = query.limit.unwrap_or(50).clamp(1, 100);
        // instr is literal: percent/underscore/backslash are ordinary user text.
        let predicate = "(?1 = '' OR instr(lower(final_transcript), lower(?1)) > 0 OR instr(lower(raw_transcript), lower(?1)) > 0 OR instr(lower(smart_transcript), lower(?1)) > 0 OR instr(lower(application_name), lower(?1)) > 0 OR instr(lower(application_process), lower(?1)) > 0) AND (?2 IS NULL OR julianday(created_at) >= julianday(?2)) AND (?3 IS NULL OR julianday(created_at) < julianday(?3)) AND (?4 IS NULL OR kind = ?4) AND (?5 IS NULL OR application_process = ?5) AND (?6 = 0 OR pinned = 1) AND (?7 IS NULL OR instr(',' || tags || ',', ',' || ?7 || ',') > 0)";
        let total: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM history WHERE {predicate}"),
                params![
                    query.query.trim(),
                    query.from,
                    query.until,
                    query.kind,
                    query.application_process,
                    query.pinned_only.unwrap_or(false),
                    query.tag
                ],
                |row| row.get(0),
            )
            .map_err(|e| format!("History count failed: {e}"))?;
        let mut statement = conn.prepare(&format!("SELECT id, created_at, duration_ms, language, raw_transcript, final_transcript, application_name, application_process, word_count, character_count, model_version, processing_mode, smart_transcript, rewriter_used, audio_available, audio_expires_at, command_input, kind, source, pinned, tags FROM history_with_audio WHERE {predicate} ORDER BY pinned DESC, julianday(created_at) DESC, id DESC LIMIT ?8 OFFSET ?9")).map_err(|e| format!("History query failed: {e}"))?;
        let entries = statement
            .query_map(
                params![
                    query.query.trim(),
                    query.from,
                    query.until,
                    query.kind,
                    query.application_process,
                    query.pinned_only.unwrap_or(false),
                    query.tag,
                    limit as i64,
                    offset as i64
                ],
                map_history_row,
            )
            .map_err(|e| format!("History query failed: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("Failed to read history entry: {e}"))?
            .into_iter()
            .map(|entry| self.decode_entry(entry))
            .collect::<Result<Vec<_>, _>>()?;
        let end = offset + entries.len();
        Ok(HistoryPage {
            entries,
            total: total as usize,
            next_offset: (end < total as usize).then_some(end),
            recovery_notice: self.recovery_notice.clone(),
        })
    }

    /// Encrypted content cannot be searched by SQLite. Scan at most 10,000
    /// metadata-filtered rows in pages of 100; ask callers to narrow date/kind
    /// filters instead of silently returning an incomplete total.
    fn query_encrypted_on(
        &self,
        conn: &Connection,
        query: &HistoryQuery,
    ) -> Result<HistoryPage, String> {
        if query.query.len() > 4096 || query.offset.unwrap_or(0) > 1_000_000 {
            return Err("History search or offset exceeds the supported range.".into());
        }
        let mut scan = query.clone();
        scan.query.clear();
        scan.limit = Some(100);
        scan.offset = Some(0);
        let needle = query.query.trim().to_lowercase();
        let offset = query.offset.unwrap_or(0);
        let limit = query.limit.unwrap_or(50).clamp(1, 100);
        let mut entries = Vec::new();
        let mut total = 0;
        loop {
            let page = self.query_on(conn, &scan)?;
            if page.total > 10_000 {
                return Err("Encrypted search is limited to 10,000 records. Narrow the date, app or kind filters.".into());
            }
            for entry in page.entries.into_iter().filter(|entry| {
                [
                    &entry.raw_transcript,
                    &entry.smart_transcript,
                    &entry.final_transcript,
                    &entry.application_name,
                    &entry.application_process,
                ]
                .iter()
                .any(|value| value.to_lowercase().contains(&needle))
            }) {
                if total >= offset && entries.len() < limit {
                    entries.push(entry);
                }
                total += 1;
            }
            match page.next_offset {
                Some(offset) => scan.offset = Some(offset),
                None => break,
            }
        }
        let end = offset + entries.len();
        Ok(HistoryPage {
            entries,
            total,
            next_offset: (end < total).then_some(end),
            recovery_notice: self.recovery_notice.clone(),
        })
    }

    pub fn export_json(
        &self,
        query: &HistoryQuery,
        output: &std::path::Path,
    ) -> Result<usize, String> {
        use std::io::Write;
        let conn = self.conn.lock();
        let transaction = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let mut query = query.clone();
        query.offset = Some(0);
        query.limit = Some(100);
        let first = self.query_on(&transaction, &query)?;
        if first.total > 10_000 {
            return Err(
                "Export is limited to 10,000 entries. Narrow the search or date range.".into(),
            );
        }
        let temporary = output.with_extension(format!("{}.partial", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|e| e.to_string())?;
            file.write_all(b"[\n").map_err(|e| e.to_string())?;
            let total = first.total;
            let mut page = first;
            let mut written = 0;
            loop {
                for entry in &page.entries {
                    if written > 0 {
                        file.write_all(b",\n").map_err(|e| e.to_string())?;
                    }
                    serde_json::to_writer(&mut file, entry).map_err(|e| e.to_string())?;
                    written += 1;
                }
                let Some(next) = page.next_offset else {
                    break;
                };
                query.offset = Some(next);
                page = self.query_on(&transaction, &query)?;
            }
            file.write_all(b"\n]\n")
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
            drop(file);
            // Output names are generated by the desktop command, never user paths.
            if output.exists() {
                return Err("An export with that name already exists.".into());
            }
            std::fs::rename(&temporary, output).map_err(|e| e.to_string())?;
            Ok(total)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }

    fn migrate_schema(conn: &mut Connection) -> Result<(), String> {
        // Cascade transcript deletion to its audio, and overwrite deleted blobs.
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA secure_delete = ON;")
            .map_err(|e| e.to_string())?;
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|e| format!("Failed to read history schema version: {e}"))?;
        if version > CURRENT_HISTORY_SCHEMA_VERSION {
            return Err(format!(
                "History schema version {version} is newer than this app supports"
            ));
        }
        let transaction = conn
            .transaction()
            .map_err(|e| format!("Failed to start history migration: {e}"))?;
        let conn = &transaction;

        if version < 1 {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS history (
                    id TEXT PRIMARY KEY,
                    created_at TEXT NOT NULL,
                    duration_ms INTEGER NOT NULL,
                    language TEXT NOT NULL,
                    raw_transcript TEXT NOT NULL,
                    final_transcript TEXT NOT NULL,
                    application_name TEXT NOT NULL,
                    application_process TEXT NOT NULL,
                    word_count INTEGER NOT NULL,
                    character_count INTEGER NOT NULL,
                    model_version TEXT NOT NULL,
                    processing_mode TEXT NOT NULL,
                    smart_transcript TEXT NOT NULL DEFAULT '',
                    rewriter_used INTEGER NOT NULL DEFAULT 0
                )",
                [],
            )
            .map_err(|e| format!("Failed to initialize database schema: {}", e))?;
        }

        if version < 2 {
            let mut statement = conn
                .prepare("PRAGMA table_info(history)")
                .map_err(|e| format!("Failed to inspect history schema: {e}"))?;
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(|e| format!("Failed to inspect history columns: {e}"))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| format!("Failed to inspect history columns: {e}"))?;
            for (column, declaration) in [
                ("smart_transcript", "TEXT NOT NULL DEFAULT ''"),
                ("rewriter_used", "INTEGER NOT NULL DEFAULT 0"),
            ] {
                if !columns.iter().any(|existing| existing == column) {
                    conn.execute(
                        &format!("ALTER TABLE history ADD COLUMN {column} {declaration}"),
                        [],
                    )
                    .map_err(|e| format!("Failed to migrate history column '{column}': {e}"))?;
                }
            }
        }

        if version < 3 {
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_history_created_at ON history(created_at DESC)",
                [],
            )
            .map_err(|e| format!("Failed to index history dates: {e}"))?;
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_history_app_proc ON history(application_process, application_name)",
                [],
            )
            .map_err(|e| format!("Failed to index history applications: {e}"))?;
            conn.execute("DROP INDEX IF EXISTS idx_history_final_tx", [])
                .map_err(|e| format!("Failed to index history transcripts: {e}"))?;
        }

        if version < 4 {
            conn.execute_batch(
                "CREATE TABLE history_audio (
                    history_id TEXT PRIMARY KEY REFERENCES history(id) ON DELETE CASCADE,
                    recorded_at_ms INTEGER NOT NULL,
                    expires_at_ms INTEGER,
                    pcm BLOB NOT NULL
                );
                CREATE INDEX idx_history_audio_expiry ON history_audio(expires_at_ms);
                CREATE VIEW history_with_audio AS SELECT history.*,
                    EXISTS(SELECT 1 FROM history_audio a WHERE a.history_id = history.id
                        AND (a.expires_at_ms IS NULL OR a.expires_at_ms > CAST(round((julianday('now') - 2440587.5) * 86400000) AS INTEGER))) AS audio_available,
                    (SELECT expires_at_ms FROM history_audio a WHERE a.history_id = history.id) AS audio_expires_at
                    FROM history;",
            ).map_err(|e| format!("Failed to migrate history audio: {e}"))?;
        }

        if version < 5 {
            conn.execute_batch(
                "ALTER TABLE history ADD COLUMN command_input TEXT;
                ALTER TABLE history ADD COLUMN kind TEXT NOT NULL DEFAULT 'dictation';
                ALTER TABLE history ADD COLUMN source TEXT NOT NULL DEFAULT 'dictation';
                ALTER TABLE history ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
                ALTER TABLE history ADD COLUMN tags TEXT NOT NULL DEFAULT '';
                CREATE INDEX idx_history_kind ON history(kind, created_at);",
            )
            .map_err(|e| format!("Failed to migrate history metadata: {e}"))?;
        }

        if version < 6 {
            conn.execute_batch("CREATE TABLE IF NOT EXISTS history_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL); INSERT OR IGNORE INTO history_settings VALUES ('encryption', '0'); CREATE TABLE IF NOT EXISTS file_segments (history_id TEXT PRIMARY KEY REFERENCES history(id) ON DELETE CASCADE, segments TEXT NOT NULL);")
                .map_err(|e| e.to_string())?;
        }

        conn.execute(
            &format!("PRAGMA user_version = {CURRENT_HISTORY_SCHEMA_VERSION}"),
            [],
        )
        .map_err(|e| format!("Failed to record history schema version: {e}"))?;
        transaction
            .commit()
            .map_err(|e| format!("Failed to commit history migration: {e}"))?;

        Ok(())
    }

    pub fn set_audio_retention(&self, policy: &str) -> Result<usize, String> {
        let duration = audio_retention_ms(policy)?;
        let mut current_policy = self.audio_retention.lock();
        let conn = self.conn.lock();
        let transaction = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        // Expired audio cannot be resurrected by extending the policy.
        let mut deleted = purge_audio_on(&transaction, Utc::now().timestamp_millis())?;
        if policy == "disabled" {
            deleted += transaction
                .execute("DELETE FROM history_audio", [])
                .map_err(|e| e.to_string())?;
        } else {
            transaction
                .execute(
                    "UPDATE history_audio SET expires_at_ms = recorded_at_ms + ?1",
                    params![duration],
                )
                .map_err(|e| e.to_string())?;
            deleted += purge_audio_on(&transaction, Utc::now().timestamp_millis())?;
        }
        transaction.commit().map_err(|e| e.to_string())?;
        *current_policy = policy.into();
        Ok(deleted)
    }

    pub fn purge_expired_audio(&self) -> Result<usize, String> {
        purge_audio_on(&self.conn.lock(), Utc::now().timestamp_millis())
    }

    /// Saves 16 kHz mono float PCM only when retention is enabled.
    pub fn save_audio(&self, id: &str, samples: &[f32]) -> Result<(), String> {
        self.ensure_private_writes()?;
        let policy = self.audio_retention.lock();
        if *policy == "disabled" || samples.is_empty() {
            return Ok(());
        }
        let duration = audio_retention_ms(&policy)?;
        let conn = self.conn.lock();
        let created_at: String = conn
            .query_row(
                "SELECT created_at FROM history WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .map_err(|e| format!("Cannot save audio for this transcript: {e}"))?;
        let recorded_at = chrono::DateTime::parse_from_rfc3339(&created_at)
            .map_err(|e| e.to_string())?
            .timestamp_millis();
        let expiry = duration.map(|ms| recorded_at + ms);
        if expiry.is_some_and(|at| at <= Utc::now().timestamp_millis()) {
            return Ok(());
        }
        let bytes: Vec<u8> = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        let bytes = match self.cipher.lock().as_ref() {
            Some(cipher) => cipher.seal(id, "pcm", &bytes)?,
            None => bytes,
        };
        conn.execute("INSERT INTO history_audio(history_id, recorded_at_ms, expires_at_ms, pcm) VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(history_id) DO UPDATE SET pcm = excluded.pcm, expires_at_ms = excluded.expires_at_ms", params![id, recorded_at, expiry, bytes])
            .map_err(|e| format!("Could not save recording: {e}"))?;
        Ok(())
    }

    pub fn load_audio(&self, id: &str) -> Result<Vec<f32>, String> {
        let conn = self.conn.lock();
        purge_audio_on(&conn, Utc::now().timestamp_millis())?;
        let bytes: Option<Vec<u8>> = conn
            .query_row(
                "SELECT pcm FROM history_audio WHERE history_id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let bytes = bytes.ok_or("Audio is no longer available for this transcript.")?;
        let bytes = match self.cipher.lock().as_ref() {
            Some(cipher) => cipher.open(id, "pcm", &bytes)?,
            None => bytes,
        };
        if bytes.is_empty() || bytes.len() % 4 != 0 {
            return Err("Saved audio is damaged.".into());
        }
        Ok(bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| f32::from_le_bytes(*chunk))
            .collect())
    }

    pub fn export_audio(&self, id: &str, output: &std::path::Path) -> Result<(), String> {
        let samples = self.load_audio(id)?;
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            let mut writer = hound::WavWriter::new(
                file,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 16_000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .map_err(|e| e.to_string())?;
            for sample in samples {
                writer
                    .write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)
                    .map_err(|e| e.to_string())?;
            }
            writer.finalize().map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(output);
        }
        result
    }

    pub fn insert_entry(&self, entry: &HistoryEntry) -> Result<(), String> {
        self.ensure_private_writes()?;
        let conn = self.conn.lock();
        let raw = self.encode_text(&entry.id, "raw_transcript", &entry.raw_transcript)?;
        let smart = self.encode_text(&entry.id, "smart_transcript", &entry.smart_transcript)?;
        let final_text =
            self.encode_text(&entry.id, "final_transcript", &entry.final_transcript)?;
        let command = entry
            .command_input
            .as_ref()
            .map(|text| self.encode_text(&entry.id, "command_input", text))
            .transpose()?;
        conn.execute(
            "INSERT INTO history (
                id, created_at, duration_ms, language, raw_transcript,
                final_transcript, application_name, application_process,
                word_count, character_count, model_version, processing_mode,
                smart_transcript, rewriter_used, command_input, kind, source, pinned, tags
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                entry.id,
                entry.created_at,
                entry.duration_ms as i64,
                entry.language,
                raw,
                final_text,
                entry.application_name,
                entry.application_process,
                entry.word_count as i64,
                entry.character_count as i64,
                entry.model_version,
                entry.processing_mode,
                smart,
                if entry.rewriter_used { 1i64 } else { 0 }, command, entry.kind, entry.source, entry.pinned as i64, entry.tags
            ],
        )
        .map_err(|e| format!("Failed to insert history entry: {}", e))?;

        Ok(())
    }

    pub fn get_entries(&self, limit: usize, offset: usize) -> Result<Vec<HistoryEntry>, String> {
        if offset > 1_000_000 {
            return Err("History offset exceeds the supported range.".into());
        }
        let conn = self.conn.lock();
        let bounded_limit = limit.clamp(1, 100);
        let mut stmt = conn
            .prepare(
                "SELECT id, created_at, duration_ms, language, raw_transcript,
                        final_transcript, application_name, application_process,
                        word_count, character_count, model_version, processing_mode,
                        smart_transcript, rewriter_used, audio_available, audio_expires_at, command_input, kind, source, pinned, tags
                 FROM history_with_audio
                 ORDER BY created_at DESC
                 LIMIT ?1 OFFSET ?2",
            )
            .map_err(|e| format!("Failed to prepare select query: {}", e))?;

        let rows = stmt
            .query_map(
                params![bounded_limit as i64, offset as i64],
                map_history_row,
            )
            .map_err(|e| format!("Query failed: {}", e))?;

        let mut results = Vec::new();
        for entry in rows {
            results.push(
                self.decode_entry(
                    entry.map_err(|e| format!("Failed to read history entry: {e}"))?,
                )?,
            );
        }

        Ok(results)
    }

    pub fn search_entries(&self, query: &str) -> Result<Vec<HistoryEntry>, String> {
        self.search_entries_paged(query, 100, 0)
    }

    pub fn search_entries_paged(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<HistoryEntry>, String> {
        if offset > 1_000_000 || query.len() > 4096 {
            return Err("History search or offset exceeds the supported range.".into());
        }
        let conn = self.conn.lock();
        let bounded_limit = limit.clamp(1, 100);
        if self.encryption_enabled() {
            return self
                .query_on(
                    &conn,
                    &HistoryQuery {
                        query: query.into(),
                        limit: Some(bounded_limit),
                        offset: Some(offset),
                        ..Default::default()
                    },
                )
                .map(|page| page.entries);
        }
        let search_pattern = format!(
            "%{}%",
            query
                .trim()
                .replace('!', "!!")
                .replace('%', "!%")
                .replace('_', "!_")
        );

        let mut stmt = conn
            .prepare(
                "SELECT id, created_at, duration_ms, language, raw_transcript,
                        final_transcript, application_name, application_process,
                        word_count, character_count, model_version, processing_mode,
                        smart_transcript, rewriter_used, audio_available, audio_expires_at, command_input, kind, source, pinned, tags
                 FROM history_with_audio
                 WHERE final_transcript LIKE ?1 ESCAPE '!' OR raw_transcript LIKE ?1 ESCAPE '!'
                    OR smart_transcript LIKE ?1 ESCAPE '!' OR application_name LIKE ?1 ESCAPE '!'
                    OR application_process LIKE ?1 ESCAPE '!'
                 ORDER BY created_at DESC
                 LIMIT ?2 OFFSET ?3",
            )
            .map_err(|e| format!("Failed to prepare search query: {}", e))?;

        let rows = stmt
            .query_map(
                params![search_pattern, bounded_limit as i64, offset as i64],
                map_history_row,
            )
            .map_err(|e| format!("Search execution failed: {}", e))?;

        let mut results = Vec::new();
        for entry in rows {
            results.push(entry.map_err(|e| format!("Failed to read history search result: {e}"))?);
        }

        Ok(results)
    }

    pub fn get_entry(&self, id: &str) -> Result<Option<HistoryEntry>, String> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, created_at, duration_ms, language, raw_transcript,
                    final_transcript, application_name, application_process,
                    word_count, character_count, model_version, processing_mode,
                    smart_transcript, rewriter_used, audio_available, audio_expires_at, command_input, kind, source, pinned, tags
             FROM history_with_audio WHERE id = ?1",
            params![id],
            map_history_row,
        )
        .optional()
        .map_err(|e| format!("Failed to read history entry: {e}"))?
        .map(|entry| self.decode_entry(entry)).transpose()
    }

    /// Replace a transcript only after audio recognition and cleanup succeed.
    pub fn replace_transcript(
        &self,
        id: &str,
        raw: &str,
        smart: &str,
        final_text: &str,
        rewriter_used: bool,
        language: &str,
    ) -> Result<bool, String> {
        self.ensure_private_writes()?;
        let conn = self.conn.lock();
        let encoded_raw = self.encode_text(id, "raw_transcript", raw)?;
        let encoded_smart = self.encode_text(id, "smart_transcript", smart)?;
        let encoded_final = self.encode_text(id, "final_transcript", final_text)?;
        let transaction = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let affected = transaction.execute(
            "UPDATE history SET raw_transcript = ?2, smart_transcript = ?3, final_transcript = ?4,
            rewriter_used = ?5, language = ?6, word_count = ?7, character_count = ?8 WHERE id = ?1",
            params![
                id,
                encoded_raw,
                encoded_smart,
                encoded_final,
                rewriter_used,
                language,
                final_text.split_whitespace().count() as i64,
                final_text.chars().count() as i64
            ],
        )
        .map_err(|e| format!("Failed to save retried transcript: {e}"))?;
        transaction
            .execute("DELETE FROM file_segments WHERE history_id = ?1", [id])
            .map_err(|e| format!("Failed to invalidate old subtitle text: {e}"))?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(affected > 0)
    }

    /// Rewrite stored transcript text after an AI-undo or a cleanup retry.
    /// Word and character counts always describe the final transcript.
    pub fn save_file_segments(
        &self,
        id: &str,
        segments: &[crate::file_jobs::TranscriptSegment],
    ) -> Result<(), String> {
        self.ensure_private_writes()?;
        let conn = self.conn.lock();
        conn.execute_batch("CREATE TABLE IF NOT EXISTS file_segments (history_id TEXT PRIMARY KEY REFERENCES history(id) ON DELETE CASCADE, segments TEXT NOT NULL)").map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO file_segments VALUES (?1, ?2)",
            params![
                id,
                self.encode_text(
                    id,
                    "segments",
                    &serde_json::to_string(segments).map_err(|e| e.to_string())?
                )?
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn file_segments(
        &self,
        id: &str,
    ) -> Result<Vec<crate::file_jobs::TranscriptSegment>, String> {
        let conn = self.conn.lock();
        conn.execute_batch("CREATE TABLE IF NOT EXISTS file_segments (history_id TEXT PRIMARY KEY REFERENCES history(id) ON DELETE CASCADE, segments TEXT NOT NULL)").map_err(|e| e.to_string())?;
        let json: Option<String> = conn
            .query_row(
                "SELECT segments FROM file_segments WHERE history_id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        json.map(|j| {
            let text = match self.cipher.lock().as_ref() {
                Some(cipher) => cipher.open_text(id, "segments", &j)?,
                None => j,
            };
            serde_json::from_str(&text).map_err(|e| e.to_string())
        })
        .unwrap_or_else(|| Ok(vec![]))
    }

    pub fn update_metadata(&self, id: &str, pinned: bool, tags: &str) -> Result<(), String> {
        if tags.len() > 500 {
            return Err("Tags are limited to 500 bytes".into());
        }
        self.conn
            .lock()
            .execute(
                "UPDATE history SET pinned=?1, tags=?2 WHERE id=?3",
                params![pinned as i32, tags, id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn update_transcript(
        &self,
        id: &str,
        smart_transcript: &str,
        final_transcript: &str,
        rewriter_used: bool,
    ) -> Result<bool, String> {
        self.ensure_private_writes()?;
        let conn = self.conn.lock();
        let encoded_smart = self.encode_text(id, "smart_transcript", smart_transcript)?;
        let encoded_final = self.encode_text(id, "final_transcript", final_transcript)?;
        let transaction = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let affected = transaction
            .execute(
                "UPDATE history SET smart_transcript = ?2, final_transcript = ?3,
                 rewriter_used = ?4, word_count = ?5, character_count = ?6
                 WHERE id = ?1",
                params![
                    id,
                    encoded_smart,
                    encoded_final,
                    if rewriter_used { 1i64 } else { 0 },
                    final_transcript.split_whitespace().count() as i64,
                    final_transcript.chars().count() as i64
                ],
            )
            .map_err(|e| format!("Failed to update history item: {e}"))?;
        transaction
            .execute("DELETE FROM file_segments WHERE history_id = ?1", [id])
            .map_err(|e| format!("Failed to invalidate old subtitle text: {e}"))?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(affected > 0)
    }

    pub fn delete_entry(&self, id: &str) -> Result<bool, String> {
        let conn = self.conn.lock();
        let affected = conn
            .execute("DELETE FROM history WHERE id = ?1", params![id])
            .map_err(|e| format!("Failed to delete history item: {}", e))?;
        Ok(affected > 0)
    }

    pub fn clear_today(&self) -> Result<usize, String> {
        // `created_at` is stored as RFC3339 (UTC, via `chrono::Utc::now()` in the
        // stop path, and `purge_older_than` compares against an RFC3339 cutoff).
        // Comparing against a bare `%Y-%m-%d` date string mixed formats and could
        // miss rows; compare against the RFC3339 start-of-today instead.
        // Local dates handle zones whose midnight is ambiguous or skipped.
        let today_iso = Local::now().date_naive().to_string();

        let conn = self.conn.lock();
        let affected = conn
            .execute(
                "DELETE FROM history WHERE date(created_at, 'localtime') = ?1",
                params![today_iso],
            )
            .map_err(|e| format!("Failed to clear today's history: {}", e))?;

        Ok(affected)
    }

    pub fn clear_all(&self) -> Result<usize, String> {
        let conn = self.conn.lock();
        let affected = conn
            .execute("DELETE FROM history", [])
            .map_err(|e| format!("Failed to clear all history: {}", e))?;
        Ok(affected)
    }

    /// Automatic privacy cleanup preserves notes the user explicitly saved.
    /// Explicit clear-all and individual deletion still remove every kind.
    pub fn clear_transcript_history(&self) -> Result<usize, String> {
        self.conn
            .lock()
            .execute("DELETE FROM history WHERE kind != 'note'", [])
            .map_err(|e| format!("Failed to clear transcript history: {e}"))
    }

    pub fn purge_older_than(&self, days: u32) -> Result<usize, String> {
        let cutoff = Utc::now() - chrono::Duration::days(days as i64);
        let cutoff_iso = cutoff.to_rfc3339();

        let conn = self.conn.lock();
        let affected = conn
            .execute(
                "DELETE FROM history WHERE kind != 'note' AND julianday(created_at) < julianday(?1)",
                params![cutoff_iso],
            )
            .map_err(|e| format!("Failed to purge old history: {}", e))?;

        Ok(affected)
    }
}

fn recovery_database_path(original: &std::path::Path) -> PathBuf {
    let stable = original.with_extension("recovery.db");
    if stable.exists() {
        return stable;
    }
    // Older releases created a new UUID store on every failed startup. Resume
    // the newest regular legacy store instead of hiding its saved history.
    if original.file_name().and_then(|name| name.to_str()) == Some("history.db") {
        if let Some(parent) = original.parent() {
            let legacy = fs::read_dir(parent)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let filename = entry.file_name();
                    let id = filename
                        .to_str()?
                        .strip_prefix("history-recovery-")?
                        .strip_suffix(".db")?;
                    uuid::Uuid::parse_str(id).ok()?;
                    if !entry.file_type().ok()?.is_file() {
                        return None;
                    }
                    Some((entry.metadata().ok()?.modified().ok()?, entry.path()))
                })
                .max();
            if let Some((_, path)) = legacy {
                return path;
            }
        }
    }
    stable
}

fn map_history_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: row.get(0)?,
        created_at: row.get(1)?,
        duration_ms: row.get::<_, i64>(2)? as u64,
        language: row.get(3)?,
        raw_transcript: row.get(4)?,
        final_transcript: row.get(5)?,
        application_name: row.get(6)?,
        application_process: row.get(7)?,
        word_count: row.get::<_, i64>(8)? as usize,
        character_count: row.get::<_, i64>(9)? as usize,
        model_version: row.get(10)?,
        processing_mode: row.get(11)?,
        smart_transcript: row.get::<_, String>(12).unwrap_or_default(),
        rewriter_used: row.get::<_, i64>(13).unwrap_or(0) != 0,
        audio_available: row.get::<_, i64>(14)? != 0,
        audio_expires_at: row.get(15)?,
        command_input: row.get(16)?,
        kind: row.get(17)?,
        source: row.get(18)?,
        pinned: row.get::<_, i64>(19)? != 0,
        tags: row.get(20)?,
    })
}

pub fn audio_retention_ms(policy: &str) -> Result<Option<i64>, String> {
    match policy {
        "disabled" => Ok(Some(0)),
        "1_day" => Ok(Some(86_400_000)),
        "7_days" => Ok(Some(7 * 86_400_000)),
        "30_days" => Ok(Some(30 * 86_400_000)),
        "forever" => Ok(None),
        _ => Err("Unsupported audio retention policy.".into()),
    }
}

fn purge_audio_on(conn: &Connection, now_ms: i64) -> Result<usize, String> {
    conn.execute(
        "DELETE FROM history_audio WHERE expires_at_ms <= ?1",
        [now_ms],
    )
    .map_err(|e| format!("Could not delete expired audio: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_filter_precedes_paging_and_literal_search_returns_honest_total() {
        let dir =
            std::env::temp_dir().join(format!("reflow_history_query_{}", uuid::Uuid::new_v4()));
        let store = HistoryStore::new(dir.join("history.db")).unwrap();
        for n in 0..60 {
            let mut entry = sample_entry();
            entry.id = format!("id-{n}");
            entry.created_at = if n == 59 {
                "2026-08-20T10:00:00+05:30"
            } else {
                "2026-08-21T10:00:00Z"
            }
            .into();
            entry.final_transcript = if n == 59 { "100%_done" } else { "newer" }.into();
            store.insert_entry(&entry).unwrap();
        }
        let page = store
            .query_entries(&HistoryQuery {
                query: "%_".into(),
                until: Some("2026-08-21T00:00:00Z".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.entries[0].id, "id-59");
        assert!(page.next_offset.is_none());
        let page = store.query_entries(&HistoryQuery::default()).unwrap();
        assert_eq!(page.total, 60);
        assert_eq!(page.next_offset, Some(50));
        assert!(store
            .query_entries(&HistoryQuery {
                offset: Some(usize::MAX),
                ..Default::default()
            })
            .is_err());
        assert_eq!(store.search_entries("%_").unwrap().len(), 1);
    }

    #[test]
    fn recovery_preserves_original_database_bytes_and_reports_new_storage() {
        let dir =
            std::env::temp_dir().join(format!("reflow_history_recovery_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.db");
        fs::write(&path, b"corrupt original").unwrap();
        let store = HistoryStore::open_recovering(path.clone());
        assert_eq!(fs::read(path).unwrap(), b"corrupt original");
        assert!(store
            .query_entries(&HistoryQuery::default())
            .unwrap()
            .recovery_notice
            .is_some());
        store.insert_entry(&sample_entry()).unwrap();
        assert_eq!(store.get_entries(10, 0).unwrap().len(), 1);
    }

    #[test]
    fn export_contains_original_and_cleaned_text_without_overwriting_a_file() {
        let dir =
            std::env::temp_dir().join(format!("reflow_history_export_{}", uuid::Uuid::new_v4()));
        let store = HistoryStore::new(dir.join("history.db")).unwrap();
        store.insert_entry(&sample_entry()).unwrap();
        let output = dir.join("export.json");
        assert_eq!(
            store
                .export_json(&HistoryQuery::default(), &output)
                .unwrap(),
            1
        );
        let entries: Vec<HistoryEntry> =
            serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
        assert_eq!(entries[0].raw_transcript, "um hello world");
        assert_eq!(entries[0].final_transcript, "Hello world.");
        assert!(store
            .export_json(&HistoryQuery::default(), &output)
            .is_err());
    }

    fn sample_entry() -> HistoryEntry {
        HistoryEntry {
            id: "id-smart".into(),
            created_at: "2026-08-21T10:00:00Z".into(),
            duration_ms: 2100,
            language: "en".into(),
            raw_transcript: "um hello world".into(),
            smart_transcript: "Hello world.".into(),
            final_transcript: "Hello world.".into(),
            rewriter_used: true,
            application_name: "Visual Studio Code".into(),
            application_process: "Code.exe".into(),
            word_count: 2,
            character_count: 12,
            model_version: "0.6B-v1".into(),
            processing_mode: "medium".into(),
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
    fn insert_and_read_smart_transcript_and_rewriter_used() {
        let dir = std::env::temp_dir().join(format!("reflow_hist_{}", uuid::Uuid::new_v4()));
        let store = HistoryStore::new(dir.join("history.db")).expect("db");
        let entry = sample_entry();
        store.insert_entry(&entry).expect("insert");
        let rows = store.get_entries(10, 0).expect("select");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].raw_transcript, "um hello world");
        assert_eq!(rows[0].smart_transcript, "Hello world.");
        assert_eq!(rows[0].final_transcript, "Hello world.");
        assert!(rows[0].rewriter_used);
        let found = store.search_entries("Hello world").expect("search");
        assert_eq!(found.len(), 1);
        assert!(found[0].rewriter_used);

        // Search by application process
        let found_proc = store
            .search_entries_paged("Code.exe", 10, 0)
            .expect("search by proc");
        assert_eq!(found_proc.len(), 1);

        // Pagination bounds clamp limit
        let paged = store.get_entries(500, 0).expect("clamped select");
        assert_eq!(paged.len(), 1);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn schema_version_is_tracked() {
        let dir = std::env::temp_dir().join(format!("reflow_hist_{}", uuid::Uuid::new_v4()));
        let db_path = dir.join("history.db");
        {
            let _store = HistoryStore::new(db_path.clone()).expect("db");
        }
        let conn = rusqlite::Connection::open(&db_path).expect("open");
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_HISTORY_SCHEMA_VERSION);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_migration_rolls_back_columns_and_schema_version() {
        let mut conn = Connection::open_in_memory().unwrap();
        // An incomplete legacy schema makes index creation fail after the
        // column upgrades; neither those upgrades nor the version may commit.
        conn.execute_batch("CREATE TABLE history (id TEXT); PRAGMA user_version = 1;")
            .unwrap();
        assert!(HistoryStore::migrate_schema(&mut conn).is_err());
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
        let count: i32 = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('history')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn newer_history_schema_is_rejected_without_downgrading() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA user_version = 99;").unwrap();
        assert!(HistoryStore::migrate_schema(&mut conn).is_err());
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 99);
    }

    #[test]
    fn unreadable_history_entry_is_reported_instead_of_hidden() {
        let dir =
            std::env::temp_dir().join(format!("reflow_hist_invalid_{}", uuid::Uuid::new_v4()));
        let store = HistoryStore::new(dir.join("history.db")).unwrap();
        let mut entry = sample_entry();
        entry.id = "invalid".into();
        store.insert_entry(&entry).unwrap();
        store
            .conn
            .lock()
            .execute(
                "UPDATE history SET duration_ms = 'invalid' WHERE id = 'invalid'",
                [],
            )
            .unwrap();
        assert!(store.get_entries(10, 0).is_err());
        assert!(store.search_entries("Hello").is_err());
        drop(store);
        let _ = fs::remove_dir_all(dir);
    }
}
