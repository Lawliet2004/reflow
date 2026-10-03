use reflow_lib::settings::{
    migrate_document, AppSettings, SettingsStore, CURRENT_SETTINGS_VERSION,
};

#[test]
fn v4_hotkey_migrates_and_legacy_patches_update_only_dictation() {
    let mut document = serde_json::to_value(AppSettings::default()).unwrap();
    let object = document.as_object_mut().unwrap();
    object.insert("settings_version".into(), 4.into());
    object.insert("hotkey".into(), "Ctrl+Shift+F9".into());
    object.remove("hotkeys");
    object.remove("modes");
    assert_eq!(migrate_document(&mut document), 4);
    let settings: AppSettings = serde_json::from_value(document).unwrap();
    assert_eq!(settings.settings_version, CURRENT_SETTINGS_VERSION);
    assert_eq!(settings.hotkeys.dictation, "Ctrl+Shift+F9");
    assert_eq!(settings.modes.len(), 4);
    let dir = std::env::temp_dir().join(format!("reflow-migrate-{}", uuid::Uuid::new_v4()));
    let store = SettingsStore::new(dir.join("settings.json"));
    store
        .merge_update(serde_json::json!({"hotkeys": {"note": "Ctrl+Alt+N"}}))
        .unwrap();
    let updated = store
        .merge_update(serde_json::json!({"hotkey": "Ctrl+Shift+F9"}))
        .unwrap();
    assert_eq!(updated.hotkeys.dictation, "Ctrl+Shift+F9");
    assert_eq!(updated.hotkeys.note.as_deref(), Some("Ctrl+Alt+N"));
    let updated = store
        .merge_update(serde_json::json!({"hotkeys": {"dictation": "Ctrl+Shift+F8"}}))
        .unwrap();
    assert_eq!(updated.hotkey, "Ctrl+Shift+F8");
    std::fs::remove_dir_all(dir).unwrap();
}
