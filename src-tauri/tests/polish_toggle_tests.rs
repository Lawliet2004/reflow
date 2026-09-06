//! The Fast/Polished switch, and whether it is actually remembered.
//!
//! The requirement is "sometimes I want speed with no LLM, sometimes I want
//! polish, and the app should still be in the mode I left it in next time I open
//! it". Persistence itself comes free — the tier lives in `settings.json` and
//! `session::stop_at` re-reads settings on every dictation — but *which* tier to
//! come back to does not: switching polish off rewrites `intelligence_tier` to
//! `raw_verbatim` and `normalized()` follows by rewriting `flow_model` to
//! `none`, so without somewhere to keep it, the user's choice of polishing tier
//! is destroyed by the act of turning polishing off.

use reflow_lib::settings::{AppSettings, SettingsStore};

fn temp_store() -> (SettingsStore, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("reflow_polish_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("settings.json");
    (SettingsStore::new(path.clone()), path)
}

/// Mirrors what `commands::set_polish_enabled` writes, so the persistence
/// contract can be tested without standing up a Tauri app.
fn apply_toggle(current: &AppSettings, enabled: bool) -> AppSettings {
    let current_tier = current.resolve_intent().tier;
    let (tier, last_polish_tier) = if enabled {
        let restore = if current
            .last_polish_tier
            .trim()
            .eq_ignore_ascii_case("raw_verbatim")
            || current.last_polish_tier.trim().is_empty()
        {
            "smart_flow".to_string()
        } else {
            current.last_polish_tier.trim().to_ascii_lowercase()
        };
        (restore.clone(), restore)
    } else {
        let remember = if current_tier == "raw_verbatim" {
            current.last_polish_tier.clone()
        } else {
            current_tier
        };
        ("raw_verbatim".to_string(), remember)
    };
    AppSettings {
        intelligence_tier: tier,
        last_polish_tier,
        ..current.clone()
    }
}

/// Off means Stage 2 genuinely does not run — not merely a lighter cleanup.
#[test]
fn turning_polish_off_disables_stage_two() {
    let settings = AppSettings::default();
    assert!(
        settings.resolve_intent().run_llm,
        "the default install should polish"
    );

    let off = apply_toggle(&settings, false);
    let intent = off.resolve_intent();
    assert!(!intent.run_llm, "polish off must stop Stage 2");
    assert_eq!(intent.flow_model, "none", "and must load no polish model");
}

/// The round trip must land back on the tier the user actually chose, not on the
/// default one.
#[test]
fn toggling_back_on_restores_the_tier_that_was_in_use() {
    let settings = AppSettings {
        intelligence_tier: "deep_context".into(),
        last_polish_tier: "deep_context".into(),
        ..AppSettings::default()
    };

    let off = apply_toggle(&settings, false);
    assert_eq!(off.intelligence_tier, "raw_verbatim");
    assert_eq!(
        off.last_polish_tier, "deep_context",
        "the tier in use must be remembered before it is overwritten"
    );

    let back_on = apply_toggle(&off, true);
    assert_eq!(
        back_on.intelligence_tier, "deep_context",
        "coming back on must restore the user's tier, not the default"
    );
    assert!(back_on.resolve_intent().run_llm);
}

/// Toggling off twice must not lose the remembered tier.
#[test]
fn disabling_twice_is_idempotent() {
    let settings = AppSettings {
        intelligence_tier: "deep_context".into(),
        last_polish_tier: "deep_context".into(),
        ..AppSettings::default()
    };

    let once = apply_toggle(&settings, false);
    let twice = apply_toggle(&once, false);
    assert_eq!(twice.intelligence_tier, "raw_verbatim");
    assert_eq!(
        twice.last_polish_tier, "deep_context",
        "a second disable must not overwrite the memory with raw_verbatim"
    );
    assert_eq!(apply_toggle(&twice, true).intelligence_tier, "deep_context");
}

/// The actual "next time I open the app" requirement: the mode has to survive a
/// fresh store reading the same file.
#[test]
fn the_mode_survives_a_restart() {
    let (store, path) = temp_store();

    let off = apply_toggle(&store.get(), false);
    store.update(off).expect("persist polish off");

    // A new store over the same file is what a restart looks like.
    let reopened = SettingsStore::new(path.clone());
    let loaded = reopened.get();
    assert!(
        !loaded.resolve_intent().run_llm,
        "polish must still be off after a restart"
    );

    let on = apply_toggle(&loaded, true);
    reopened.update(on).expect("persist polish on");

    let again = SettingsStore::new(path);
    assert!(
        again.get().resolve_intent().run_llm,
        "polish must still be on after a restart"
    );
}

/// Enabling from a document that has never polished must pick a real tier rather
/// than restoring `raw_verbatim` and appearing to do nothing.
#[test]
fn enabling_from_a_never_polished_document_picks_a_real_tier() {
    let settings = AppSettings {
        intelligence_tier: "raw_verbatim".into(),
        last_polish_tier: "raw_verbatim".into(),
        ..AppSettings::default()
    };

    let on = apply_toggle(&settings, true);
    assert_eq!(on.intelligence_tier, "smart_flow");
    assert!(on.resolve_intent().run_llm);
}
