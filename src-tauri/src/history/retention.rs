use super::db::HistoryStore;

pub struct RetentionCleaner;

impl RetentionCleaner {
    pub fn apply_retention(store: &HistoryStore, policy: &str) -> Result<usize, String> {
        let trimmed = policy.trim().to_lowercase();
        let days = match trimmed.as_str() {
            "1_day" | "1" => Some(1),
            "7_days" | "7" => Some(7),
            "30_days" | "30" => Some(30),
            "90_days" | "90" => Some(90),
            "disabled" => return store.clear_transcript_history(),
            "indefinite" | "forever" | "0" | "none" | "" => None,
            _ => None,
        };

        if let Some(d) = days {
            store.purge_older_than(d)
        } else {
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::db::HistoryEntry;

    fn make_entry(id: &str, created_at: &str) -> HistoryEntry {
        HistoryEntry {
            id: id.into(),
            created_at: created_at.into(),
            duration_ms: 1000,
            language: "en".into(),
            raw_transcript: "test".into(),
            smart_transcript: "test".into(),
            final_transcript: "test".into(),
            rewriter_used: false,
            application_name: "test".into(),
            application_process: "test.exe".into(),
            word_count: 1,
            character_count: 4,
            model_version: "v1".into(),
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
    fn retention_policies_purge_appropriately() {
        let dir = std::env::temp_dir().join(format!("reflow_ret_{}", uuid::Uuid::new_v4()));
        let store = HistoryStore::new(dir.join("history.db")).expect("db");

        // Insert old entry (100 days old) and new entry (1 day old)
        let old_date = (chrono::Utc::now() - chrono::Duration::days(100)).to_rfc3339();
        let new_date = chrono::Utc::now().to_rfc3339();

        store.insert_entry(&make_entry("old", &old_date)).unwrap();
        store.insert_entry(&make_entry("new", &new_date)).unwrap();

        assert_eq!(store.get_entries(10, 0).unwrap().len(), 2);

        // Indefinite does not purge
        assert_eq!(
            RetentionCleaner::apply_retention(&store, "indefinite").unwrap(),
            0
        );
        assert_eq!(store.get_entries(10, 0).unwrap().len(), 2);

        // 30 days purges the 100-day-old entry
        let purged = RetentionCleaner::apply_retention(&store, "30_days").unwrap();
        assert_eq!(purged, 1);
        let remaining = store.get_entries(10, 0).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "new");

        let _ = std::fs::remove_dir_all(dir);
    }
}
