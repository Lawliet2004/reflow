use reflow_lib::settings::{AppSettings, SettingsStore, CURRENT_SETTINGS_VERSION};

fn saved_settings(settings: &AppSettings) -> (std::path::PathBuf, std::path::PathBuf) {
    let directory =
        std::env::temp_dir().join(format!("reflow_platform_keys_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("settings.json");
    std::fs::write(&path, serde_json::to_vec_pretty(settings).unwrap()).unwrap();
    (directory, path)
}

#[test]
fn supported_cross_platform_hotkeys_are_preserved() {
    let mut settings = AppSettings {
        hotkey: "Ctrl+Shift+F9".into(),
        ..AppSettings::default()
    };
    settings.hotkeys.dictation = settings.hotkey.clone();
    settings.hotkeys.note = Some("Ctrl+Alt+N".into());
    let (directory, path) = saved_settings(&settings);
    let before = std::fs::read(&path).unwrap();
    let store = SettingsStore::new(path.clone());
    assert_eq!(store.get().hotkey, "Ctrl+Shift+F9");
    assert_eq!(store.get().hotkeys.note.as_deref(), Some("Ctrl+Alt+N"));
    assert_eq!(std::fs::read(path).unwrap(), before);
    assert!(store.recovery_notice().is_none());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn windows_modifier_only_shortcuts_do_not_block_linux_dictation() {
    let mut settings = AppSettings {
        hotkey: "Ctrl+Super".into(),
        ..AppSettings::default()
    };
    settings.hotkeys.dictation = settings.hotkey.clone();
    settings.hotkeys.note = Some("Shift+Win".into());
    settings.hotkeys.command = Some("Ctrl+Alt+K".into());
    settings.modes[0].hotkey = Some("Ctrl+Shift".into());
    let (directory, path) = saved_settings(&settings);
    let store = SettingsStore::new(path.clone());
    let loaded = store.get();
    assert_eq!(loaded.settings_version, CURRENT_SETTINGS_VERSION);
    #[cfg(not(windows))]
    {
        assert_eq!(loaded.hotkey, reflow_lib::platform::default_hotkey());
        assert_eq!(loaded.hotkeys.dictation, loaded.hotkey);
        assert!(loaded.hotkeys.note.is_none());
        assert!(loaded.modes[0].hotkey.is_none());
        assert_eq!(loaded.hotkeys.command.as_deref(), Some("Ctrl+Alt+K"));
        assert!(store.recovery_notice().unwrap().contains("Ctrl+Super"));
        reflow_lib::hotkey::registry::HotkeyRegistry::from_settings(&loaded).unwrap();
        let persisted: AppSettings =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted.hotkey, loaded.hotkey);
        assert!(persisted.hotkeys.note.is_none());
        assert!(SettingsStore::new(path).recovery_notice().is_none());
    }
    #[cfg(windows)]
    {
        assert_eq!(loaded.hotkey, "Ctrl+Super");
        assert_eq!(loaded.hotkeys.note.as_deref(), Some("Shift+Win"));
        assert_eq!(loaded.modes[0].hotkey.as_deref(), Some("Ctrl+Shift"));
        assert!(store.recovery_notice().is_none());
    }
    std::fs::remove_dir_all(directory).unwrap();
}
