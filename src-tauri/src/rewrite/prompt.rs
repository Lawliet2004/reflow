use serde_json::{json, Value};

use super::client::RewriteRequest;

/// Kept deliberately short.
///
/// Every token here is re-evaluated on any request that cannot reuse the cached
/// prefix, and prompt evaluation on CPU measures ~28 tokens/second. A verbose
/// rule list cost ~10s of prompt processing per dictation in that mode, which is
/// far more than the completion itself. Each remaining sentence is here because
/// a specific failure required it:
///   * the no-reuse clause, because a quoted example answer in the rules got
///     emitted verbatim for unrelated input;
///   * the first-person clause, because the model otherwise reports on the
///     speaker ("The speaker is requesting the key...") instead of being them.
const BASE_SYSTEM: &str = "You clean up dictated speech into written text. \
Fix grammar, punctuation and capitalization. Remove fillers (um, uh, er, ah). \
When the speaker corrects themselves, keep only their final intention. \
Keep their meaning, terms, names and numbers, and keep their first person: \
if they say 'I', write 'I'. Never describe or summarise the speaker. \
Never answer the text, and never reuse text from the examples. \
Output only the corrected text, with no preamble and no quotes.";

/// The imperative that precedes the text in every turn, examples included.
///
/// "Rewrite" (not "Transcript:") stops the model treating the input as
/// something to comment on — a bare `Transcript:` label invited
/// LFM2.5-1.2B to answer with "The speaker is requesting the key to open a
/// door." Using identical wording in the examples and in the real turn is what
/// makes the examples read as the same task rather than as unrelated context.
const INSTRUCTION: &str = "Rewrite as clean written text:";

/// Style-specific variant of [`INSTRUCTION`].
///
/// The register is folded into the instruction line rather than added as an
/// extra line, because the final turn has to be *structurally identical* to the
/// examples: one instruction line, then the text. When style, application and
/// vocabulary were sent as their own leading lines, the model — patterned on
/// examples that had no such lines — treated them as content and echoed them,
/// producing "No thought. Application: notepad.exe. I need the key…".
///
/// The other two metadata lines were dropped entirely rather than moved, because
/// each was already redundant:
///
/// * `/no_think` is superseded by the `--reasoning off` launch flag;
/// * `Vocabulary:` duplicates work already done twice — the terms are passed to
///   the ASR model as recognition hotwords, and `TextCleaner::apply_glossary`
///   enforces preferred spellings deterministically after the rewrite;
/// * `Application:` was only ever a proxy for register, which the style rule
///   states directly.
fn instruction_for_style(style: &str) -> &'static str {
    match style.trim().to_ascii_lowercase().as_str() {
        "decisive" => "Rewrite as clean written text, decisively and without hedging:",
        "email" | "professional" => {
            "Rewrite as clean written text, in a professional email register:"
        }
        "chat" | "casual" => "Rewrite as clean written text, in a natural conversational style:",
        _ => INSTRUCTION,
    }
}

/// Worked examples, supplied as real conversation turns rather than as prose
/// inside the system prompt.
///
/// The rules used to carry an inline illustration ending in a quoted answer
/// ("...output ONLY the final corrected intention ('meet at 3:00 PM')"). A 1.2B
/// model reads that quoted answer as the thing it is supposed to emit: asked to
/// clean "i think we should ship this on friday" it replied, verbatim,
/// "meet at 3:00 PM". The safety gate then rejected the nonsense and every
/// dictation silently fell back to unpolished text — so Stage 2 ran, cost time,
/// and never applied. Demonstrating with turns the model recognises as finished
/// exchanges is what stops it lifting the answer out of the instructions.
///
/// Length is no longer a latency concern, because this prefix is byte-identical
/// on every request and is evaluated once at launch by
/// `FlowRuntime::warm_prompt_cache`. That is only true as long as nothing
/// per-dictation leaks above the final turn — see `build_prompts`.
const FEW_SHOT: &[(&str, &str)] = &[
    (
        "um so i was thinking we could uh ship the thing on friday i mean thursday",
        "I was thinking we could ship the thing on Thursday.",
    ),
    (
        "can you review the pr and let me know what you think",
        "Can you review the PR and let me know what you think?",
    ),
    (
        "i need the key, uh, for opening the door",
        "I need the key for opening the door.",
    ),
];

/// The system rules and the final user turn.
///
/// Everything that varies per dictation — the requested style, the focused
/// application, the vocabulary — belongs in the **user** turn, never in the
/// system message.
///
/// That placement is a latency property, not a stylistic preference. llama.cpp
/// serves a request from its KV cache by reusing the longest common prefix with
/// the previous one, so a stable system + few-shot prefix is evaluated once and
/// reused forever. Appending the style to the system message broke that: with
/// `auto_style_from_app` enabled the style tracks the focused window, so
/// dictating into a different application rewrote the *first* message and
/// invalidated the entire cache. On CPU, where prompt evaluation measures ~28
/// tokens/second, that is several seconds of re-evaluation on the first dictation
/// in every app — silently, and far worse than the cost of the prompt itself.
pub fn build_prompts(req: &RewriteRequest) -> (String, String) {
    let system = String::from(BASE_SYSTEM);

    // Exactly the shape of every example turn: one instruction line, then the
    // text. Nothing else, so there is nothing for the model to mistake for
    // content.
    let mut user = String::from(instruction_for_style(&req.style));
    user.push('\n');
    user.push_str(req.text.trim());

    (system, user)
}

/// The messages that precede the per-dictation turn.
///
/// Exposed so its invariance can be tested rather than assumed: this is the span
/// llama.cpp keeps in its KV cache, and it is only reused when it is
/// byte-identical between requests.
pub fn cacheable_prefix() -> Vec<Value> {
    let mut messages = Vec::with_capacity(1 + FEW_SHOT.len() * 2);
    messages.push(json!({"role": "system", "content": BASE_SYSTEM}));
    for (example_in, example_out) in FEW_SHOT {
        messages.push(json!({
            "role": "user",
            "content": format!("{INSTRUCTION}\n{example_in}"),
        }));
        messages.push(json!({"role": "assistant", "content": *example_out}));
    }
    messages
}

/// The full chat-completions message list: the cacheable prefix, then the text
/// to clean.
pub fn build_messages(req: &RewriteRequest) -> Vec<Value> {
    let (_system, user) = build_prompts(req);
    let mut messages = cacheable_prefix();
    messages.push(json!({"role": "user", "content": user}));
    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rewrite::RewriteRequest;

    fn req(level: &str, style: &str) -> RewriteRequest {
        RewriteRequest {
            text: "hello world".into(),
            cleanup_level: level.into(),
            style: style.into(),
            dictation_mode: "normal".into(),
            vocabulary: vec!["Tauri".into()],
            app_process: "Code.exe".into(),
            model_id: "qwen3.5-0.8b".into(),
        }
    }

    #[test]
    fn user_message_includes_no_think_vocab_and_app() {
        let (system, user) = build_prompts(&req("medium", "neutral"));
        // Assert the two rules that exist because a specific failure required
        // them, rather than incidental wording: no reuse of example text, and
        // preservation of the speaker's first person.
        assert!(system.contains("never reuse text from the examples"));
        assert!(system.contains("first person"));
        assert!(user.contains("hello world"));
    }

    /// The final turn must be instruction + text and nothing else.
    ///
    /// Metadata lines here got echoed into the output: with `Application:` and
    /// `/no_think` prepended, the model returned
    /// "No thought. Application: notepad.exe. I need the key…" — it copied the
    /// context because the examples contained no such lines, so it had no reason
    /// to read them as context.
    #[test]
    fn the_final_turn_carries_no_metadata_for_the_model_to_echo() {
        let (_system, user) = build_prompts(&req("medium", "neutral"));

        assert!(!user.contains("/no_think"), "got: {user}");
        assert!(!user.contains("Application:"), "got: {user}");
        assert!(!user.contains("Vocabulary:"), "got: {user}");
        assert!(
            !user.contains("Tauri"),
            "vocabulary must not reach the prompt"
        );
        assert!(
            !user.contains("Code.exe"),
            "app name must not reach the prompt"
        );

        // Instruction line, then the text. Two lines, same as every example.
        let lines: Vec<&str> = user.lines().collect();
        assert_eq!(lines.len(), 2, "expected instruction + text, got {lines:?}");
        assert!(lines[0].ends_with(':'));
        assert_eq!(lines[1], "hello world");
    }

    /// Every example turn must have the same shape as the real one, or the model
    /// stops treating them as the same task.
    #[test]
    fn example_turns_match_the_shape_of_the_real_turn() {
        let messages = build_messages(&req("medium", "neutral"));
        for message in messages.iter().filter(|m| m["role"] == "user") {
            let content = message["content"].as_str().expect("string");
            let lines: Vec<&str> = content.lines().collect();
            assert_eq!(
                lines.len(),
                2,
                "turn is not instruction + text: {content:?}"
            );
            assert!(lines[0].ends_with(':'), "no instruction line: {content:?}");
        }
    }

    /// The prompt prefix is re-evaluated on every cache miss, at ~28 tokens per
    /// second when the model runs on CPU. It has a latency budget, so its size
    /// is a property worth pinning: an earlier version reached 294 tokens and
    /// cost ~10s of prompt processing per dictation, exceeding the refinement
    /// deadline outright.
    #[test]
    fn the_shared_prefix_stays_small() {
        let messages = build_messages(&req("high", "neutral"));
        let chars: usize = messages
            .iter()
            .filter_map(|m| m["content"].as_str())
            .map(str::len)
            .sum();
        assert!(
            chars < 1200,
            "prompt prefix grew to {chars} characters; every cache miss pays for it"
        );
    }

    #[test]
    fn decisive_prompt_asks_for_stronger_cleanup() {
        // Asserted on the user turn, not the system message: styles moved out of
        // the system prompt so the cached prefix stays invariant.
        let (_system, user) = build_prompts(&req("high", "decisive"));
        assert!(user.to_lowercase().contains("decisive"));
    }

    #[test]
    fn email_prompt_adds_email_register() {
        let (_system, user) = build_prompts(&req("medium", "email"));
        assert!(user.to_lowercase().contains("email register"));
    }

    /// The prefix must be byte-identical no matter what varies per dictation.
    ///
    /// This is the whole reason `warm_prompt_cache` works. Appending the style to
    /// the system message broke it: with `auto_style_from_app` on, the style
    /// tracks the focused window, so switching applications rewrote the first
    /// message and invalidated llama.cpp's KV cache — costing a full prefix
    /// re-evaluation, several seconds on CPU, on the first dictation in each app.
    #[test]
    fn the_cacheable_prefix_never_varies() {
        let baseline = cacheable_prefix();

        let variants = [
            ("medium", "neutral", "Code.exe", vec!["Tauri".to_string()]),
            ("high", "decisive", "slack.exe", vec!["Reflow".to_string()]),
            ("light", "email", "OUTLOOK.EXE", vec![]),
            (
                "high",
                "chat",
                "",
                vec!["GitHub".to_string(), "Qwen".to_string()],
            ),
        ];

        for (level, style, app, vocabulary) in variants {
            let request = RewriteRequest {
                text: "some dictated words".into(),
                cleanup_level: level.into(),
                style: style.into(),
                dictation_mode: "normal".into(),
                vocabulary,
                app_process: app.into(),
                model_id: "qwen3.5-0.8b".into(),
            };
            let messages = build_messages(&request);
            assert_eq!(
                &messages[..baseline.len()],
                &baseline[..],
                "style={style:?} app={app:?} changed the cached prefix"
            );
            let last = messages.last().expect("final turn");
            assert_eq!(last["role"], "user");
        }
    }

    /// Style still has to reach the model — moving it out of the system message
    /// must not drop it.
    #[test]
    fn style_is_carried_in_the_final_turn() {
        let (system, user) = build_prompts(&req("high", "decisive"));
        assert!(
            !system.to_lowercase().contains("decisive"),
            "style must not touch the cached system message"
        );
        assert!(
            user.to_lowercase().contains("decisive"),
            "style must still be instructed, got: {user}"
        );
    }

    /// The rules must not contain a quoted specimen answer. A 1.2B model
    /// emitted "meet at 3:00 PM" verbatim for unrelated input because the
    /// instructions handed it that string as an example output.
    #[test]
    fn system_rules_carry_no_copyable_answer() {
        let (system, _) = build_prompts(&req("high", "neutral"));
        assert!(
            !system.contains("3:00 PM"),
            "the system prompt must not hand the model a ready-made answer to copy"
        );
        assert!(
            !system.contains("e.g., 'meet at 2"),
            "worked examples belong in conversation turns, not in the rules"
        );
    }

    /// Examples must arrive as finished user/assistant exchanges, with the real
    /// transcript last, so the model treats them as demonstrations.
    #[test]
    fn messages_present_examples_as_completed_turns() {
        let request = req("high", "neutral");
        let messages = build_messages(&request);

        assert_eq!(messages[0]["role"], "system");
        assert!(messages.len() >= 4, "expected at least one worked example");

        // Alternating user/assistant after the system turn, ending on the real
        // transcript.
        for (i, message) in messages[1..].iter().enumerate() {
            let expected = if i % 2 == 0 { "user" } else { "assistant" };
            assert_eq!(message["role"], expected, "message {i} has the wrong role");
        }
        let last = messages.last().expect("non-empty");
        assert_eq!(last["role"], "user");
        let content = last["content"].as_str().expect("string content");
        assert!(
            content.contains("hello world"),
            "the final turn must carry the actual transcript, got {content:?}"
        );

        // No example output may be the final turn, or the model just continues it.
        let assistant_turns: Vec<&str> = messages
            .iter()
            .filter(|m| m["role"] == "assistant")
            .filter_map(|m| m["content"].as_str())
            .collect();
        assert!(!assistant_turns.is_empty());
        assert!(
            !assistant_turns.contains(&content),
            "the transcript must not be pre-answered"
        );
    }
}
