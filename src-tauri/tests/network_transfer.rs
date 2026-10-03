use reflow_lib::network_policy::{self, NetworkEntry};
use reflow_lib::settings::{AppSettings, OutputAction};
use reflow_lib::transfer::{bundle_patch, import_model_into, make_bundle};

fn temporary() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("reflow-transfer-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn offline_is_fail_closed_and_only_pinned_https_hosts_are_allowed() {
    assert_eq!(
        network_policy::require_online(true).unwrap_err(),
        "Offline mode is on"
    );
    assert!(network_policy::require_online(false).is_ok());
    assert!(network_policy::check_download().is_err());
    for invalid in [
        "http://huggingface.co/file",
        "https://huggingface.co.attacker.test/file",
        "https://user:secret@huggingface.co/file",
        "https://127.0.0.1/file",
    ] {
        assert!(
            network_policy::validate_download_url(&reqwest::Url::parse(invalid).unwrap()).is_err()
        );
    }
    assert!(network_policy::validate_download_url(
        &reqwest::Url::parse("https://huggingface.co/repo/resolve/pinned/model.gguf").unwrap()
    )
    .is_ok());
}

#[test]
fn config_roundtrip_preserves_local_privileges_and_merges_collections() {
    let mut current = AppSettings {
        offline_mode: true,
        api_enabled: true,
        ..Default::default()
    };
    current.modes[0].output = OutputAction::RunCommand {
        template: "trusted local tool".into(),
    };
    let mut bundle = make_bundle(&current).unwrap();
    assert!(bundle.settings.get("offline_mode").is_none());
    assert!(bundle.settings.get("api_enabled").is_none());
    assert!(matches!(bundle.modes[0].output, OutputAction::Paste));
    bundle.settings["language"] = "ja".into();
    bundle.modes.truncate(1);
    let patch = bundle_patch(&current, &bundle, false).unwrap();
    assert_eq!(patch["language"], "ja");
    assert!(patch.get("offline_mode").is_none());
    assert_eq!(patch["modes"][0]["output"]["type"], "run_command");
    assert_eq!(
        patch["modes"].as_array().unwrap().len(),
        current.modes.len()
    );
    assert_eq!(
        bundle_patch(&current, &bundle, true).unwrap()["modes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn config_rejects_versions_paths_privilege_changes_and_executable_actions() {
    let current = AppSettings::default();
    let original = make_bundle(&current).unwrap();
    let mut bundle = original.clone();
    bundle.version = 999;
    assert!(bundle_patch(&current, &bundle, false).is_err());
    let mut bundle = original.clone();
    bundle.settings["api_enabled"] = true.into();
    assert!(bundle_patch(&current, &bundle, false).is_err());
    let mut bundle = original.clone();
    bundle.modes[0].output = OutputAction::RunCommand {
        template: "arbitrary shell".into(),
    };
    assert!(bundle_patch(&current, &bundle, false)
        .unwrap_err()
        .contains("shell command"));
    let mut bundle = original;
    bundle.modes[0].output = OutputAction::AppendFile {
        path: "private machine path".into(),
    };
    assert!(bundle_patch(&current, &bundle, false).is_err());
}

#[test]
fn unknown_model_bytes_are_rejected_before_destination_changes() {
    let dir = temporary();
    let source = dir.join("Qwen3.5-0.8B-Q4_K_M.gguf");
    std::fs::write(&source, b"GGUF corrupt bytes").unwrap();
    let target = dir.join("models");
    assert!(import_model_into(&source, &target)
        .unwrap_err()
        .contains("SHA-256"));
    assert!(!target.exists());
    assert_eq!(std::fs::read(source).unwrap(), b"GGUF corrupt bytes");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn journal_reads_newest_bounded_entries_and_discards_untrusted_hosts() {
    let dir = temporary();
    let rows: Vec<_> = (0..25)
        .map(|bytes| NetworkEntry {
            timestamp: "2026-10-02T00:00:00Z".into(),
            host: "huggingface.co".into(),
            bytes,
        })
        .collect();
    let mut lines = rows
        .iter()
        .map(|row| serde_json::to_string(row).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    lines.push_str(
        "\nmalformed\n{\"timestamp\":\"now\",\"host\":\"secret/path?token=bad\",\"bytes\":1}\n",
    );
    std::fs::write(dir.join("network-journal.jsonl"), lines).unwrap();
    let result = network_policy::read_journal(&dir, 3).unwrap();
    assert_eq!(
        result.iter().map(|row| row.bytes).collect::<Vec<_>>(),
        [24, 23, 22]
    );
    std::fs::remove_dir_all(dir).unwrap();
}
