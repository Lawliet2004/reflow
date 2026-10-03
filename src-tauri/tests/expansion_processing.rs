use reflow_lib::{
    rewrite::{tasks::command_instruction, FlowClient},
    session::{postprocess_transcript, session_vocabulary, SessionContext},
    settings::{AppSettings, Snippet},
};

#[test]
fn snippets_follow_replacements_and_remain_verbatim_at_final_output() {
    let mut settings = AppSettings {
        intelligence_tier: "raw_verbatim".into(),
        ..Default::default()
    };
    settings.snippets.push(Snippet {
        id: "a".into(),
        trigger: "insert my address".into(),
        expansion: "42 avenue\nFRANCE".into(),
        enabled: true,
    });
    let result = postprocess_transcript(
        "please insert my address",
        &settings,
        "",
        &FlowClient::new_missing(),
    );
    assert!(result.final_text.contains("42 avenue\nFRANCE"));
    assert!(session_vocabulary(&settings).contains(&"insert my address".into()));
}

#[test]
fn command_translation_resolves_catalogue_names_and_context_is_bounded() {
    assert!(command_instruction("translate to French.")
        .unwrap()
        .contains("French"));
    assert!(command_instruction("translate to xx-invalid").is_err());
    let context = SessionContext {
        clipboard: Some("日本語".repeat(1000)),
        window_title: Some("".into()),
        ..Default::default()
    };
    let inputs = context.inputs(1024);
    assert_eq!(inputs.len(), 1);
    assert!(inputs[0].1.chars().count() <= 683);
}
