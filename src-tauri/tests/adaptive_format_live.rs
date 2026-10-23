//! Opt-in measurement through the actual dictation pipeline. Never installs models.
use reflow_lib::{
    rewrite::{flow_gguf_path, llama_server_bin, FlowRuntime},
    session::postprocess_transcript,
    settings::AppSettings,
};

#[test]
#[ignore = "requires existing Qwen3.5 GGUF and llama-server; prints cold/warm measurements"]
fn adaptive_format_cold_and_warm_measurements() {
    let model = std::env::var("REFLOW_EVAL_MODEL").unwrap_or_else(|_| "qwen3.5-0.8b".into());
    assert!(matches!(model.as_str(), "qwen3.5-0.8b" | "qwen3.5-2b"));
    assert!(
        flow_gguf_path(&model).is_file(),
        "existing GGUF unavailable: {}",
        flow_gguf_path(&model).display()
    );
    assert!(
        llama_server_bin().is_file(),
        "existing llama-server unavailable"
    );
    let runtime = FlowRuntime::default();
    runtime.ensure(&model, "cpu", Some(0), 0, 4096).unwrap();
    let client = runtime.client.read().clone();
    let settings = AppSettings {
        intelligence_tier: if model.ends_with("2b") {
            "deep_context"
        } else {
            "smart_flow"
        }
        .into(),
        flow_model: model.clone(),
        cleanup_level: "medium".into(),
        transcript_format: "auto".into(),
        auto_style_from_app: false,
        ..AppSettings::default()
    };
    let cases = [
        "hey so um I was thinking we could meet tomorrow maybe around five pm and discuss the project and yeah bring your laptop if possible",
        "build a backend using node js no wait actually python fast api and postgres database jwt authentication password hashing and redis caching",
        "I want an ai browser that's lightweight uses less ram and maybe it can open websites search stuff do tasks and I think privacy should be important too",
        "rahul will finish frontend by friday priya will do the api integration testing is saturday and whole thing before monday",
        "improve my speech to text application make transcription faster use a small llm to correct mistakes format the text automatically recognize code and commands support normal writing coding meeting notes and keep it offline and don't change my meaning",
    ];
    for pass in 0..2 {
        for (case, source) in cases.iter().enumerate() {
            let started = std::time::Instant::now();
            let out = postprocess_transcript(source, &settings, "preview", &client);
            assert!(!out.final_text.is_empty());
            if !out.rewriter_used {
                assert_eq!(out.final_text, out.smart);
            }
            eprintln!(
                "{}",
                serde_json::json!({
                    "model": model, "pass": pass, "case": case,
                    "used": out.rewriter_used, "fallback_reason": out.rewriter_error,
                    "cleanup_ms": out.formatting_ms, "rewrite_ms": out.rewrite_ms,
                    "total_postprocess_ms": started.elapsed().as_millis(),
                    "generation": client.generation.read().clone(), "output": out.final_text,
                })
            );
        }
    }
}
