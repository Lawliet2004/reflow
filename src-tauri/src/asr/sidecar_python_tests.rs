use super::*;

#[test]
fn release_checkout_discovery_depends_on_the_executable_location() {
    let root = std::env::temp_dir().join(format!("reflow_checkout_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("src-tauri/target/release")).unwrap();
    std::fs::create_dir_all(root.join("model-runtime")).unwrap();
    std::fs::write(root.join("src-tauri/Cargo.toml"), "[package]").unwrap();
    std::fs::write(root.join("model-runtime/qwen3_asr_runtime.py"), "").unwrap();
    assert_eq!(
        Qwen3AsrSidecar::checkout_root(&root.join("src-tauri/target/release/reflow")),
        Some(root.clone())
    );
    assert_eq!(
        Qwen3AsrSidecar::checkout_root(&root.join("installed/reflow")),
        None
    );
    assert_eq!(
        Qwen3AsrSidecar::checkout_root(Path::new("/usr/bin/reflow")),
        None
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn interpreter_setting_is_additive_and_survives_unrelated_updates() {
    let root = std::env::temp_dir().join(format!("reflow_python_config_{}", uuid::Uuid::new_v4()));
    let store = crate::settings::SettingsStore::new(root.join("settings.json"));
    let original = store.get().asr;
    store
        .merge_update(json!({"asr":{"python_path":" /user/runtime/bin/python "}}))
        .unwrap();
    store.merge_update(json!({"paste_delay_ms":40})).unwrap();
    let mut saved = crate::settings::SettingsStore::new(root.join("settings.json"))
        .get()
        .asr;
    assert_eq!(
        saved.python_path.take().as_deref(),
        Some("/user/runtime/bin/python")
    );
    assert_eq!(saved, original);
    store
        .merge_update(json!({"asr":{"python_path":" "}}))
        .unwrap();
    assert_eq!(store.get().asr.python_path, None);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn development_python_prefers_venv_without_shell_activation() {
    let candidates =
        Qwen3AsrSidecar::python_candidates(None, None, Some(Path::new("/checkout")), false);
    let expected = Path::new("/checkout")
        .join(".venv")
        .join("bin/python")
        .to_string_lossy()
        .into_owned();
    assert_eq!(candidates, [expected.as_str(), "python3", "python"]);
    let mut attempted = Vec::new();
    let selected = Qwen3AsrSidecar::select_python(&candidates, |candidate| {
        attempted.push(candidate.to_string());
        Some(format!("{candidate}\n").into_bytes())
    });
    assert_eq!(selected.as_deref(), Some(expected.as_str()));
    assert_eq!(attempted, [expected]);
}

#[test]
fn active_environment_and_explicit_python_keep_their_priority() {
    let candidates = Qwen3AsrSidecar::python_candidates(
        None,
        Some(Path::new("/active")),
        Some(Path::new("/checkout")),
        false,
    );
    assert_eq!(
        candidates[0],
        Path::new("/active").join("bin/python").to_string_lossy()
    );
    assert_eq!(
        candidates[1],
        Path::new("/checkout")
            .join(".venv")
            .join("bin/python")
            .to_string_lossy()
    );
    let candidates = Qwen3AsrSidecar::python_candidates(
        Some("/chosen/python"),
        Some(Path::new("/active")),
        Some(Path::new("/checkout")),
        false,
    );
    assert_eq!(candidates, ["/chosen/python"]);
    assert!(Qwen3AsrSidecar::select_python(&candidates, |_| None).is_none());
}

#[test]
fn discovery_skips_unusable_environments_and_windows_store_aliases() {
    let candidates = Qwen3AsrSidecar::python_candidates(None, None, None, true);
    assert_eq!(candidates, ["python", "python3"]);
    let selected = Qwen3AsrSidecar::select_python(&candidates, |candidate| {
        Some(if candidate == "python" {
            br"C:\Users\test\AppData\Local\Microsoft\WindowsApps\python.exe".to_vec()
        } else {
            b"C:\\Python311\\python.exe\n".to_vec()
        })
    });
    assert_eq!(selected.as_deref(), Some(r"C:\Python311\python.exe"));
    let candidates =
        Qwen3AsrSidecar::python_candidates(None, None, Some(Path::new("checkout")), true);
    assert!(
        candidates[0].ends_with(".venv/Scripts/python.exe")
            || candidates[0].ends_with(r".venv\Scripts/python.exe")
    );
}

#[test]
fn missing_numpy_names_the_interpreter_and_requirements_instead_of_crash_loop() {
    let Some(python) = Qwen3AsrSidecar::find_python() else {
        return;
    };
    let root = std::env::temp_dir().join(format!("reflow_python_env_{}", uuid::Uuid::new_v4()));
    let result = Command::new(python)
        .args(["-m", "venv", "--without-pip"])
        .arg(&root)
        .output()
        .unwrap();
    if !result.status.success() {
        panic!(
            "Could not create isolated test Python: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let interpreter = root.join(if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    });
    let script = root.join("qwen3_asr_runtime.py");
    let error =
        Qwen3AsrSidecar::check_startup_dependencies(&interpreter.to_string_lossy(), &script)
            .unwrap_err();
    assert!(error.contains("'numpy' is missing"), "{error}");
    assert!(
        error.contains(&interpreter.to_string_lossy().to_string()),
        "{error}"
    );
    assert!(error.contains("-m pip install -r"), "{error}");
    assert!(error.contains("requirements.txt"), "{error}");
    assert!(!error.contains("unresponsive"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}
