use reflow_lib::asr::engine::LoadFailureKind;
use reflow_lib::rewrite::client::{polish_or_fallback, FlowClient, RewriteRequest};
use std::time::Duration;

#[test]
fn test_load_failure_kind_classification() {
    let oom = LoadFailureKind::parse("oom");
    assert_eq!(oom, LoadFailureKind::Oom);
    assert!(oom.invalidates_benchmark());

    let spill = LoadFailureKind::parse("spill");
    assert_eq!(spill, LoadFailureKind::Spill);
    assert!(spill.invalidates_benchmark());
    assert!(spill.is_permanent());

    let unsupp = LoadFailureKind::parse("unsupported_precision");
    assert_eq!(unsupp, LoadFailureKind::UnsupportedPrecision);
    assert!(!unsupp.invalidates_benchmark());
    assert!(unsupp.is_permanent());
}

#[test]
fn test_llm_refinement_fallback_on_error_preserves_raw_text() {
    // FlowClient pointing to unreachable port to simulate sudden connection drop or OOM
    let client = FlowClient::new_url("http://127.0.0.1:59999".into(), Duration::from_millis(50));
    // Must need polishing: an already well-formed short sentence is skipped
    // before the client is reached, so it could not exercise a connection drop.
    let raw = "deploying the production application now";

    let req = RewriteRequest {
        text: raw.into(),
        cleanup_level: "smart".into(),
        style: "normal".into(),
        dictation_mode: "normal".into(),
        vocabulary: vec![],
        app_process: "app.exe".into(),
        model_id: "qwen3.5-0.8b".into(),
    };

    let outcome = polish_or_fallback(&client, raw, &req);

    // Verifies that on LLM error/OOM:
    // 1. used is false
    // 2. error is captured (not swallowed)
    // 3. raw / smart text is safely preserved for injection
    assert!(!outcome.used);
    assert!(outcome.error.is_some());
    assert_eq!(outcome.final_text, raw);
}
