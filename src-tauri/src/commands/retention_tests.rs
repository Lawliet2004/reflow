use super::*;

#[test]
fn delayed_retention_cleanup_honors_the_latest_policy_and_preserves_notes() {
    for (scheduled, latest, transcript_survives) in [
        ("disabled", "indefinite", true),
        ("1_day", "indefinite", true),
        ("indefinite", "disabled", false),
    ] {
        let dir = std::env::temp_dir().join(format!("reflow_retention_{}", Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        let mut transcript = crate::expansion_commands::note_entry("old transcript".into());
        transcript.kind = "dictation".into();
        transcript.created_at = (chrono::Utc::now() - chrono::Duration::days(2)).to_rfc3339();
        let note = crate::expansion_commands::note_entry("explicit note".into());
        ctx.history_store.insert_entry(&transcript).unwrap();
        ctx.history_store.insert_entry(&note).unwrap();

        ctx.settings_store
            .merge_update(serde_json::json!({"history_retention": scheduled}))
            .unwrap();
        let cleanup = history_retention_cleanup(ctx.clone());
        {
            let _operation = ctx.settings_operation.lock();
            ctx.settings_store
                .merge_update(serde_json::json!({"history_retention": latest}))
                .unwrap();
        }
        cleanup().unwrap();

        let entries = ctx.history_store.get_entries(10, 0).unwrap();
        assert_eq!(
            entries.iter().any(|entry| entry.id == transcript.id),
            transcript_survives,
            "cleanup scheduled under {scheduled} must honor the latest {latest} policy"
        );
        assert!(entries.iter().any(|entry| entry.id == note.id));
        drop(ctx);
        let _ = std::fs::remove_dir_all(dir);
    }
}
