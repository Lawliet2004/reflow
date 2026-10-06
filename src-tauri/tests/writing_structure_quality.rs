//! Independent writing categories: structure may improve, dictated facts stay.

use reflow_lib::formatting::{format_transcript, CustomReplacements, ReplacementRule};
use reflow_lib::rewrite::{
    prompt::build_prompts, safety::accept_rewrite, FlowClient, RewriteRequest,
};
use reflow_lib::session::postprocess_transcript;
use reflow_lib::settings::{AppSettings, DictionaryTerm};
use std::time::Duration;

fn prepare(raw: &str, mode: &str) -> String {
    format_transcript(
        raw,
        "high",
        mode,
        true,
        true,
        &CustomReplacements::new(vec![]),
    )
}

fn assert_accepted(original: &str, polished: &str) {
    assert_eq!(
        accept_rewrite(original, polished, "high").as_deref(),
        Some(polished)
    );
}

#[test]
fn explicit_paragraph_and_line_commands_keep_their_distinct_structure() {
    assert_eq!(
        prepare("first paragraph period new paragraph second paragraph period new line third line period", "normal"),
        "First paragraph.\n\nSecond paragraph.\nThird line."
    );
}

#[test]
fn spoken_comma_and_semicolon_form_the_dictated_clause_boundaries() {
    assert_eq!(
        prepare(
            "maya comma please bring bread semicolon Arun will bring muffins period",
            "normal"
        ),
        "Maya, please bring bread; Arun will bring muffins."
    );
}

#[test]
fn an_explicit_terminal_colon_remains_a_heading_delimiter() {
    assert_eq!(
        prepare("packing checklist colon", "notes"),
        "Packing checklist:"
    );
}

#[test]
fn a_literal_period_in_a_noun_phrase_is_not_a_punctuation_command() {
    assert_eq!(
        prepare("the trial period lasts 14 days", "normal"),
        "The trial period lasts 14 days."
    );
}

#[test]
fn a_question_in_one_paragraph_does_not_change_the_next_statement() {
    assert_eq!(
        prepare(
            "can you review the draft question mark new paragraph I will send the invoice",
            "normal"
        ),
        "Can you review the draft?\n\nI will send the invoice."
    );
}

#[test]
fn writing_bullets_keep_quantities_and_a_dictated_heading() {
    assert_eq!(
        prepare(
            "Packing checklist: first two notebooks, second three pens",
            "notes"
        ),
        "Packing checklist:\n\n• Two notebooks\n• Three pens"
    );
}

#[test]
fn an_explicit_numbered_list_uses_positions_without_changing_quantities() {
    assert_eq!(
        prepare(
            "numbered list: first pack two notebooks, second bring three pens",
            "notes"
        ),
        "Numbered list:\n\n1. Pack two notebooks\n2. Bring three pens"
    );
}

#[test]
fn paragraphs_can_improve_prose_without_adding_a_heading() {
    let original =
        "I finished the draft. The charts still need labels. I will send the revision tomorrow.";
    let polished =
        "I finished the draft.\n\nThe charts still need labels. I will send the revision tomorrow.";
    assert_accepted(original, polished);
    assert!(accept_rewrite(original, &format!("Project status:\n\n{polished}"), "high").is_none());
}

#[test]
fn an_email_can_separate_its_existing_greeting_body_and_signoff() {
    let original = "Hi Sarah, I will send the report tomorrow. Thanks, Papan.";
    let polished = "Hi Sarah,\n\nI will send the report tomorrow.\n\nThanks,\nPapan.";
    assert_accepted(original, polished);
    assert!(accept_rewrite(original, &polished.replace("Sarah", "Priya"), "high").is_none());
    assert!(accept_rewrite(original, &polished.replace("tomorrow", "today"), "high").is_none());
}

#[test]
fn a_review_can_gain_paragraphs_without_inventing_a_rating_or_verdict() {
    let original = "The service was friendly. The food was cold, and I would not return.";
    let polished = "The service was friendly.\n\nThe food was cold, and I would not return.";
    assert_accepted(original, polished);
    for invented in [
        format!("Five-star review:\n\n{polished}"),
        format!("Verdict:\n\n{polished}"),
        format!("{polished}\n\nHighly recommended."),
    ] {
        assert!(accept_rewrite(original, &invented, "high").is_none());
    }
}

#[test]
fn review_negation_stays_attached_to_the_same_opinion() {
    let original = "The service was good, but the food was not good.";
    let changed = "The service was not good, but the food was good.";
    assert!(accept_rewrite(original, changed, "high").is_none());
}

#[test]
fn mixed_language_punctuation_preserves_names_quantities_and_negation() {
    let original = "Amina ने 2 boxes भेजे लेकिन मुझे 3 नहीं चाहिए";
    let polished = "Amina ने 2 boxes भेजे, लेकिन मुझे 3 नहीं चाहिए.";
    assert_accepted(original, polished);
    for changed in [
        polished.replace("Amina", "Anita"),
        polished.replace("2 boxes", "4 boxes"),
        polished.replace("नहीं ", ""),
        "Amina sent 2 boxes, but I do not want 3.".into(),
    ] {
        assert!(accept_rewrite(original, &changed, "high").is_none());
    }
}

#[test]
fn grammar_fixes_keep_the_sentence_content_and_voice() {
    for (original, polished) in [
        (
            "They sends the invoice to Noura.",
            "They send the invoice to Noura.",
        ),
        ("My phone are broken", "My phone is broken."),
        ("My meeting has been cancelled", "My meeting was cancelled."),
    ] {
        assert_accepted(original, polished);
    }
}

#[test]
fn a_possession_cannot_be_rewritten_as_an_identity() {
    assert!(accept_rewrite("I have a car.", "I am a car.", "high").is_none());
}

#[test]
fn a_passive_rewrite_cannot_reverse_the_actor_and_recipient() {
    assert!(accept_rewrite("Alice helped Bob.", "Alice was helped by Bob.", "high").is_none());
}

#[test]
fn configured_vocabulary_is_applied_in_the_actual_postprocessing_pipeline() {
    let settings = AppSettings {
        cleanup_level: "light".into(),
        processing_mode: "smart".into(),
        auto_style_from_app: false,
        custom_replacements: vec![
            ReplacementRule {
                id: "project".into(),
                before: "nebula tracker".into(),
                after: "NebulaTrack".into(),
                enabled: true,
            },
            ReplacementRule {
                id: "disabled".into(),
                before: "ticket".into(),
                after: "case".into(),
                enabled: false,
            },
        ],
        dictionary_terms: vec![DictionaryTerm {
            id: "recipient".into(),
            term: "meenak shee".into(),
            preferred_spelling: "Meenakshi".into(),
            category: "people".into(),
        }],
        ..AppSettings::default()
    };
    let outcome = postprocess_transcript(
        "send the nebula tracker report to meenak shee comma do not change ticket ZX-19",
        &settings,
        "notepad",
        &FlowClient::new_missing(),
    );
    assert_eq!(
        outcome.smart,
        "Send the NebulaTrack report to Meenakshi, do not change ticket ZX-19."
    );
    assert_eq!(outcome.final_text, outcome.smart);
    assert!(!outcome.rewriter_used);
}

#[test]
fn a_grammar_refinement_does_not_apply_a_second_vocabulary_replacement() {
    let settings = AppSettings {
        cleanup_level: "high".into(),
        processing_mode: "flow".into(),
        auto_style_from_app: false,
        style: "faithful".into(),
        custom_replacements: vec![
            ReplacementRule {
                id: "first".into(),
                before: "alpha".into(),
                after: "beta".into(),
                enabled: true,
            },
            ReplacementRule {
                id: "second".into(),
                before: "beta".into(),
                after: "gamma".into(),
                enabled: true,
            },
        ],
        dictionary_terms: vec![],
        ..AppSettings::default()
    };
    let model = LocalModel::returning("My beta is ready.");
    let outcome =
        postprocess_transcript("My alpha are ready", &settings, "notepad", &model.client());

    assert_eq!(outcome.smart, "My beta are ready.");
    assert_eq!(outcome.final_text, "My beta is ready.");
    assert!(outcome.rewriter_used, "{:?}", outcome.rewriter_error);
    assert!(outcome.rewriter_error.is_none());
}

#[test]
fn vocabulary_spelling_does_not_change_path_or_email_identifiers() {
    let mut settings = AppSettings {
        cleanup_level: "light".into(),
        processing_mode: "smart".into(),
        auto_style_from_app: false,
        ..AppSettings::default()
    };
    settings.dictionary_terms.push(DictionaryTerm {
        id: "contact".into(),
        term: "meena".into(),
        preferred_spelling: "Meena".into(),
        category: "people".into(),
    });
    let raw = "send src/qwen.rs to meena@example.com and tell meena about qwen";

    assert_eq!(
        format_transcript(
            raw,
            "light",
            "normal",
            true,
            true,
            &CustomReplacements::new(settings.custom_replacements.clone()),
        ),
        "Send src/qwen.rs to meena@example.com and tell meena about Qwen."
    );
    let outcome = postprocess_transcript(raw, &settings, "notepad", &FlowClient::new_missing());
    assert_eq!(
        outcome.smart,
        "Send src/qwen.rs to meena@example.com and tell Meena about Qwen."
    );
    assert_eq!(outcome.final_text, outcome.smart);
    assert!(!outcome.rewriter_used);
}

#[test]
fn builtin_email_mode_requests_layout_even_with_its_light_defaults() {
    let settings = AppSettings::default();
    let mode = settings
        .modes
        .iter()
        .find(|mode| mode.id == "email")
        .expect("builtin Email mode");
    let effective = settings.settings_for_mode("notepad", mode);
    let intent = effective.resolve_intent();
    assert_eq!(effective.style, "email");
    assert_eq!(effective.dictation_mode, "normal");
    assert_eq!(intent.cleanup_level, "light");
    let request = RewriteRequest {
        text: "Hi Noura, the invoice is attached. Thanks, Maya.".into(),
        cleanup_level: intent.cleanup_level,
        style: effective.style,
        dictation_mode: effective.dictation_mode,
        vocabulary: vec![],
        app_process: "notepad".into(),
        model_id: intent.flow_model,
    };
    let (_, user) = build_prompts(&request);
    let (instruction, transcript) = user.split_once('\n').unwrap();
    let instruction = instruction.to_lowercase();
    assert!(instruction.contains("paragraph"));
    assert!(instruction.contains("greeting"));
    assert!(instruction.contains("sign-off") || instruction.contains("signoff"));
    assert!(instruction.contains("bullet"));
    assert!(
        instruction.contains("only")
            || instruction.contains("if ")
            || instruction.contains("when ")
    );
    assert_eq!(transcript, request.text);
}

struct LocalModel {
    url: String,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl LocalModel {
    fn returning(candidate: &str) -> Self {
        let candidate = candidate.to_owned();
        Self::responding_to(move |_| candidate.clone())
    }

    fn responding_to(response: impl Fn(&str) -> String + Send + Sync + 'static) -> Self {
        use axum::{routing::post, Json, Router};
        use serde_json::{json, Value};
        let response = std::sync::Arc::new(response);
        let (ready, address) = std::sync::mpsc::channel();
        let (shutdown, stopped) = tokio::sync::oneshot::channel();
        let worker = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                let app = Router::new()
                    .route("/apply-template", post(|| async { Json(json!({"prompt": "rendered prompt"})) }))
                    .route("/tokenize", post(|| async { Json(json!({"tokens": vec![1; 128]})) }))
                    .route("/v1/chat/completions", post(move |Json(request): Json<Value>| {
                        let response = response.clone();
                        async move {
                            let user = request["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap();
                            let (_, transcript) = user.split_once('\n').unwrap();
                            let candidate = response(transcript);
                            Json(json!({"choices": [{"message": {"content": candidate}, "finish_reason": "stop"}]}))
                        }
                    }));
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                ready.send(format!("http://{}", listener.local_addr().unwrap())).unwrap();
                axum::serve(listener, app).with_graceful_shutdown(async { let _ = stopped.await; }).await.unwrap();
            });
        });
        Self {
            url: address.recv_timeout(Duration::from_secs(5)).unwrap(),
            shutdown: Some(shutdown),
            worker: Some(worker),
        }
    }

    fn client(&self) -> FlowClient {
        FlowClient::new_url(self.url.clone(), Duration::from_secs(3)).with_context_size(4096)
    }
}

impl Drop for LocalModel {
    fn drop(&mut self) {
        let _ = self.shutdown.take().unwrap().send(());
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn accepted_email_and_review_structure_reach_the_final_pipeline_output() {
    for (mode, original, polished) in [
        (
            "email",
            "Hi Sarah, the report are ready. Thanks, Papan.",
            "Hi Sarah,\n\nThe report is ready.\n\nThanks,\nPapan.",
        ),
        (
            "notes",
            "The service was friendly. The food was cold, and I would not return.",
            "The service was friendly.\n\nThe food was cold, and I would not return.",
        ),
    ] {
        let model = LocalModel::responding_to(move |part| {
            if mode == "email" {
                part.replace("The report are ready.", "The report is ready.")
            } else {
                polished.to_owned()
            }
        });
        let settings = AppSettings {
            cleanup_level: "high".into(),
            processing_mode: "flow".into(),
            dictation_mode: mode.into(),
            auto_style_from_app: false,
            style: "faithful".into(),
            custom_replacements: vec![],
            dictionary_terms: vec![],
            ..AppSettings::default()
        };
        let outcome = postprocess_transcript(original, &settings, "notepad", &model.client());
        if mode == "email" {
            assert_eq!(
                outcome.smart,
                "Hi Sarah,\n\nThe report are ready.\n\nThanks,\nPapan."
            );
        }
        assert_eq!(outcome.final_text, polished);
        assert!(outcome.rewriter_used, "{:?}", outcome.rewriter_error);
        assert!(outcome.rewriter_error.is_none());
    }
}

#[test]
fn builtin_email_mode_formats_a_short_greeting_body_and_signoff_in_the_pipeline() {
    let original = "Hi Noura, the invoice are attached. Thanks, Maya.";
    let polished = "Hi Noura,\n\nThe invoice is attached.\n\nThanks,\nMaya.";
    let model = LocalModel::responding_to(|part| {
        part.replace("The invoice are attached.", "The invoice is attached.")
    });
    let defaults = AppSettings::default();
    let mode = defaults
        .modes
        .iter()
        .find(|mode| mode.id == "email")
        .expect("builtin Email mode");
    let settings = defaults.settings_for_mode("notepad", mode);
    let outcome = postprocess_transcript(original, &settings, "notepad", &model.client());

    assert_eq!(
        outcome.smart,
        "Hi Noura,\n\nThe invoice are attached.\n\nThanks,\nMaya."
    );
    assert_eq!(outcome.final_text, polished);
    assert!(outcome.rewriter_used, "{:?}", outcome.rewriter_error);
    assert!(outcome.rewriter_error.is_none());

    let fallback = postprocess_transcript(
        "Hi Noura, the invoice is attached. Thanks, Maya.",
        &settings,
        "notepad",
        &FlowClient::new_missing(),
    );
    assert_eq!(fallback.smart, polished);
    assert_eq!(fallback.final_text, polished);
    assert!(!fallback.rewriter_used);
}
