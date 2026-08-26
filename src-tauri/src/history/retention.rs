use super::db::HistoryStore;

pub struct RetentionCleaner;

impl RetentionCleaner {
    pub fn apply_retention(store: &HistoryStore, policy: &str) -> Result<usize, String> {
        let days = match policy {
            "1_day" => Some(1),
            "7_days" => Some(7),
            "30_days" => Some(30),
            "90_days" => Some(90),
            "disabled" => return store.clear_all(),
            _ => None, // "forever"
        };

        if let Some(d) = days {
            store.purge_older_than(d)
        } else {
            Ok(0)
        }
    }
}
