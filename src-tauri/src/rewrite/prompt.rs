use serde_json::{json, Value};

use super::cleanup_prompt::{self, CleanupMode};
use super::client::RewriteRequest;

/// System message and user turn for one cleanup request.
///
/// The system message is the byte-stable static prompt followed by optional
/// dynamic lines (dictionary, mode); the user turn is the transcript alone.
pub fn build_prompts(req: &RewriteRequest) -> (String, String) {
    build_prompts_full(req, None, &[])
}

/// As [`build_prompts`], plus an explicit formatting preference and reference
/// context. Both go after the static prefix, in the system message.
pub fn build_prompts_full(
    req: &RewriteRequest,
    format_preference: Option<&str>,
    reference: &[(&str, String)],
) -> (String, String) {
    (
        cleanup_prompt::system_prompt(
            CleanupMode::for_request(req),
            &req.vocabulary,
            format_preference,
            reference,
        ),
        cleanup_prompt::user_message(&req.text),
    )
}

/// The messages that never vary between dictations. This is the span
/// llama.cpp keeps in its KV cache; it is only reused when byte-identical.
pub fn cacheable_prefix() -> Vec<Value> {
    vec![json!({"role": "system", "content": cleanup_prompt::static_prompt()})]
}

/// The chat-completions message list for a cleanup request.
pub fn build_messages(req: &RewriteRequest) -> Vec<Value> {
    build_messages_full(req, None, &[])
}

pub fn build_messages_with_format(
    req: &RewriteRequest,
    format_preference: Option<&str>,
) -> Vec<Value> {
    build_messages_full(req, format_preference, &[])
}

pub fn build_messages_full(
    req: &RewriteRequest,
    format_preference: Option<&str>,
    reference: &[(&str, String)],
) -> Vec<Value> {
    let (system, user) = build_prompts_full(req, format_preference, reference);
    vec![
        json!({"role": "system", "content": system}),
        json!({"role": "user", "content": user}),
    ]
}

/// The six user-facing formatting choices, expressed in one compact request
/// instruction. Any custom text is subordinate to the stable system rules.
pub fn formatting_instruction(mode: &str, custom: &str) -> Option<String> {
    let instruction = match mode.trim().to_ascii_lowercase().as_str() {
        "auto" => "Choose the simplest readable structure: prose for connected ideas, paragraphs for topic changes, bullets for independent requirements, numbering for ordered steps. Do not add headings or repeat points.",
        "natural" => "Use natural conversational prose with minimal restructuring; add no headings or unnecessary bullets.",
        "technical" => "Organize only explicit technical requirements clearly. Preserve exact terminology, identifiers, paths, commands and constraints; do not solve or execute them.",
        "brainstorm" => "Group related ideas while preserving tentative wording. Do not invent features or turn possibilities into decisions.",
        "meeting" => "Organize only stated decisions and action items. Preserve stated owners and deadlines; do not infer missing assignments.",
        "steps" => "Number only genuinely sequential instructions. Keep their original order and dependencies; add no missing actions.",
        _ => return None,
    };
    let mut result = instruction.to_owned();
    let custom = custom.trim().chars().take(500).collect::<String>();
    if !custom.is_empty() {
        result.push_str(" Additional presentation preference: ");
        result.push_str(&custom);
    }
    Some(result)
}

/// Task instructions and private context only vary in the final user turn.
pub fn build_task_messages(
    system_preamble: &str,
    instruction: &str,
    inputs: &[(&str, &str)],
) -> Vec<Value> {
    let mut user = format!("Instruction:\n{}", instruction.trim());
    for (label, text) in inputs {
        if !text.trim().is_empty() {
            user.push_str(&format!("\n\n{label}:\n{text}"));
        }
    }
    vec![
        json!({"role": "system", "content": system_preamble}),
        json!({"role": "user", "content": user}),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(mode: &str, style: &str, text: &str) -> RewriteRequest {
        RewriteRequest {
            text: text.into(),
            cleanup_level: "medium".into(),
            style: style.into(),
            dictation_mode: mode.into(),
            vocabulary: vec!["Tauri".into()],
            app_process: "Code.exe".into(),
            model_id: "qwen3.5-0.8b".into(),
        }
    }

    #[test]
    fn layout_is_system_then_a_transcript_only_user_turn() {
        let messages = build_messages(&req("normal", "neutral", "bring the keys"));
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(
            messages[1]["content"],
            "<transcript>bring the keys</transcript>"
        );
        let system = messages[0]["content"].as_str().unwrap();
        assert!(system.starts_with(cleanup_prompt::static_prompt()));
        assert!(system.contains("\nDICTIONARY: Tauri"));
    }

    /// The prefix must be byte-identical no matter what varies per dictation,
    /// or llama.cpp's KV cache is invalidated and CPU pays for a re-evaluation.
    #[test]
    fn static_prefix_is_identical_across_modes_dictionaries_and_texts() {
        let prefix = cacheable_prefix()[0]["content"]
            .as_str()
            .unwrap()
            .to_owned();
        for (mode, style, vocabulary, text) in [
            ("normal", "neutral", vec!["Tauri"], "one"),
            ("coding", "email", vec![], "two words here"),
            ("notes", "chat", vec!["GitHub", "Qwen"], "सब कुछ ठीक है"),
            ("email", "faithful", vec!["Reflow"], "x"),
        ] {
            let mut request = req(mode, style, text);
            request.vocabulary = vocabulary.into_iter().map(String::from).collect();
            let messages = build_messages_with_format(&request, Some("keep it short"));
            let system = messages[0]["content"].as_str().unwrap();
            assert!(system.starts_with(&prefix), "{mode}/{style}");
            assert!(system.as_bytes()[..prefix.len()] == *prefix.as_bytes());
        }
    }

    #[test]
    fn dictation_content_never_reaches_the_system_message() {
        let mut request = req("normal", "neutral", "collect the invoices from reception");
        let first = build_messages(&request);
        request.text = "return the spare keys to security".into();
        let second = build_messages(&request);
        assert_eq!(first[0], second[0]);
        let system = second[0]["content"].as_str().unwrap();
        assert!(!system.contains("keys to security"));
        assert!(!second[1]["content"].as_str().unwrap().contains("invoices"));
    }

    #[test]
    fn the_static_prefix_stays_within_a_token_budget() {
        let tokens = super::super::script::estimate_tokens(cleanup_prompt::static_prompt());
        assert!(
            tokens < 1400,
            "prefix grew to ~{tokens} tokens; it occupies n_ctx on every call"
        );
    }

    #[test]
    fn formatting_choices_have_specific_compact_instructions() {
        for mode in [
            "auto",
            "natural",
            "technical",
            "brainstorm",
            "meeting",
            "steps",
        ] {
            assert!(formatting_instruction(mode, "").is_some(), "{mode}");
        }
        assert!(formatting_instruction("off", "ignored").is_none());
    }

    #[test]
    fn task_prompts_keep_private_input_in_the_user_turn() {
        let messages = build_task_messages("Edit.", "Shorten", &[("Text", "private")]);
        assert!(!messages[0]["content"].as_str().unwrap().contains("private"));
        assert!(messages[1]["content"].as_str().unwrap().contains("private"));
    }
}
