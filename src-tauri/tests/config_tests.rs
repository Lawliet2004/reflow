//! Milestone 0 / Task 3: `resolve_intent()` is the single authority, and the
//! persisted document can never contradict itself.

use reflow_lib::settings::{AppSettings, SettingsStore, CURRENT_SETTINGS_VERSION};

/// Every combination reachable through the UI or a persisted file must resolve
/// to a self-consistent intent. This is the property the old
/// independent-field scheme violated for the shipped default.
#[test]
fn resolve_intent_is_self_consistent_for_every_reachable_combination() {
    let tiers = [
        "",
        "raw_verbatim",
        "smart_flow",
        "deep_context",
        "bogus",
        "SMART_FLOW",
    ];
    let cleanups = ["", "raw", "light", "medium", "high", "bogus", "HIGH"];
    let modes = ["", "raw", "smart", "flow", "bogus"];

    for tier in tiers {
        for cleanup in cleanups {
            for mode in modes {
                let s = AppSettings {
                    intelligence_tier: tier.into(),
                    cleanup_level: cleanup.into(),
                    processing_mode: mode.into(),
                    ..AppSettings::default()
                };
                let intent = s.resolve_intent();
                let label = format!("tier={tier:?} cleanup={cleanup:?} mode={mode:?}");

                assert!(
                    ["raw_verbatim", "smart_flow", "deep_context"].contains(&intent.tier.as_str()),
                    "{label}: unknown tier {}",
                    intent.tier
                );

                // The cleanup level is always one of the four known values.
                assert!(
                    ["raw", "light", "medium", "high"].contains(&intent.cleanup_level.as_str()),
                    "{label}: unknown cleanup level {}",
                    intent.cleanup_level
                );

                // `run_llm` is determined by the tier, with one exception:
                // `cleanup_level: raw` is a verbatim request and suppresses it.
                // Crucially `light` does *not* — that was the original bug.
                let expected_run_llm =
                    intent.tier != "raw_verbatim" && intent.cleanup_level != "raw";
                assert_eq!(
                    intent.run_llm, expected_run_llm,
                    "{label}: run_llm={} for tier {} + cleanup {}",
                    intent.run_llm, intent.tier, intent.cleanup_level
                );

                if intent.tier == "raw_verbatim" {
                    assert_eq!(intent.flow_model, "none", "{label}");
                    assert!(
                        matches!(intent.cleanup_level.as_str(), "raw" | "light"),
                        "{label}: verbatim resolved to an aggressive cleanup {}",
                        intent.cleanup_level
                    );
                } else {
                    assert_ne!(intent.flow_model, "none", "{label}");
                }

                if intent.run_llm {
                    assert!(
                        intent.cleanup_level != "raw",
                        "{label}: refinement over verbatim text"
                    );
                }

                // Normalizing is idempotent and reproduces the same intent.
                let once = s.clone().normalized();
                let twice = once.clone().normalized();
                assert_eq!(once.intelligence_tier, twice.intelligence_tier, "{label}");
                assert_eq!(once.cleanup_level, twice.cleanup_level, "{label}");
                assert_eq!(once.processing_mode, twice.processing_mode, "{label}");
                assert_eq!(once.flow_model, twice.flow_model, "{label}");
                assert_eq!(once.resolve_intent(), intent, "{label}");
                assert_eq!(once.settings_version, CURRENT_SETTINGS_VERSION, "{label}");
            }
        }
    }
}

/// The headline bug: a fresh install advertised smart_flow refinement and never
/// ran it, because `cleanup_level: light` doubled as the Stage 2 gate.
///
/// Fresh installs request Natural cleanup, with refinement enabled by the tier.
#[test]
fn fresh_install_actually_runs_refinement() {
    let defaults = AppSettings::default();
    assert_eq!(defaults.cleanup_level, "medium");
    assert_eq!(defaults.style, "faithful");
    assert!(!defaults.auto_style_from_app);

    let intent = defaults.resolve_intent();
    assert_eq!(intent.tier, "smart_flow");
    assert_eq!(intent.flow_model, "qwen3.5-0.8b");
    assert_eq!(intent.cleanup_level, "medium");
    assert!(
        intent.run_llm,
        "the default install must run its Natural cleanup pass"
    );
}

/// A legacy schema-1 document with the contradictory shipped default must
/// migrate on load, without user action, and stay migrated.
#[test]
fn legacy_config_migrates_without_user_action() {
    let dir = std::env::temp_dir().join(format!("reflow_cfg_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.json");

    let mut value = serde_json::to_value(AppSettings::default()).unwrap();
    let obj = value.as_object_mut().unwrap();
    obj.remove("settings_version");
    obj.insert("intelligence_tier".into(), "smart_flow".into());
    obj.insert("cleanup_level".into(), "light".into());
    obj.insert("processing_mode".into(), "smart".into());
    std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

    let loaded: AppSettings =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(loaded.settings_version, 1);
    assert!(loaded.needs_migration());
    assert!(
        !loaded.resolve_intent().cleanup_level.is_empty(),
        "precondition"
    );

    let migrated = SettingsStore::new(path.clone()).get();
    assert_eq!(migrated.settings_version, CURRENT_SETTINGS_VERSION);
    assert!(!migrated.needs_migration());
    assert!(migrated.resolve_intent().run_llm);
    assert_eq!(
        migrated.cleanup_level, "light",
        "migration preserves the Stage 1 choice"
    );

    // Persisted, so the next launch is a no-op.
    let reloaded = SettingsStore::new(path).get();
    assert_eq!(reloaded.settings_version, CURRENT_SETTINGS_VERSION);
    assert_eq!(reloaded.cleanup_level, "light");
    assert!(!reloaded.needs_migration());

    let _ = std::fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------
// Task 9: ASR and refinement compute settings are independent.
// ---------------------------------------------------------------------------

/// The configuration a 4 GB card needs: ASR on CUDA, refinement on CPU. A single
/// `compute_backend` field could not express this at all.
#[test]
fn asr_and_refinement_devices_are_independent_and_both_honoured() {
    let dir = std::env::temp_dir().join(format!("reflow_split_{}", uuid::Uuid::new_v4()));
    let path = dir.join("settings.json");
    let store = SettingsStore::new(path.clone());

    let updated = store
        .merge_update(serde_json::json!({
            "memory_policy": { "allow_gpu_refinement": true },
            "asr": { "device": "cuda", "precision": "bf16" },
            "refinement": { "device": "cpu", "gpu_layers": 0 },
        }))
        .unwrap();

    assert_eq!(updated.asr.device, "cuda");
    assert_eq!(updated.asr.precision, "bf16");
    assert_eq!(updated.refinement.device, "cpu");
    assert_eq!(updated.refinement.gpu_layers, 0);

    // And the reverse split: ASR on CPU while refinement uses the GPU.
    let flipped = store
        .merge_update(serde_json::json!({
            "asr": { "device": "cpu" },
            "refinement": { "device": "cuda", "gpu_layers": 16 },
        }))
        .unwrap();
    assert_eq!(flipped.asr.device, "cpu");
    assert_eq!(flipped.refinement.device, "cuda");
    assert_eq!(flipped.refinement.gpu_layers, 16);
    // The ASR precision set earlier must survive a refinement-only patch.
    assert_eq!(flipped.asr.precision, "bf16");

    // Both survive a restart.
    let reloaded = SettingsStore::new(path).get();
    assert_eq!(reloaded.asr.device, "cpu");
    assert_eq!(reloaded.refinement.device, "cuda");
    assert_eq!(reloaded.refinement.gpu_layers, 16);

    let _ = std::fs::remove_dir_all(dir);
}

/// A patch that names one field of a nested section must not reset the others.
#[test]
fn a_nested_patch_merges_rather_than_replaces() {
    let dir = std::env::temp_dir().join(format!("reflow_nested_{}", uuid::Uuid::new_v4()));
    let store = SettingsStore::new(dir.join("settings.json"));

    let base = store
        .merge_update(serde_json::json!({
            "asr": { "model": "0.6b", "device": "cuda", "precision": "int8", "keep_loaded": false }
        }))
        .unwrap();
    assert_eq!(base.asr.model, "0.6b");
    assert!(!base.asr.keep_loaded);

    let patched = store
        .merge_update(serde_json::json!({ "asr": { "precision": "bf16" } }))
        .unwrap();
    assert_eq!(patched.asr.precision, "bf16");
    assert_eq!(
        patched.asr.model, "0.6b",
        "model was reset by a precision patch"
    );
    assert_eq!(patched.asr.device, "cuda", "device was reset");
    assert!(!patched.asr.keep_loaded, "keep_loaded was reset");

    let _ = std::fs::remove_dir_all(dir);
}

/// Migration from the schema-2 flat layout.
#[test]
fn schema_2_flat_compute_fields_migrate_into_nested_sections() {
    let dir = std::env::temp_dir().join(format!("reflow_v2_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.json");

    // A real schema-2 document: one backend governing both stages.
    let legacy = serde_json::json!({
        "settings_version": 2,
        "hotkey": "Shift+Win",
        "push_to_talk": true,
        "auto_stop_silence_ms": 1500,
        "max_duration_sec": 60,
        "language": "auto",
        "auto_detect_language": true,
        "microphone_device_id": null,
        "input_gain": 1.0,
        "vad_sensitivity": 0.5,
        "processing_mode": "smart",
        "cleanup_level": "light",
        "intelligence_tier": "smart_flow",
        "flow_model": "qwen3.5-0.8b",
        "style": "neutral",
        "auto_style_from_app": true,
        "dictation_mode": "normal",
        "compute_backend": "gpu",
        "asr_model": "0.6b",
        "asr_precision": "int8",
        "flow_n_gpu_layers": 24,
        "keep_model_loaded": false,
        "history_retention": "30_days",
        "overlay_position": "bottom_center",
        "overlay_theme": "dark",
        "active_profile": "Default",
        "launch_at_startup": false,
        "start_minimized": false,
        "offline_mode": true,
        "spoken_punctuation_enabled": true,
        "filler_removal_enabled": true,
        "clipboard_restore_enabled": true,
        "custom_replacements": [],
        "dictionary_terms": []
    });
    std::fs::write(&path, serde_json::to_string_pretty(&legacy).unwrap()).unwrap();

    let migrated = SettingsStore::new(path.clone()).get();
    assert_eq!(migrated.settings_version, CURRENT_SETTINGS_VERSION);

    // The single backend seeded the ASR device.
    assert_eq!(migrated.asr.device, "cuda", "compute_backend 'gpu' -> cuda");
    assert_eq!(migrated.asr.model, "0.6b");
    assert_eq!(migrated.asr.precision, "int8");
    assert!(!migrated.asr.keep_loaded, "keep_model_loaded carried over");

    // An explicit positive layer count is preserved, but the memory policy has
    // not authorised GPU refinement, so it is not applied yet.
    assert_eq!(migrated.refinement.model, "qwen3.5-0.8b");
    assert_eq!(
        migrated.refinement.device, "cpu",
        "GPU refinement stays off until the memory policy allows it"
    );
    assert_eq!(migrated.refinement.gpu_layers, 0);

    // New sections get their defaults.
    assert_eq!(migrated.streaming.silence_boundary_ms, 400);
    assert_eq!(migrated.refinement.context_size, 1024);
    assert!(migrated.refinement.keep_warm);

    // Persisted, so the next launch is a no-op.
    let reloaded = SettingsStore::new(path).get();
    assert_eq!(reloaded.settings_version, CURRENT_SETTINGS_VERSION);
    assert_eq!(reloaded.asr.device, "cuda");
    assert!(!reloaded.needs_migration());

    let _ = std::fs::remove_dir_all(dir);
}

/// A legacy `compute_backend: "cpu"` must not become "auto" and quietly move
/// ASR onto the GPU on the user's next launch.
#[test]
fn a_legacy_cpu_backend_stays_on_cpu_after_migration() {
    let dir = std::env::temp_dir().join(format!("reflow_v2cpu_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.json");

    let mut value = serde_json::to_value(AppSettings::default()).unwrap();
    {
        let obj = value.as_object_mut().unwrap();
        obj.insert("settings_version".into(), 2.into());
        obj.remove("asr");
        obj.remove("refinement");
        obj.insert("compute_backend".into(), "cpu".into());
    }
    std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

    let migrated = SettingsStore::new(path).get();
    assert_eq!(migrated.asr.device, "cpu");

    let _ = std::fs::remove_dir_all(dir);
}

/// The legacy flat vocabulary still works as a patch, because the LAN API and
/// any older frontend speak it.
#[test]
fn legacy_flat_patch_keys_still_reach_the_nested_sections() {
    let dir = std::env::temp_dir().join(format!("reflow_flatpatch_{}", uuid::Uuid::new_v4()));
    let store = SettingsStore::new(dir.join("settings.json"));

    let updated = store
        .merge_update(serde_json::json!({
            "compute_backend": "cpu",
            "asr_precision": "bf16",
            "asr_model": "0.6b",
            "keep_model_loaded": false
        }))
        .unwrap();
    assert_eq!(updated.asr.device, "cpu");
    assert_eq!(updated.asr.precision, "bf16");
    assert_eq!(updated.asr.model, "0.6b");
    assert!(!updated.asr.keep_loaded);

    // A legacy patch that only names the backend must leave the rest alone.
    let again = store
        .merge_update(serde_json::json!({ "compute_backend": "gpu" }))
        .unwrap();
    assert_eq!(again.asr.device, "cuda");
    assert_eq!(again.asr.precision, "bf16", "precision was reset");
    assert_eq!(again.asr.model, "0.6b", "model was reset");

    let _ = std::fs::remove_dir_all(dir);
}

/// The tier and `refinement.model` are the same choice expressed twice, so one
/// must be the authority and they must never drift.
#[test]
fn refinement_model_always_agrees_with_the_tier() {
    let dir = std::env::temp_dir().join(format!("reflow_tiersync_{}", uuid::Uuid::new_v4()));
    let store = SettingsStore::new(dir.join("settings.json"));

    for (tier, expected) in [
        ("raw_verbatim", "none"),
        ("smart_flow", "qwen3.5-0.8b"),
        ("deep_context", "qwen3.5-2b"),
    ] {
        let updated = store
            .merge_update(serde_json::json!({ "intelligence_tier": tier }))
            .unwrap();
        assert_eq!(updated.refinement.model, expected, "tier {tier}");
        assert_eq!(
            updated.refinement.model,
            updated.resolve_intent().flow_model,
            "tier {tier}: refinement.model drifted from the resolved intent"
        );
    }

    let _ = std::fs::remove_dir_all(dir);
}

/// GPU refinement requires explicit authorisation, because an unmeasured GPU
/// load can spill into shared system memory silently.
#[test]
fn gpu_refinement_requires_the_memory_policy_to_allow_it() {
    let dir = std::env::temp_dir().join(format!("reflow_gpupolicy_{}", uuid::Uuid::new_v4()));
    let store = SettingsStore::new(dir.join("settings.json"));

    let denied = store
        .merge_update(serde_json::json!({
            "refinement": { "device": "cuda", "gpu_layers": 20 }
        }))
        .unwrap();
    assert_eq!(denied.refinement.device, "cpu");
    assert_eq!(denied.refinement.gpu_layers, 0);

    let allowed = store
        .merge_update(serde_json::json!({
            "memory_policy": { "allow_gpu_refinement": true },
            "refinement": { "device": "cuda", "gpu_layers": 20 }
        }))
        .unwrap();
    assert_eq!(allowed.refinement.device, "cuda");
    assert_eq!(allowed.refinement.gpu_layers, 20);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_cpu_refinement_device_never_keeps_gpu_layers() {
    let settings = AppSettings {
        memory_policy: reflow_lib::settings::MemoryPolicySettings {
            allow_gpu_refinement: true,
            ..Default::default()
        },
        refinement: reflow_lib::settings::RefinementSettings {
            device: "cpu".into(),
            gpu_layers: 32,
            ..Default::default()
        },
        ..AppSettings::default()
    }
    .normalized();
    assert_eq!(settings.refinement.gpu_layers, 0);
}

/// Every historically shipped config shape must round-trip. Fields absent from
/// an old document take their defaults and then get normalized.
#[test]
fn every_historic_config_shape_round_trips() {
    let removals: Vec<Vec<&str>> = vec![
        vec![],
        vec!["settings_version"],
        vec!["settings_version", "cleanup_level"],
        vec!["settings_version", "intelligence_tier", "flow_model"],
        vec!["settings_version", "asr_precision", "flow_n_gpu_layers"],
        vec![
            "settings_version",
            "cleanup_level",
            "intelligence_tier",
            "flow_model",
            "style",
            "auto_style_from_app",
            "developer_mode",
            "app_theme",
            "accent_color",
            "hud_scale",
            "waveform_style",
            "reduce_motion",
            "ui_font_scale",
            "asr_precision",
            "flow_n_gpu_layers",
            "api_enabled",
            "api_bind",
            "api_port",
            "api_mdns",
            "api_inject_default",
        ],
    ];

    for removed in removals {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        {
            let obj = value.as_object_mut().unwrap();
            for key in &removed {
                obj.remove(*key);
            }
        }
        let loaded: AppSettings = serde_json::from_value(value)
            .unwrap_or_else(|err| panic!("shape missing {removed:?} failed to load: {err}"));
        let normalized = loaded.normalized();
        let intent = normalized.resolve_intent();
        assert!(
            ["raw_verbatim", "smart_flow", "deep_context"].contains(&intent.tier.as_str()),
            "shape missing {removed:?} produced tier {}",
            intent.tier
        );
        assert_eq!(normalized.settings_version, CURRENT_SETTINGS_VERSION);
    }
}

/// Selecting a tier sets the Stage 2 model and gate, and nothing else.
#[test]
fn selecting_a_tier_sets_the_stage_two_gate_only() {
    let dir = std::env::temp_dir().join(format!("reflow_cfg_tier_{}", uuid::Uuid::new_v4()));
    let store = SettingsStore::new(dir.join("settings.json"));

    // Start from an explicit Stage 1 choice.
    let base = store
        .merge_update(serde_json::json!({ "cleanup_level": "medium" }))
        .unwrap();
    assert_eq!(base.cleanup_level, "medium");

    for (tier, expected_model, expect_llm) in [
        ("deep_context", "qwen3.5-2b", true),
        ("smart_flow", "qwen3.5-0.8b", true),
    ] {
        let updated = store
            .merge_update(serde_json::json!({ "intelligence_tier": tier }))
            .unwrap();
        assert_eq!(updated.intelligence_tier, tier);
        assert_eq!(updated.flow_model, expected_model, "tier {tier}");
        assert_eq!(updated.resolve_intent().run_llm, expect_llm, "tier {tier}");
        assert_eq!(
            updated.cleanup_level, "medium",
            "tier {tier} must not silently change the Stage 1 level"
        );
    }

    // Verbatim is the one tier that constrains Stage 1, because an aggressive
    // rewrite contradicts asking for your words back unchanged.
    let verbatim = store
        .merge_update(serde_json::json!({ "intelligence_tier": "raw_verbatim" }))
        .unwrap();
    assert_eq!(verbatim.flow_model, "none");
    assert!(!verbatim.resolve_intent().run_llm);
    assert_eq!(verbatim.cleanup_level, "light");

    let _ = std::fs::remove_dir_all(dir);
}

/// Changing the cleanup level must never turn refinement off. That would be
/// the same class of surprise as the original bug, inverted.
#[test]
fn changing_cleanup_level_does_not_disable_refinement() {
    let dir = std::env::temp_dir().join(format!("reflow_cfg_clean_{}", uuid::Uuid::new_v4()));
    let store = SettingsStore::new(dir.join("settings.json"));

    let initial = store.get();
    assert_eq!(initial.intelligence_tier, "smart_flow");
    assert!(initial.resolve_intent().run_llm);

    for level in ["light", "medium", "high"] {
        let updated = store
            .merge_update(serde_json::json!({ "cleanup_level": level }))
            .unwrap();
        assert_eq!(updated.cleanup_level, level, "level {level}");
        assert_eq!(
            updated.intelligence_tier, "smart_flow",
            "cleanup {level} must not rewrite the tier"
        );
        assert!(
            updated.resolve_intent().run_llm,
            "cleanup {level} must not disable refinement"
        );
    }

    // `raw` is the deliberate exception: asking for verbatim words suppresses
    // refinement, but it still does not rewrite the tier, so switching back to
    // any other level restores it.
    let raw = store
        .merge_update(serde_json::json!({ "cleanup_level": "raw" }))
        .unwrap();
    assert_eq!(raw.intelligence_tier, "smart_flow");
    assert!(!raw.resolve_intent().run_llm);
    let restored = store
        .merge_update(serde_json::json!({ "cleanup_level": "light" }))
        .unwrap();
    assert!(restored.resolve_intent().run_llm);

    let _ = std::fs::remove_dir_all(dir);
}

/// The legacy `processing_mode` field still maps onto a cleanup level.
#[test]
fn legacy_processing_mode_still_maps_to_a_cleanup_level() {
    let dir = std::env::temp_dir().join(format!("reflow_cfg_legacy_{}", uuid::Uuid::new_v4()));
    let store = SettingsStore::new(dir.join("settings.json"));

    for (mode, expected) in [("raw", "raw"), ("flow", "medium"), ("smart", "light")] {
        let updated = store
            .merge_update(serde_json::json!({ "processing_mode": mode }))
            .unwrap();
        assert_eq!(updated.cleanup_level, expected, "mode {mode}");
        // And it still does not touch the tier.
        assert_eq!(updated.intelligence_tier, "smart_flow", "mode {mode}");
    }

    let _ = std::fs::remove_dir_all(dir);
}
