use reflow_lib::{
    expansion_commands::correction_suggestions, formatting::TextCleaner, settings::SettingsStore,
};

#[test]
fn learns_automatically_persists_and_isolates_users() {
    let directory =
        std::env::temp_dir().join(format!("reflow-dictionary-{}", uuid::Uuid::new_v4()));
    let user_a = SettingsStore::new(directory.join("a/settings.json"));
    let user_b = SettingsStore::new(directory.join("b/settings.json"));
    let learned = user_a
        .learn_dictionary_corrections("Use type script.", "Use TypeScript.")
        .unwrap()
        .unwrap();
    assert!(learned
        .dictionary_terms
        .iter()
        .any(|t| t.term == "type script" && t.preferred_spelling == "TypeScript"));
    let reloaded = SettingsStore::new(directory.join("a/settings.json"));
    assert!(reloaded
        .get()
        .dictionary_terms
        .iter()
        .any(|t| t.term == "type script"));
    assert!(!user_b
        .get()
        .dictionary_terms
        .iter()
        .any(|t| t.term == "type script"));
    assert!(user_a
        .learn_dictionary_corrections("Use type script.", "Use TypeScript.")
        .unwrap()
        .is_none());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn respects_opt_out_and_deleted_learned_rules() {
    let directory =
        std::env::temp_dir().join(format!("reflow-dictionary-{}", uuid::Uuid::new_v4()));
    let store = SettingsStore::new(directory.join("settings.json"));
    store
        .merge_update(serde_json::json!({"auto_learn_dictionary":false}))
        .unwrap();
    assert!(store
        .learn_dictionary_corrections("type script", "TypeScript")
        .unwrap()
        .is_none());
    store
        .merge_update(serde_json::json!({"auto_learn_dictionary":true}))
        .unwrap();
    store
        .learn_dictionary_corrections("type script", "TypeScript")
        .unwrap();
    let terms = store
        .get()
        .dictionary_terms
        .into_iter()
        .filter(|t| t.term != "type script")
        .collect::<Vec<_>>();
    store
        .merge_update(serde_json::json!({"dictionary_terms":terms}))
        .unwrap();
    assert!(store
        .learn_dictionary_corrections("type script", "TypeScript")
        .unwrap()
        .is_none());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn newest_preference_updates_existing_aliases() {
    let directory =
        std::env::temp_dir().join(format!("reflow-dictionary-{}", uuid::Uuid::new_v4()));
    let store = SettingsStore::new(directory.join("settings.json"));
    store
        .learn_dictionary_corrections("type script", "TypeScript")
        .unwrap();
    let latest = store
        .learn_dictionary_corrections("TypeScript", "Typescript")
        .unwrap()
        .unwrap();
    let terms = latest
        .dictionary_terms
        .iter()
        .map(|t| (t.term.clone(), t.preferred_spelling.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        TextCleaner::apply_glossary("use type script", &terms),
        "use Typescript"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn observed_edit_updates_history_preserves_raw_and_applies_next_time() {
    use reflow_lib::{
        context::AppContext, correction_observer::record_observed_edit,
        expansion_commands::note_entry, rewrite::FlowClient, session::postprocess_transcript,
    };
    let directory =
        std::env::temp_dir().join(format!("reflow-dictionary-{}", uuid::Uuid::new_v4()));
    let ctx = AppContext::bootstrap_test(directory.clone());
    let mut entry = note_entry("Use type script.".into());
    entry.kind = "dictation".into();
    entry.smart_transcript = "Use type script.".into();
    ctx.history_store.insert_entry(&entry).unwrap();
    assert!(record_observed_edit(
        &ctx,
        Some(&entry.id),
        &entry.final_transcript,
        "Use TypeScript."
    )
    .unwrap());
    let corrected = ctx.history_store.get_entry(&entry.id).unwrap().unwrap();
    assert_eq!(corrected.final_transcript, "Use TypeScript.");
    assert_eq!(corrected.raw_transcript, "Use type script.");
    assert_eq!(corrected.smart_transcript, "Use type script.");
    let mut settings = ctx.settings_store.get();
    settings.intelligence_tier = "raw_verbatim".into();
    settings.cleanup_level = "light".into();
    let next = postprocess_transcript(
        "use typescript",
        &settings,
        "notepad.exe",
        &FlowClient::new_missing(),
    );
    assert!(next.final_text.contains("TypeScript"));
    assert!(!next.rewriter_used);
    drop(ctx);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn observed_history_edits_work_with_encryption_and_keep_user_source() {
    use reflow_lib::{
        expansion_commands::note_entry,
        history::{encryption::HistoryKeyProvider, HistoryStore},
    };
    struct FixtureKey;
    impl HistoryKeyProvider for FixtureKey {
        fn load_key(&self, _: bool) -> Result<[u8; 32], String> {
            Ok([7; 32])
        }
    }
    let directory =
        std::env::temp_dir().join(format!("reflow-dictionary-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let store = HistoryStore::new_with_key_provider(
        directory.join("history.db"),
        std::sync::Arc::new(FixtureKey),
    )
    .unwrap();
    let mut entry = note_entry("Use type script".into());
    entry.kind = "dictation".into();
    store.insert_entry(&entry).unwrap();
    store.set_encryption(true).unwrap();
    assert!(store
        .update_observed_dictation(&entry.id, "Use type script", "Use TypeScript")
        .unwrap());
    let saved = store.get_entry(&entry.id).unwrap().unwrap();
    assert_eq!(saved.raw_transcript, "Use type script");
    assert_eq!(saved.final_transcript, "Use TypeScript");
    assert!(!store
        .update_observed_dictation(&entry.id, "Use type script", "Stale edit")
        .unwrap());
    assert!(store.encryption_enabled());
    assert!(!std::fs::read(directory.join("history.db"))
        .unwrap()
        .windows(14)
        .any(|w| w == b"Use TypeScript"));
    drop(store);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn fallback_cleanup_does_not_apply_replacements_twice() {
    use reflow_lib::{
        formatting::replacements::ReplacementRule, rewrite::FlowClient,
        session::postprocess_transcript, settings::AppSettings,
    };
    let settings = AppSettings {
        cleanup_level: "medium".into(),
        custom_replacements: vec![
            ReplacementRule {
                id: "a".into(),
                before: "Don".into(),
                after: "Dawn".into(),
                enabled: true,
            },
            ReplacementRule {
                id: "b".into(),
                before: "Dawn".into(),
                after: "Donna".into(),
                enabled: true,
            },
        ],
        ..AppSettings::default()
    };
    let result = postprocess_transcript(
        "Don says that this feature needs to be implemented.",
        &settings,
        "chat.exe",
        &FlowClient::new_missing(),
    );
    assert!(
        result.final_text.starts_with("Dawn"),
        "{}",
        result.final_text
    );
    assert!(!result.rewriter_used);
}

#[test]
fn ordinary_edits_sync_history_without_learning_but_later_spelling_edits_still_learn() {
    use reflow_lib::{
        context::AppContext, correction_observer::record_observed_edit,
        expansion_commands::note_entry,
    };
    let directory =
        std::env::temp_dir().join(format!("reflow-dictionary-{}", uuid::Uuid::new_v4()));
    let ctx = AppContext::bootstrap_test(directory.clone());
    let mut entry = note_entry("Use type script tomorrow.".into());
    entry.kind = "dictation".into();
    ctx.history_store.insert_entry(&entry).unwrap();
    let changed = "Use type script on Friday!";
    assert!(record_observed_edit(&ctx, Some(&entry.id), &entry.final_transcript, changed).unwrap());
    assert_eq!(
        ctx.history_store
            .get_entry(&entry.id)
            .unwrap()
            .unwrap()
            .final_transcript,
        changed
    );
    assert!(!ctx
        .settings_store
        .get()
        .dictionary_terms
        .iter()
        .any(|t| t.category == "Learned"));
    assert!(
        record_observed_edit(&ctx, Some(&entry.id), changed, "Use TypeScript on Friday!").unwrap()
    );
    assert!(ctx
        .settings_store
        .get()
        .dictionary_terms
        .iter()
        .any(|t| t.term == "type script"));
    drop(ctx);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn observer_does_not_overwrite_newer_edits_or_recreate_deleted_history() {
    use reflow_lib::{
        context::AppContext, correction_observer::record_observed_edit,
        expansion_commands::note_entry,
    };
    let directory =
        std::env::temp_dir().join(format!("reflow-dictionary-{}", uuid::Uuid::new_v4()));
    let ctx = AppContext::bootstrap_test(directory.clone());
    let mut entry = note_entry("Use type script.".into());
    entry.kind = "dictation".into();
    ctx.history_store.insert_entry(&entry).unwrap();
    ctx.history_store
        .update_transcript(&entry.id, "", "Use Rust.", false)
        .unwrap();
    assert!(!record_observed_edit(
        &ctx,
        Some(&entry.id),
        &entry.final_transcript,
        "Use TypeScript."
    )
    .unwrap());
    assert!(!ctx
        .settings_store
        .get()
        .dictionary_terms
        .iter()
        .any(|t| t.term == "type script"));
    ctx.history_store.delete_entry(&entry.id).unwrap();
    assert!(!record_observed_edit(
        &ctx,
        Some(&entry.id),
        &entry.final_transcript,
        "Use TypeScript."
    )
    .unwrap());
    assert!(ctx.history_store.get_entry(&entry.id).unwrap().is_none());
    drop(ctx);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn dictionary_learning_works_without_storing_history() {
    use reflow_lib::{context::AppContext, correction_observer::record_observed_edit};
    let directory =
        std::env::temp_dir().join(format!("reflow-dictionary-{}", uuid::Uuid::new_v4()));
    let ctx = AppContext::bootstrap_test(directory.clone());
    ctx.settings_store
        .merge_update(serde_json::json!({"history_retention":"disabled"}))
        .unwrap();
    assert!(record_observed_edit(&ctx, None, "type script", "TypeScript").unwrap());
    assert!(ctx
        .settings_store
        .get()
        .dictionary_terms
        .iter()
        .any(|t| t.term == "type script"));
    drop(ctx);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn literal_replacements_do_not_cascade_or_change_contractions() {
    let terms = vec![
        ("Don".into(), "Dawn".into()),
        ("Dawn".into(), "Donna".into()),
    ];
    assert_eq!(
        TextCleaner::apply_glossary("Don and Dawn don't wait", &terms),
        "Dawn and Donna don't wait"
    );
}

#[test]
fn learns_joined_words_and_split_names_from_an_edit() {
    for (before, after) in [
        ("I use type script.", "I use TypeScript."),
        ("Push to git hub.", "Push to GitHub."),
        ("Ask Papon Ghosh.", "Ask Papan Ghosh."),
        ("Ask Maryjane.", "Ask Mary Jane."),
    ] {
        let suggestions = correction_suggestions(before, after);
        assert_eq!(suggestions.len(), 1, "{before} -> {after}");
    }
}

#[test]
fn vocabulary_enforces_saved_capitalization() {
    let terms = vec![("TypeScript".into(), "TypeScript".into())];
    assert_eq!(
        TextCleaner::apply_glossary("use typescript", &terms),
        "use TypeScript"
    );
}

#[test]
fn dictionary_outputs_are_literal_and_longer_phrases_win() {
    let terms = vec![
        ("git".into(), "Git".into()),
        ("git hub".into(), "GitHub$1".into()),
    ];
    assert_eq!(TextCleaner::apply_glossary("git hub", &terms), "GitHub$1");
}

#[test]
fn does_not_learn_changes_in_meaning_or_added_prose() {
    for (before, after) in [
        ("Send it tomorrow.", "Send it Friday."),
        ("Buy the item.", "Sell the item."),
        ("Use TypeScript.", "Use TypeScript. Also run the tests."),
        ("Send it.", "Send it!"),
    ] {
        assert!(correction_suggestions(before, after).is_empty());
    }
}
