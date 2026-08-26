use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use chrono::{Local, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
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
}

pub struct HistoryStore {
    conn: Arc<Mutex<Connection>>,
}

impl HistoryStore {
    pub fn new(db_path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create database directory: {}", e))?;
        }

        let conn = Connection::open(&db_path)
            .map_err(|e| format!("Failed to open SQLite database: {}", e))?;

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

        let _ = conn.execute(
            "ALTER TABLE history ADD COLUMN smart_transcript TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE history ADD COLUMN rewriter_used INTEGER NOT NULL DEFAULT 0",
            [],
        );

        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_history_created_at ON history(created_at DESC)",
            [],
        );

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn insert_entry(&self, entry: &HistoryEntry) -> Result<(), String> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO history (
                id, created_at, duration_ms, language, raw_transcript,
                final_transcript, application_name, application_process,
                word_count, character_count, model_version, processing_mode,
                smart_transcript, rewriter_used
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                entry.id,
                entry.created_at,
                entry.duration_ms as i64,
                entry.language,
                entry.raw_transcript,
                entry.final_transcript,
                entry.application_name,
                entry.application_process,
                entry.word_count as i64,
                entry.character_count as i64,
                entry.model_version,
                entry.processing_mode,
                entry.smart_transcript,
                if entry.rewriter_used { 1i64 } else { 0 }
            ],
        )
        .map_err(|e| format!("Failed to insert history entry: {}", e))?;

        Ok(())
    }

    pub fn get_entries(&self, limit: usize, offset: usize) -> Result<Vec<HistoryEntry>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, created_at, duration_ms, language, raw_transcript,
                        final_transcript, application_name, application_process,
                        word_count, character_count, model_version, processing_mode,
                        smart_transcript, rewriter_used
                 FROM history
                 ORDER BY created_at DESC
                 LIMIT ?1 OFFSET ?2",
            )
            .map_err(|e| format!("Failed to prepare select query: {}", e))?;

        let rows = stmt
            .query_map(params![limit as i64, offset as i64], map_history_row)
            .map_err(|e| format!("Query failed: {}", e))?;

        let mut results = Vec::new();
        for row in rows {
            if let Ok(entry) = row {
                results.push(entry);
            }
        }

        Ok(results)
    }

    pub fn search_entries(&self, query: &str) -> Result<Vec<HistoryEntry>, String> {
        let conn = self.conn.lock();
        let search_pattern = format!("%{}%", query);

        let mut stmt = conn
            .prepare(
                "SELECT id, created_at, duration_ms, language, raw_transcript,
                        final_transcript, application_name, application_process,
                        word_count, character_count, model_version, processing_mode,
                        smart_transcript, rewriter_used
                 FROM history
                 WHERE final_transcript LIKE ?1 OR raw_transcript LIKE ?1
                    OR smart_transcript LIKE ?1 OR application_name LIKE ?1
                 ORDER BY created_at DESC
                 LIMIT 100",
            )
            .map_err(|e| format!("Failed to prepare search query: {}", e))?;

        let rows = stmt
            .query_map(params![search_pattern], map_history_row)
            .map_err(|e| format!("Search execution failed: {}", e))?;

        let mut results = Vec::new();
        for row in rows {
            if let Ok(entry) = row {
                results.push(entry);
            }
        }

        Ok(results)
    }

    pub fn delete_entry(&self, id: &str) -> Result<bool, String> {
        let conn = self.conn.lock();
        let affected = conn
            .execute("DELETE FROM history WHERE id = ?1", params![id])
            .map_err(|e| format!("Failed to delete history item: {}", e))?;
        Ok(affected > 0)
    }

    pub fn clear_today(&self) -> Result<usize, String> {
        let today_start = Local::now().date_naive().and_hms_opt(0, 0, 0).unwrap();
        let today_iso = today_start.format("%Y-%m-%d").to_string();

        let conn = self.conn.lock();
        let affected = conn
            .execute(
                "DELETE FROM history WHERE created_at >= ?1",
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

    pub fn purge_older_than(&self, days: u32) -> Result<usize, String> {
        let cutoff = Utc::now() - chrono::Duration::days(days as i64);
        let cutoff_iso = cutoff.to_rfc3339();

        let conn = self.conn.lock();
        let affected = conn
            .execute(
                "DELETE FROM history WHERE created_at < ?1",
                params![cutoff_iso],
            )
            .map_err(|e| format!("Failed to purge old history: {}", e))?;

        Ok(affected)
    }
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let _ = std::fs::remove_dir_all(dir);
    }
}
