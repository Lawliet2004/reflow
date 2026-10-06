//! Measures writing quality separately from safe fallback; uses no network model.
use reflow_lib::rewrite::{flow_gguf_path, llama_server_bin, safety::accept_rewrite, FlowRuntime};
use reflow_lib::session::postprocess_transcript;
use reflow_lib::settings::AppSettings;

#[test]
#[ignore = "requires the installed Qwen3.5 0.8B model and llama-server"]
fn writing_categories_record_quality_separately_from_safe_fallback() {
    let model = "qwen3.5-0.8b";
    assert!(llama_server_bin().is_file());
    assert!(flow_gguf_path(model).is_file());
    let runtime = FlowRuntime::default();
    runtime.ensure(model, "vulkan", None, 0, 4096).unwrap();
    runtime.warm_prompt_cache(model);
    let client = runtime.client.read().clone();
    let base = AppSettings {
        processing_mode: "flow".into(),
        cleanup_level: "high".into(),
        intelligence_tier: "smart_flow".into(),
        flow_model: model.into(),
        style: "faithful".into(),
        auto_style_from_app: false,
        ..AppSettings::default()
    };
    let cases = [
        ("explicit_paragraphs", "normal", "high", "faithful", "the north wing is ready period new paragraph the south wing still needs paint period"),
        ("automatic_paragraphs", "notes", "high", "faithful", "The draft is complete. The charts need labels. The meeting is on Thursday. The room is confirmed."),
        ("review", "notes", "high", "faithful", "i bought this lamp last week the light is warm but the base feels flimsy i would keep it for reading"),
        ("bullet_points", "normal", "high", "faithful", "Use bullet points for these three items: first two notebooks, second three pens, third a printed map."),
        ("numbered_points", "notes", "high", "faithful", "Use numbered points: first save the draft, second review the charts, third send the invoice."),
        ("email", "normal", "light", "email", "Hi Noura, the invoice is attached. Thanks, Maya."),
        ("grammar_agreement", "normal", "medium", "faithful", "my packages is ready"),
        ("grammar_regular_verb", "normal", "medium", "faithful", "she go to school every morning"),
        ("grammar_irregular_verb", "normal", "medium", "faithful", "i seen the result"),
        ("vocabulary", "normal", "medium", "faithful", "please send the git hub report using vs code"),
        ("comma", "normal", "medium", "faithful", "after the meeting we can call Noura but please do not send the invoice"),
        ("semicolon", "normal", "medium", "faithful", "the train is late semi colon we can wait period"),
        ("uncertainty_and_negation", "normal", "medium", "faithful", "i think maybe we should delay the launch but do not cancel the meeting on Friday"),
        ("decisive_intent", "normal", "high", "decisive", "I want to visit the museum, but maybe we should wait until Friday."),
        ("mixed_language", "normal", "medium", "faithful", "Amina ने 2 boxes भेजे लेकिन मुझे 3 नहीं चाहिए"),
    ];
    let mut results = Vec::new();
    let mut unsafe_categories = Vec::new();
    for (category, mode, level, style, source) in cases {
        let settings = AppSettings {
            dictation_mode: mode.into(),
            cleanup_level: level.into(),
            style: style.into(),
            ..base.clone()
        };
        let outcome = postprocess_transcript(source, &settings, "notepad", &client);
        let text = &outcome.final_text;
        let lower = text.to_lowercase();
        let quality_met = match category {
            "explicit_paragraphs" | "automatic_paragraphs" => text.contains("\n\n"),
            "review" => text.matches('.').count() >= 2 && lower.contains("would"),
            "bullet_points" => text.lines().filter(|line| line.starts_with("• ")).count() == 3,
            "numbered_points" => text.contains("1. Save") && text.contains("3. Send"),
            "email" => {
                text.contains("\n\n") && text.lines().next().is_some_and(|line| line == "Hi Noura,")
            }
            "grammar_agreement" => lower.contains("packages are ready"),
            "grammar_regular_verb" => lower.contains("she goes to school"),
            "grammar_irregular_verb" => {
                lower.contains("i saw the result") || lower.contains("i have seen the result")
            }
            "vocabulary" => text.contains("GitHub") && text.contains("VS Code"),
            "comma" => text.contains(',') && lower.contains("do not"),
            "semicolon" => text.contains(';') && !lower.contains("semi colon"),
            "uncertainty_and_negation" => {
                lower.contains("maybe") && lower.contains("not") && text.contains("Friday")
            }
            "decisive_intent" => {
                lower.contains("want to") && lower.contains("maybe") && text.contains("Friday")
            }
            "mixed_language" => {
                text.contains("Amina") && text.contains("नहीं") && text.contains("2 boxes")
            }
            _ => unreachable!(),
        };
        let safe = accept_rewrite(&outcome.smart, text, level).is_some();
        if !safe
            || (matches!(
                category,
                "explicit_paragraphs" | "bullet_points" | "numbered_points" | "vocabulary"
            ) && !quality_met)
        {
            unsafe_categories.push(category);
        }
        eprintln!(
            "{category}: quality={quality_met}, safe={safe}, model_used={}, error={:?}\n{text}",
            outcome.rewriter_used, outcome.rewriter_error
        );
        results.push(serde_json::json!({
            "category": category,
            "source": source,
            "prepared": outcome.smart,
            "output": text,
            "model_used": outcome.rewriter_used,
            "fallback_error": outcome.rewriter_error,
            "quality_met": quality_met,
            "safety_accepted": safe,
        }));
    }
    runtime.shutdown();
    if let Ok(path) = std::env::var("REFLOW_WRITING_AUDIT_PATH") {
        std::fs::write(path, serde_json::to_string_pretty(&results).unwrap()).unwrap();
    }
    assert!(
        unsafe_categories.is_empty(),
        "unsafe or broken deterministic structure: {unsafe_categories:?}"
    );
    // Model-dependent quality is intentionally measured, not silently counted
    // as passing when the validator merely retained the prepared sentence.
}
