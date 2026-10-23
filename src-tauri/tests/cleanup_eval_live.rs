//! Golden-corpus evaluation of the dictation cleanup pipeline.
//!
//! The fast test checks the corpus is self-consistent. The `#[ignore]`d test
//! runs every case through the real pipeline (deterministic cleaner, then the
//! installed local model) and prints input / expected / actual / pass-fail.
//! It never installs models. Run it with `node scripts/eval_cleanup.cjs`.

use reflow_lib::{
    rewrite::{flow_gguf_path, llama_server_bin, FlowRuntime},
    session::postprocess_transcript,
    settings::AppSettings,
};
use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/cleanup_cases.json");

fn cases() -> Vec<Value> {
    let corpus: Value = serde_json::from_str(FIXTURE).expect("cleanup_cases.json is valid JSON");
    corpus["cases"].as_array().expect("cases array").clone()
}

fn words(text: &str) -> usize {
    text.split_whitespace().count()
}

fn is_list_line(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("- ")
        || line.starts_with("• ")
        || line
            .split_once(". ")
            .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// Share of the text's letters written in `script`.
fn script_share(text: &str, script: &str) -> f64 {
    let (mut latin, mut devanagari, mut bengali) = (0usize, 0usize, 0usize);
    for ch in text.chars() {
        match ch {
            '\u{0900}'..='\u{097F}' if ch != '\u{0964}' => devanagari += 1,
            '\u{0980}'..='\u{09FF}' => bengali += 1,
            c if c.is_ascii_alphabetic() => latin += 1,
            _ => {}
        }
    }
    let total = (latin + devanagari + bengali).max(1) as f64;
    let count = match script {
        "devanagari" => devanagari,
        "bengali" => bengali,
        _ => latin,
    };
    count as f64 / total
}

/// Returns the reasons a case failed; empty means it passed.
fn evaluate(case: &Value, actual: &str) -> Vec<String> {
    let check = &case["check"];
    let input = case["input"].as_str().unwrap();
    let expected = case["expected"].as_str().unwrap();
    let mut failures = Vec::new();
    let lines: Vec<&str> = actual.lines().filter(|l| !l.trim().is_empty()).collect();
    if check["exact"].as_bool() == Some(true) && actual.trim() != expected.trim() {
        failures.push("not exactly the expected text".into());
    }
    if let Some(options) = check["one_of"].as_array() {
        if !options.iter().any(|o| o.as_str() == Some(actual.trim())) {
            failures.push("not one of the accepted texts".into());
        }
    }
    for needle in check["contains"].as_array().into_iter().flatten() {
        if !actual.contains(needle.as_str().unwrap()) {
            failures.push(format!("missing {needle}"));
        }
    }
    for needle in check["not_contains"].as_array().into_iter().flatten() {
        if actual.contains(needle.as_str().unwrap()) {
            failures.push(format!("contains forbidden {needle}"));
        }
    }
    for needle in check["must_keep_words"].as_array().into_iter().flatten() {
        if !actual
            .to_lowercase()
            .contains(&needle.as_str().unwrap().to_lowercase())
        {
            failures.push(format!("dropped {needle}"));
        }
    }
    if let Some(n) = check["numbered_items"].as_u64() {
        let numbered = (1..=n)
            .filter(|k| {
                lines
                    .iter()
                    .any(|l| l.trim_start().starts_with(&format!("{k}. ")))
            })
            .count() as u64;
        if numbered != n {
            failures.push(format!("expected {n} numbered items, found {numbered}"));
        }
    }
    if let Some(n) = check["bullet_items"].as_u64() {
        let bullets = lines
            .iter()
            .filter(|l| l.trim_start().starts_with("- "))
            .count() as u64;
        if bullets != n {
            failures.push(format!("expected {n} bullets, found {bullets}"));
        }
    }
    if check["no_list"].as_bool() == Some(true) && lines.iter().any(|l| is_list_line(l)) {
        failures.push("made a list".into());
    }
    if let Some(n) = check["min_lines"].as_u64() {
        if (lines.len() as u64) < n {
            failures.push(format!("fewer than {n} lines"));
        }
    }
    if let Some(n) = check["max_lines"].as_u64() {
        if lines.len() as u64 > n {
            failures.push(format!("more than {n} lines"));
        }
    }
    if let Some(n) = check["min_paragraphs"].as_u64() {
        let paragraphs = actual
            .split("\n\n")
            .filter(|p| !p.trim().is_empty())
            .count() as u64;
        if paragraphs < n {
            failures.push(format!("{paragraphs} paragraphs, wanted at least {n}"));
        }
    }
    if let Some(ratio) = check["min_word_ratio"].as_f64() {
        if (words(actual) as f64) < ratio * words(input) as f64 {
            failures.push(format!("kept {}/{} words", words(actual), words(input)));
        }
    }
    if let Some(script) = check["script"].as_str() {
        // Code-switched text keeps its English words, so "kept the script"
        // means a substantial share, not a majority.
        let wanted = if script == "latin" { 0.9 } else { 0.3 };
        if script_share(actual, script) < wanted {
            failures.push(format!("lost the {script} script"));
        }
    }
    failures
}

#[test]
fn the_corpus_covers_every_required_case_and_its_expectations_satisfy_their_own_checks() {
    let cases = cases();
    let ids: Vec<&str> = cases.iter().map(|c| c["id"].as_str().unwrap()).collect();
    for required in [
        "self_correction",
        "negation_kept",
        "deliberate_repeat_kept",
        "accidental_repeat",
        "correction_and_time",
        "spoken_numbered_list",
        "four_item_bullets",
        "three_items_stay_prose",
        "question_not_answered",
        "command_not_obeyed",
        "actually_is_not_a_cue",
        "hindi_with_english_terms",
        "bengali_with_english_terms",
        "long_three_topics_chunked",
        "two_words",
    ] {
        assert!(ids.contains(&required), "missing case {required}");
    }
    let long = cases
        .iter()
        .find(|c| c["id"] == "long_three_topics_chunked")
        .unwrap();
    assert!(words(long["input"].as_str().unwrap()) > 400);
    for case in &cases {
        let id = case["id"].as_str().unwrap();
        let expected = case["expected"].as_str().unwrap();
        if id == "long_three_topics_chunked" {
            continue; // `expected` describes the shape; it is not output text.
        }
        let failures = evaluate(case, expected);
        assert!(failures.is_empty(), "{id}: {failures:?}");
    }
}

#[test]
#[ignore = "requires the installed Qwen3.5 GGUF and llama-server; run via scripts/eval_cleanup.cjs"]
fn cleanup_corpus_against_the_real_local_model() {
    let model = std::env::var("REFLOW_EVAL_MODEL").unwrap_or_else(|_| "qwen3.5-0.8b".into());
    let backend = std::env::var("REFLOW_EVAL_BACKEND").unwrap_or_else(|_| "cpu".into());
    assert!(
        flow_gguf_path(&model).is_file(),
        "GGUF not installed: {}",
        flow_gguf_path(&model).display()
    );
    assert!(llama_server_bin().is_file(), "llama-server not installed");
    let runtime = FlowRuntime::default();
    let layers = if backend == "cpu" { Some(0) } else { None };
    runtime.ensure(&model, &backend, layers, 0, 4096).unwrap();
    runtime.warm_prompt_cache(&model);
    let client = runtime.client.read().clone();
    let settings = AppSettings {
        intelligence_tier: "smart_flow".into(),
        flow_model: model.clone(),
        cleanup_level: "medium".into(),
        transcript_format: "auto".into(),
        auto_style_from_app: false,
        ..AppSettings::default()
    };
    let only = std::env::var("REFLOW_EVAL_ONLY").ok();
    let (mut passed, mut total) = (0, 0);
    let mut rewrite_ms = Vec::new();
    let mut deterministic_ms = Vec::new();
    for case in cases() {
        let id = case["id"].as_str().unwrap();
        if only.as_deref().is_some_and(|only| only != id) {
            continue;
        }
        let input = case["input"].as_str().unwrap();
        let outcome = postprocess_transcript(input, &settings, "preview", &client);
        let failures = evaluate(&case, &outcome.final_text);
        let ok = failures.is_empty();
        total += 1;
        passed += usize::from(ok);
        rewrite_ms.push(outcome.rewrite_ms.unwrap_or(0));
        deterministic_ms.push(outcome.formatting_ms);
        // The opt-in eval prints synthetic fixture content in full so omissions
        // in long dictations remain reviewable. Production logs never do this.
        eprintln!(
            "\n[{}] {id}\n  input:    {}\n  expected: {}\n  actual:   {}\n  llm_used={} fallback={:?} rewrite_ms={:?}{}",
            if ok { "PASS" } else { "FAIL" },
            input,
            case["expected"].as_str().unwrap().replace('\n', "\\n"),
            outcome.final_text.replace('\n', "\\n"),
            outcome.rewriter_used,
            outcome.fallback_reason,
            outcome.rewrite_ms,
            if ok { String::new() } else { format!("\n  why:      {}", failures.join("; ")) },
        );
    }
    rewrite_ms.sort_unstable();
    let median = rewrite_ms.get(rewrite_ms.len() / 2).copied().unwrap_or(0);
    let mean = rewrite_ms.iter().sum::<u64>() / rewrite_ms.len().max(1) as u64;
    eprintln!(
        "\nEVAL_SUMMARY passed={passed} total={total} rewrite_ms_median={median} rewrite_ms_mean={mean} \
         deterministic_ms_max={}",
        deterministic_ms.iter().max().copied().unwrap_or(0)
    );
    runtime.shutdown();
}
