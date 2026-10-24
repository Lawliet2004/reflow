//! Audio and metadata commit together. No external file/manifest commit window.
use super::*;
use crate::history::encryption::{HistoryCipher, HistoryKeyProvider, OsHistoryKeyProvider};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub struct Store {
    conn: Connection,
    cipher: Option<HistoryCipher>,
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
impl Store {
    pub fn open(path: &Path, encrypt: bool) -> Result<Self, String> {
        let mut store = Self::open_with_key(path, None)?;
        // Key policy is persisted independently of preferences: missing keys never
        // cause an encrypted session to be read or overwritten as plaintext.
        let encrypted: bool = store
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM meeting WHERE encrypted=1)",
                [],
                |r| r.get(0),
            )
            .map_err(err)?;
        if encrypt || encrypted {
            store.cipher = Some(HistoryCipher::new(OsHistoryKeyProvider.load_key(false)?));
        }
        Ok(store)
    }
    pub fn open_with_key(path: &Path, key: Option<[u8; 32]>) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(err)?;
        }
        let conn = Connection::open(path).map_err(err)?;
        conn.busy_timeout(Duration::from_secs(2)).map_err(err)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON; PRAGMA wal_autocheckpoint=256;
        CREATE TABLE IF NOT EXISTS meeting(id TEXT PRIMARY KEY, encrypted INTEGER NOT NULL, state BLOB NOT NULL);
        CREATE TABLE IF NOT EXISTS meeting_chunk(session TEXT NOT NULL REFERENCES meeting(id) ON DELETE CASCADE, track INTEGER NOT NULL, seq INTEGER NOT NULL, start_frame INTEGER NOT NULL, frames INTEGER NOT NULL, checksum TEXT NOT NULL, pcm BLOB NOT NULL, PRIMARY KEY(session,track,seq));
        CREATE TABLE IF NOT EXISTS meeting_work(session TEXT NOT NULL REFERENCES meeting(id) ON DELETE CASCADE, key TEXT NOT NULL, content BLOB NOT NULL, PRIMARY KEY(session,key));
        PRAGMA user_version=1;").map_err(err)?;
        // fsync the containing directory on creation (SQLite owns WAL fsync).
        #[cfg(unix)]
        if let Some(parent) = path.parent() {
            std::fs::File::open(parent)
                .and_then(|f| f.sync_all())
                .map_err(err)?;
        }
        Ok(Self {
            conn,
            cipher: key.map(HistoryCipher::new),
        })
    }
    fn encrypted(&self, id: &str) -> Result<bool, String> {
        self.conn
            .query_row("SELECT encrypted FROM meeting WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .map_err(err)
    }
    fn seal(&self, id: &str, field: &str, data: &[u8], encrypted: bool) -> Result<Vec<u8>, String> {
        if encrypted {
            self.cipher
                .as_ref()
                .ok_or("Meeting encryption key unavailable")?
                .seal(id, field, data)
        } else {
            Ok(data.to_vec())
        }
    }
    fn open_value(&self, id: &str, field: &str, data: &[u8]) -> Result<Vec<u8>, String> {
        if self.encrypted(id)? {
            self.cipher
                .as_ref()
                .ok_or("Meeting encryption key unavailable")?
                .open(id, field, data)
        } else {
            Ok(data.to_vec())
        }
    }
    pub fn create(&self, meeting: &Meeting) -> Result<(), String> {
        let data = self.seal(
            &meeting.id,
            "state",
            &serde_json::to_vec(meeting).map_err(err)?,
            meeting.encrypted,
        )?;
        self.conn
            .execute(
                "INSERT INTO meeting VALUES(?1,?2,?3)",
                params![meeting.id, meeting.encrypted, data],
            )
            .map_err(err)?;
        Ok(())
    }
    pub fn save(&self, meeting: &Meeting) -> Result<(), String> {
        let data = self.seal(
            &meeting.id,
            "state",
            &serde_json::to_vec(meeting).map_err(err)?,
            self.encrypted(&meeting.id)?,
        )?;
        self.conn
            .execute(
                "UPDATE meeting SET state=?2 WHERE id=?1",
                params![meeting.id, data],
            )
            .map_err(err)?;
        Ok(())
    }
    pub fn get(&self, id: &str) -> Result<Meeting, String> {
        let data: Vec<u8> = self
            .conn
            .query_row("SELECT state FROM meeting WHERE id=?1", [id], |r| r.get(0))
            .map_err(err)?;
        serde_json::from_slice(&self.open_value(id, "state", &data)?).map_err(err)
    }
    pub fn list(&self) -> Result<Vec<Meeting>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM meeting ORDER BY rowid DESC")
            .map_err(err)?;
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        ids.iter().map(|id| self.get(id)).collect()
    }
    pub fn chunk(
        &mut self,
        m: &mut Meeting,
        track: usize,
        seq: u64,
        start: u64,
        pcm: &[f32],
    ) -> Result<(), String> {
        let bytes = crate::audio::AudioResampler::f32_to_pcm16_bytes(pcm);
        let field = format!("audio/{track}/{seq}");
        let data = self.seal(&m.id, &field, &bytes, m.encrypted)?;
        let mut committed = m.clone();
        committed.saved_frames[track] += pcm.len() as u64;
        committed.last_commit_ms = m.elapsed_ms;
        let state = self.seal(
            &m.id,
            "state",
            &serde_json::to_vec(&committed).map_err(err)?,
            m.encrypted,
        )?;
        let tx = self.conn.transaction().map_err(err)?;
        tx.execute(
            "INSERT INTO meeting_chunk VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![m.id, track, seq, start, pcm.len(), digest(&bytes), data],
        )
        .map_err(err)?;
        tx.execute(
            "UPDATE meeting SET state=?2 WHERE id=?1",
            params![m.id, state],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        *m = committed;
        Ok(())
    }
    pub fn chunks(&self, id: &str) -> Result<Vec<Chunk>, String> {
        let mut s=self.conn.prepare("SELECT track,seq,start_frame,frames FROM meeting_chunk WHERE session=?1 ORDER BY start_frame,track,seq").map_err(err)?;
        let result = s
            .query_map([id], |r| {
                Ok(Chunk {
                    track: r.get(0)?,
                    seq: r.get(1)?,
                    start: r.get(2)?,
                    frames: r.get(3)?,
                })
            })
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        Ok(result)
    }
    pub fn audio(&self, id: &str, c: &Chunk) -> Result<Vec<f32>, String> {
        let (checksum, data): (String, Vec<u8>) = self
            .conn
            .query_row(
                "SELECT checksum,pcm FROM meeting_chunk WHERE session=?1 AND track=?2 AND seq=?3",
                params![id, c.track, c.seq],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(err)?;
        let bytes = self.open_value(id, &format!("audio/{}/{}", c.track, c.seq), &data)?;
        if digest(&bytes) != checksum || bytes.len() != c.frames * 2 {
            return Err("Meeting audio checksum/sample count mismatch; input preserved".into());
        }
        Ok(crate::audio::AudioResampler::pcm16_bytes_to_f32(&bytes))
    }
    pub fn work(&self, id: &str, key: &str) -> Result<Option<String>, String> {
        let data: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT content FROM meeting_work WHERE session=?1 AND key=?2",
                params![id, key],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        data.map(|d| String::from_utf8(self.open_value(id, key, &d)?).map_err(err))
            .transpose()
    }
    pub fn put(&self, id: &str, key: &str, text: &str) -> Result<(), String> {
        let data = self.seal(id, key, text.as_bytes(), self.encrypted(id)?)?;
        self.conn.execute("INSERT INTO meeting_work VALUES(?1,?2,?3) ON CONFLICT(session,key) DO UPDATE SET content=excluded.content",params![id,key,data]).map_err(err)?;
        Ok(())
    }
    pub fn delete(&self, id: &str) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM meeting WHERE id=?1", [id])
            .map_err(err)?;
        self.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .map_err(err)
    }
    pub fn remove_audio(&self, id: &str) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM meeting_chunk WHERE session=?1", [id])
            .map_err(err)?;
        self.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .map_err(err)
    }
    pub fn recover(&self) -> Result<(), String> {
        // Never reopen capture automatically; a restart does not capture downtime.
        for mut m in self.list()? {
            if matches!(m.capture.as_str(), "recording" | "saving") {
                m.capture = "interrupted".into();
                m.warning=Some("Reflow stopped during capture. Only committed audio was recovered; no audio was recorded while the app was closed.".into());
            }
            if matches!(m.processing.as_str(), "transcribing" | "summarizing") {
                if m.processing == "transcribing" {
                    m.transcription_status = "interrupted".into();
                }
                if m.processing == "summarizing" {
                    m.summary_status = "interrupted".into();
                }
                m.processing = "interrupted".into();
            }
            self.save(&m)?;
        }
        Ok(())
    }
}
pub fn path() -> PathBuf {
    crate::platform::PlatformSys::get_app_dir().join("meetings/meetings.db")
}
impl Store {
    /// Keep the entire migration atomic; no plaintext staging files. Only runs
    /// while capture/jobs are idle, under application resource admission.
    pub fn set_encryption(&mut self, enabled: bool) -> Result<(), String> {
        if self.cipher.is_none() {
            self.cipher = Some(HistoryCipher::new(OsHistoryKeyProvider.load_key(false)?));
        }
        let meetings = self.list()?;
        let tx = self.conn.unchecked_transaction().map_err(err)?;
        for mut m in meetings {
            if m.encrypted == enabled {
                continue;
            }
            for c in self.chunks(&m.id)? {
                let field = format!("audio/{}/{}", c.track, c.seq);
                let stored: Vec<u8> = self
                    .conn
                    .query_row(
                        "SELECT pcm FROM meeting_chunk WHERE session=?1 AND track=?2 AND seq=?3",
                        params![m.id, c.track, c.seq],
                        |r| r.get(0),
                    )
                    .map_err(err)?;
                let bytes = self.open_value(&m.id, &field, &stored)?;
                let data = self.seal(&m.id, &field, &bytes, enabled)?;
                tx.execute("UPDATE meeting_chunk SET pcm=?4,checksum=?5 WHERE session=?1 AND track=?2 AND seq=?3",params![m.id,c.track,c.seq,data,digest(&bytes)]).map_err(err)?;
            }
            let mut stmt = self
                .conn
                .prepare("SELECT key FROM meeting_work WHERE session=?1")
                .map_err(err)?;
            let keys = stmt
                .query_map([&m.id], |r| r.get::<_, String>(0))
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)?;
            for key in keys {
                let text = self.work(&m.id, &key)?.ok_or("Missing meeting work")?;
                let data = self.seal(&m.id, &key, text.as_bytes(), enabled)?;
                tx.execute(
                    "UPDATE meeting_work SET content=?3 WHERE session=?1 AND key=?2",
                    params![m.id, key, data],
                )
                .map_err(err)?;
            }
            m.encrypted = enabled;
            let state = self.seal(
                &m.id,
                "state",
                &serde_json::to_vec(&m).map_err(err)?,
                enabled,
            )?;
            tx.execute(
                "UPDATE meeting SET encrypted=?2,state=?3 WHERE id=?1",
                params![m.id, enabled, state],
            )
            .map_err(err)?;
        }
        tx.commit().map_err(err)?;
        self.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")
            .map_err(err)
    }
    pub fn retention(&self, audio: &str, history: &str) -> Result<(), String> {
        let days = |p: &str| -> Option<i64> {
            match p {
                "1_day" | "1_days" => Some(1),
                "7_days" => Some(7),
                "30_days" => Some(30),
                "90_days" => Some(90),
                "disabled" => Some(0),
                _ => None,
            }
        };
        for mut m in self.list()? {
            // Failed/cancelled/interrupted/pending jobs retain recoverable inputs.
            if m.processing != "ready" || matches!(m.capture.as_str(), "recording" | "saving") {
                continue;
            }
            let age = chrono::DateTime::parse_from_rfc3339(&m.created_at)
                .map_err(err)?
                .with_timezone(&chrono::Utc);
            let elapsed = chrono::Utc::now().signed_duration_since(age).num_seconds();
            if days(history).is_some_and(|d| elapsed >= d * 86400) {
                self.delete(&m.id)?;
                continue;
            }
            if days(audio).is_some_and(|d| elapsed >= d * 86400) && !m.audio_removed {
                self.remove_audio(&m.id)?;
                m.audio_removed = true;
                self.save(&m)?;
            }
        }
        Ok(())
    }
}
#[cfg(test)]
impl Store {
    pub(super) fn refuse_growth(&self) {
        let pages: u64 = self
            .conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .unwrap();
        self.conn
            .execute_batch(&format!("PRAGMA max_page_count={pages}"))
            .unwrap();
    }
}
