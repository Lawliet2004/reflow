use reflow_lib::{
    context::AppContext,
    dory::{CaptureKind, SessionIntent},
    formatting::PunctuationInferer,
    rewrite::FlowClient,
    session::{hotkey_owns_session, postprocess_transcript, voice_command, VoiceCommand},
    settings::{AppSettings, SettingsStore},
    state::AppStateEnum,
};

#[test]
fn privacy_exclusions_match_exact_process_basenames() {
    let apps = vec!["C:\\Apps\\Private.EXE".into(), "/opt/secure/chat".into()];
    assert!(AppSettings::process_is_excluded("private.exe", &apps));
    assert!(AppSettings::process_is_excluded("/bin/CHAT", &apps));
    assert!(!AppSettings::process_is_excluded("notprivate.exe", &apps));
    assert!(!AppSettings::process_is_excluded("", &apps));
}
#[test]
fn invalid_options_do_not_replace_persisted_settings() {
    let directory = std::env::temp_dir().join(format!("reflow-options-{}", uuid::Uuid::new_v4()));
    let store = SettingsStore::new(directory.join("settings.json"));
    for patch in [
        serde_json::json!({"paste_delay_ms":2001}),
        serde_json::json!({"sounds_volume":1.1}),
        serde_json::json!({"inject_method":"shell"}),
        serde_json::json!({"min_dictation_ms":10001}),
    ] {
        assert!(store.merge_update(patch).is_err());
    }
    assert_eq!(store.get().paste_delay_ms, 30);
    let _ = std::fs::remove_dir_all(directory);
}
#[test]
fn disabling_initial_capitalization_preserves_continuation_and_later_sentences() {
    let settings = AppSettings {
        capitalize_first: false,
        intelligence_tier: "raw_verbatim".into(),
        cleanup_level: "light".into(),
        ..Default::default()
    };
    let text = postprocess_transcript(
        "continuing here. next sentence",
        &settings,
        "",
        &FlowClient::new_missing(),
    )
    .final_text;
    assert!(text.starts_with("continuing here."), "{text}");
    assert!(text.contains("Next sentence"), "{text}");
}
#[test]
fn whole_voice_commands_do_not_match_embedded_or_extended_phrases() {
    assert_eq!(
        voice_command(" Scratch that! "),
        Some(VoiceCommand::Scratch)
    );
    assert_eq!(voice_command("undo that"), Some(VoiceCommand::Undo));
    assert_eq!(voice_command("please undo that change"), None);
    assert_eq!(voice_command("scratch that thing"), None);
    assert!(PunctuationInferer::replace_spoken_punctuation(
        "open bracket value close bracket smiley"
    )
    .contains("[value] :)"));
}
#[test]
fn automation_capture_is_never_owned_by_a_physical_shortcut() {
    let directory =
        std::env::temp_dir().join(format!("reflow-automation-owner-{}", uuid::Uuid::new_v4()));
    let ctx = AppContext::bootstrap_test(directory.clone());
    *ctx.state_enum.write() = AppStateEnum::Recording;
    *ctx.capture_kind.write() = CaptureKind::Microphone;
    *ctx.session_intent.write() = SessionIntent::Dictate;
    ctx.session_context.write().automation = true;
    for binding in ["dictate", "mode:dictation", "command", "assistant", "note"] {
        assert!(!hotkey_owns_session(&ctx, binding));
    }
    drop(ctx);
    let _ = std::fs::remove_dir_all(directory);
}
