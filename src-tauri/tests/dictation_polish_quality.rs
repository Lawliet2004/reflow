//! Meaning-preserving cleanup contracts for ordinary dictation and spoken lists.

use reflow_lib::formatting::{format_transcript, CustomReplacements};
use reflow_lib::rewrite::safety::accept_rewrite;
use reflow_lib::rewrite::{polish_or_fallback, FlowClient, RewriteRequest};
use std::time::Duration;

const SPOKEN_LIST: &str = "I want you to bring me um uh uh three things that is one the mobile, second the power bank, third the earbuds.";
const BAD_LIST_REWRITE: &str = "I want you to bring me three things: one of the mobile, one of the power bank, and one of the earbuds.";
const NATURAL_LIST_REWRITE: &str =
    "I want you to bring me three things: the mobile, the power bank, and the earbuds.";
const NUMBERED_INVENTORY: &str =
    "I want you to bring me these four things:\n\n1. My mobile\n2. My earbuds\n3. My power bank";
const SHOP_DICTATION: &str = "But first of all from the bakery shop you will bring me first the chocolate cake, second the strawberry cake, and from the electronics shop you will be bringing me first one charger, one power bank, one new smartphone and laptop.";
const GROUPED_SHOP_DICTATION: &str = "But first of all, from the bakery shop, you will bring me:\n\n• The chocolate cake\n• The strawberry cake\n\nFrom the electronics shop, you will be bringing me:\n\n• One charger\n• One power bank\n• One new smartphone and laptop";
const UNRELATED_SHOP_REWRITE: &str =
    "Once you need three things:\n\n• The mobile\n• The power bank\n• The earbuds";

#[test]
fn rejects_user_reported_quantity_hallucination() {
    assert!(
        accept_rewrite(SPOKEN_LIST, BAD_LIST_REWRITE, "medium").is_none(),
        "enumeration labels must not become invented quantities"
    );
}

#[test]
fn accepts_the_same_spoken_list_as_natural_prose() {
    assert_eq!(
        accept_rewrite(SPOKEN_LIST, NATURAL_LIST_REWRITE, "medium").as_deref(),
        Some(NATURAL_LIST_REWRITE)
    );
}

#[test]
fn accepts_spoken_enumeration_as_bullets_without_inventing_counts() {
    let original =
        "Please bring three things: first the mobile, second the power bank, third the earbuds.";
    let polished = "Please bring three things:\n\n• The mobile\n• The power bank\n• The earbuds";
    assert_eq!(
        accept_rewrite(original, polished, "medium").as_deref(),
        Some(polished)
    );
}

#[test]
fn explicit_item_quantities_survive_list_formatting() {
    let original = "Please bring these things: first two phones, second three power banks, third four pairs of earbuds.";
    let polished =
        "Please bring these things:\n\n• Two phones\n• Three power banks\n• Four pairs of earbuds";
    assert_eq!(
        accept_rewrite(original, polished, "medium").as_deref(),
        Some(polished),
        "removing ordinal labels must preserve quantities inside each item"
    );
    assert!(
        accept_rewrite(
            original,
            "Please bring these things:\n\n• Phones\n• Power banks\n• Pairs of earbuds",
            "medium"
        )
        .is_none(),
        "quantities inside list items are meaningful and must not be removed"
    );
}

#[test]
fn rejects_added_or_changed_quantities_in_enumerated_items() {
    let original =
        "Please bring three things: first the mobile, second the power bank, third the earbuds.";
    for candidate in [
        "Please bring three things: one mobile, one power bank, and one pair of earbuds.",
        "Please bring three things: two mobiles, one power bank, and one pair of earbuds.",
    ] {
        assert!(
            accept_rewrite(original, candidate, "medium").is_none(),
            "invented item quantities must be rejected: {candidate}"
        );
    }
}

#[test]
fn ordinary_quantity_prose_is_not_interpreted_as_an_enumeration() {
    let original = "um I have one mobile and two power banks";
    let formatted = format_transcript(
        original,
        "flow",
        "notes",
        true,
        true,
        &CustomReplacements::new(vec![]),
    );
    assert_eq!(formatted, "I have one mobile and two power banks.");
    assert_eq!(
        accept_rewrite(original, &formatted, "medium").as_deref(),
        Some(formatted.as_str())
    );
}

#[test]
fn cleanup_preserves_negation_numbers_and_identifiers() {
    let original = "um I don't want you to change 42 to 43 in src/main.rs or rename userId";
    let polished = "I don't want you to change 42 to 43 in src/main.rs or rename userId.";
    assert_eq!(
        accept_rewrite(original, polished, "medium").as_deref(),
        Some(polished)
    );
    for unsafe_rewrite in [
        "I want you to change 42 to 43 in src/main.rs or rename userId.",
        "I don't want you to change 42 to 44 in src/main.rs or rename userId.",
        "I don't want you to change 42 to 43 in src/index.rs or rename userId.",
        "I don't want you to change 42 to 43 in src/main.rs or rename user_id.",
    ] {
        assert!(
            accept_rewrite(original, unsafe_rewrite, "medium").is_none(),
            "cleanup changed protected content: {unsafe_rewrite}"
        );
    }
}

#[test]
fn an_explicit_line_break_survives_polishing() {
    let original = "I need the mobile.\nPlease bring the earbuds too.";
    let candidate = "I need the mobile. Please bring the earbuds too.";
    assert!(accept_rewrite(original, candidate, "medium").is_none());
    assert_eq!(
        accept_rewrite(original, original, "medium").as_deref(),
        Some(original)
    );
}

#[test]
fn an_explicit_prose_inventory_preserves_the_full_item_names() {
    let original =
        "I want you to bring me these four things: my mobile, my earbuds, and my power bank.";
    let truncated =
        "I want you to bring me these four things: my mobile, my earbuds, and my power.";
    assert!(
        accept_rewrite(original, truncated, "medium").is_none(),
        "the final word of an item is meaningful even when most transcript words match"
    );
    assert_eq!(
        accept_rewrite(original, original, "medium").as_deref(),
        Some(original)
    );
}

#[test]
fn a_spoken_ordinal_inventory_preserves_the_full_item_names() {
    let original = "I want you to bring me these four things: first my mobile, second my earbuds, third my power bank.";
    let truncated =
        "I want you to bring me these four things: my mobile, my earbuds, and my power.";
    assert!(
        accept_rewrite(original, truncated, "medium").is_none(),
        "removing positional labels must not remove any part of the listed objects"
    );
    assert_eq!(
        accept_rewrite(original, NUMBERED_INVENTORY, "high").as_deref(),
        Some(NUMBERED_INVENTORY)
    );
}

#[test]
fn a_single_list_item_preserves_its_full_name() {
    assert!(accept_rewrite("My power bank", "My power", "high").is_none());
    assert_eq!(
        accept_rewrite("um my uh power bank", "My power bank.", "high").as_deref(),
        Some("My power bank."),
        "filler removal and punctuation are still valid cleanup"
    );
}

#[test]
fn compound_item_names_and_modifiers_are_preserved_generally() {
    for (complete_name, truncated_name) in [
        ("credit card", "credit"),
        ("boarding pass", "pass"),
        ("external hard drive", "hard drive"),
        ("noise cancelling headphones", "headphones"),
        ("folding bicycle lock", "bicycle lock"),
    ] {
        let original =
            format!("Please bring three things: my mobile, my earbuds, and my {complete_name}.");
        let truncated =
            format!("Please bring three things: my mobile, my earbuds, and my {truncated_name}.");
        assert!(
            accept_rewrite(&original, &truncated, "medium").is_none(),
            "cleanup lost part of {complete_name:?}: {truncated}"
        );
        assert!(
            accept_rewrite(
                &format!("um my uh {complete_name}"),
                &format!("My {complete_name}."),
                "high"
            )
            .is_some(),
            "cleaning fillers must remain valid for {complete_name:?}"
        );
    }
}

#[test]
fn a_stated_inventory_count_does_not_justify_fabricating_a_missing_item() {
    let original = "I want you to bring me these four things: first my mobile, second my earbuds, third my power bank.";
    assert_eq!(
        accept_rewrite(original, NUMBERED_INVENTORY, "high").as_deref(),
        Some(NUMBERED_INVENTORY),
        "a stated count can disagree with the number of actually dictated items"
    );
    assert!(
        accept_rewrite(
            original,
            &format!("{NUMBERED_INVENTORY}\n4. My charger"),
            "high"
        )
        .is_none(),
        "only the three named items may be emitted; the model must not invent a fourth"
    );
}

#[test]
fn inventory_grammar_and_fillers_can_be_cleaned_without_changing_objects() {
    let original = "I want you to bring me um these three things that is my credit card, uh my boarding pass, and my external hard drive.";
    let polished = "I want you to bring me these three things: my credit card, my boarding pass, and my external hard drive.";
    assert_eq!(
        accept_rewrite(original, polished, "medium").as_deref(),
        Some(polished)
    );
}

#[test]
fn an_unparsed_inventory_cannot_gain_an_extra_item() {
    let original =
        "Please bring these three things: first my mobile, second my earbuds, third my power bank.";
    let invented =
        "Please bring these three things: my mobile; my earbuds; my power bank; my charger.";
    assert!(
        accept_rewrite(original, invented, "high").is_none(),
        "retaining the original item names does not permit appending another object"
    );
}

#[test]
fn an_inventory_cannot_become_a_substitution_between_its_items() {
    let original = "I need these two things: first my credit card, second my boarding pass.";
    let substitution = "I need these two things: my credit card instead of my boarding pass.";
    assert!(
        accept_rewrite(original, substitution, "medium").is_none(),
        "both objects remain named, but the candidate no longer requests both"
    );
}

#[test]
fn conjunctions_inside_an_item_are_preserved_when_removing_list_labels() {
    let original = "first rock and roll records second power bank";
    let changed_item = "Rock roll records and power bank.";
    let complete_items = "Rock and roll records and power bank.";
    assert!(
        accept_rewrite(original, changed_item, "medium").is_none(),
        "a conjunction within an object name is content, not a list separator"
    );
    assert_eq!(
        accept_rewrite(original, complete_items, "medium").as_deref(),
        Some(complete_items)
    );
}

#[test]
fn formatting_an_inventory_preserves_the_request_after_its_last_item() {
    let original = "I need two items: first my phone, second my charger. Please call Sarah.";
    let missing_tail = "I need two items:\n\n1. My phone\n2. My charger";
    assert!(
        accept_rewrite(original, missing_tail, "high").is_none(),
        "preserving every item does not permit dropping the following request"
    );
}

#[test]
fn formatting_an_inventory_preserves_names_in_its_following_request() {
    let original = "I need two items: first my phone, second my charger. Please call Sarah.";
    let changed_tail = "I need two items:\n\n1. My phone\n2. My charger\n\nPlease call John.";
    assert!(
        accept_rewrite(original, changed_tail, "high").is_none(),
        "the request following an inventory must not change its recipient"
    );
}

#[test]
fn a_complete_inventory_and_following_request_can_be_formatted() {
    let original = "I need two items: first my phone, second my charger. Please call Sarah.";
    let polished = "I need two items:\n\n1. My phone\n2. My charger\n\nPlease call Sarah.";
    assert_eq!(
        accept_rewrite(original, polished, "high").as_deref(),
        Some(polished)
    );
}

#[test]
fn compound_hyphenation_is_valid_inventory_cleanup() {
    let original =
        "Please bring three things: my mobile, my earbuds, and my noise cancelling headphones.";
    let polished =
        "Please bring three things: my mobile, my earbuds, and my noise-cancelling headphones.";
    assert_eq!(
        accept_rewrite(original, polished, "medium").as_deref(),
        Some(polished)
    );
    assert_eq!(
        accept_rewrite(
            "My noise cancelling headphones",
            "My noise-cancelling headphones.",
            "high"
        )
        .as_deref(),
        Some("My noise-cancelling headphones.")
    );
}

#[test]
fn article_like_words_inside_names_are_preserved() {
    for (original_item, truncated_item) in [
        ("my A grade report", "my grade report"),
        ("\"The Who\" tickets", "\"Who\" tickets"),
    ] {
        let original = format!("Please bring these two things: my notebook, and {original_item}.");
        let truncated =
            format!("Please bring these two things: my notebook, and {truncated_item}.");
        assert!(
            accept_rewrite(&original, &truncated, "medium").is_none(),
            "a letter or part of a quoted name is not a removable article: {original_item}"
        );
        assert!(accept_rewrite(&original, &original, "medium").is_some());
    }
}

#[test]
fn possessive_sentence_grammar_and_contractions_remain_editable() {
    for (original, polished) in [
        ("My phone are broken", "My phone is broken."),
        ("My meeting has been cancelled", "My meeting was cancelled."),
        ("My code will not run", "My code won't run."),
    ] {
        assert_eq!(
            accept_rewrite(original, polished, "medium").as_deref(),
            Some(polished),
            "a possessive opener does not make a full sentence an immutable object name"
        );
    }
}

#[test]
fn a_four_things_dictation_with_three_items_preserves_the_ambiguous_last_word() {
    let original = "I want you to bring me these four things. First my mobile, second my earbuds, third my power.";
    let formatted = format_transcript(
        original,
        "flow",
        "normal",
        true,
        true,
        &CustomReplacements::new(vec![]),
    );
    let expected =
        "I want you to bring me these four things:\n\n1. My mobile\n2. My earbuds\n3. My power";
    assert_eq!(formatted, expected);
    assert_eq!(
        accept_rewrite(original, expected, "medium").as_deref(),
        Some(expected)
    );
    for invented in [
        expected.replace("My power", "My power bank"),
        format!("{expected}\n4. My charger"),
    ] {
        assert!(
            accept_rewrite(&formatted, &invented, "medium").is_none(),
            "an ambiguous word or inconsistent stated count must not be completed by guessing"
        );
    }
}

#[test]
fn a_shop_dictation_rejects_an_unrelated_inventory() {
    assert!(
        accept_rewrite(SHOP_DICTATION, UNRELATED_SHOP_REWRITE, "high").is_none(),
        "a familiar output shape cannot replace the actual dictated requests"
    );
}

#[test]
fn shop_groups_can_be_formatted_with_all_objects_and_explicit_quantities() {
    assert_eq!(
        accept_rewrite(SHOP_DICTATION, GROUPED_SHOP_DICTATION, "high").as_deref(),
        Some(GROUPED_SHOP_DICTATION)
    );
    assert!(!GROUPED_SHOP_DICTATION.contains("One laptop"));
    assert!(GROUPED_SHOP_DICTATION.contains("One new smartphone and laptop"));
}

#[test]
fn grouping_applies_to_other_places_objects_and_quantities() {
    let original = "From the stationery store you will bring me first a sketchbook, second a fountain pen, and from the hardware store you will bring me first two hinges, one brass handle and a screwdriver.";
    let polished = "From the stationery store you will bring me:\n\n• A sketchbook\n• A fountain pen\n\nFrom the hardware store you will bring me:\n\n• Two hinges\n• One brass handle and a screwdriver";
    assert_eq!(
        accept_rewrite(original, polished, "high").as_deref(),
        Some(polished),
        "group formatting must follow this input's places and objects"
    );
}

#[test]
fn non_inventory_prose_rejects_unrelated_nouns_even_at_the_same_length() {
    for (original, unrelated) in [
        (
            "Please bring me the vanilla cake from the bakery.",
            "Please bring me the caramel cake from the office.",
        ),
        (
            "I need the grey charger before the meeting.",
            "I need the blue battery before the meeting.",
        ),
        (
            "We should send the invoice to Sarah this evening.",
            "We should send the picture to Priya this evening.",
        ),
    ] {
        assert_eq!(original.chars().count(), unrelated.chars().count());
        assert_eq!(
            original.split_whitespace().count(),
            unrelated.split_whitespace().count()
        );
        assert!(
            accept_rewrite(original, unrelated, "high").is_none(),
            "a small edit distance does not make changed content faithful: {unrelated}"
        );
    }
}

#[test]
fn non_inventory_grammar_can_improve_without_changing_its_content() {
    for (original, polished) in [
        (
            "I want you bring me chocolate cake from bakery",
            "I want you to bring me the chocolate cake from the bakery.",
        ),
        (
            "I want you to brings me the chocolate cake from the bakery",
            "I want you to bring me the chocolate cake from the bakery.",
        ),
        ("She carry it tomorrow", "She carries it tomorrow."),
    ] {
        assert_eq!(
            accept_rewrite(original, polished, "high").as_deref(),
            Some(polished)
        );
    }
}

#[test]
fn bakery_prose_cannot_be_replaced_with_electronics_objects() {
    let original = "From the bakery, bring me bread, croissants, and muffins.";
    let unrelated = "From the bakery, bring me mobile, power bank, and earbuds.";
    assert!(
        accept_rewrite(original, unrelated, "high").is_none(),
        "ordinary prose must retain its objects even without explicit enumeration labels"
    );
    assert_eq!(
        accept_rewrite(original, original, "high").as_deref(),
        Some(original)
    );
}

#[test]
fn a_single_quantity_with_an_ambiguous_conjunction_is_preserved() {
    let original = "um one new smartphone and laptop";
    let polished = "One new smartphone and laptop.";
    assert_eq!(
        accept_rewrite(original, polished, "high").as_deref(),
        Some(polished),
        "cleanup must not guess a new boundary or another quantity inside the phrase"
    );
}

#[test]
fn noun_suffixes_are_not_grammatical_variants_of_distinct_objects_or_names() {
    for (original, changed) in [
        (
            "Please bring the painting from the gallery.",
            "Please bring the paint from the gallery.",
        ),
        (
            "Please bring the dressing from the kitchen.",
            "Please bring the dress from the kitchen.",
        ),
        (
            "Please send the receipt to Miles this afternoon.",
            "Please send the receipt to Mile this afternoon.",
        ),
    ] {
        assert!(
            accept_rewrite(original, changed, "high").is_none(),
            "a suffix can distinguish the intended object or person: {original} -> {changed}"
        );
    }
}

#[test]
fn recipient_names_or_ambiguous_nouns_are_not_verb_agreement_candidates() {
    for (original, renamed) in [
        (
            "Give it to James the manager.",
            "Give it to Jame the manager.",
        ),
        (
            "Give it to Chris the manager.",
            "Give it to Chri the manager.",
        ),
        (
            "Give it to canvas the design.",
            "Give it to canva the design.",
        ),
    ] {
        assert!(
            accept_rewrite(original, renamed, "high").is_none(),
            "an ambiguous recipient or object must not be changed by assuming a verb frame"
        );
    }
}

#[test]
fn changing_name_casing_does_not_rename_the_recipient() {
    let original = "Please send the receipt to Miles this afternoon.";
    let candidate = "Please send the receipt to miles this afternoon.";
    assert_eq!(
        accept_rewrite(original, candidate, "medium").as_deref(),
        Some(candidate)
    );
}

#[test]
fn common_contractions_preserve_the_speaker_and_sentence_meaning() {
    for (original, polished) in [
        (
            "We will bring the cake tomorrow.",
            "We'll bring the cake tomorrow.",
        ),
        (
            "We would bring the cake tomorrow.",
            "We'd bring the cake tomorrow.",
        ),
        (
            "You will bring the cakes tomorrow.",
            "You'll bring the cakes tomorrow.",
        ),
        (
            "You have brought the cakes already.",
            "You've brought the cakes already.",
        ),
        (
            "They will bring the cakes tomorrow.",
            "They'll bring the cakes tomorrow.",
        ),
        (
            "They have brought the cakes already.",
            "They've brought the cakes already.",
        ),
        (
            "They would bring the cakes tomorrow.",
            "They'd bring the cakes tomorrow.",
        ),
        (
            "Let us bring the cakes tomorrow.",
            "Let's bring the cakes tomorrow.",
        ),
    ] {
        assert_eq!(
            accept_rewrite(original, polished, "medium").as_deref(),
            Some(polished),
            "an unambiguous contraction in this sentence is valid cleanup"
        );
    }
}

#[test]
fn a_headerless_list_cannot_gain_an_invented_recipient() {
    let original = "• Bread\n• Muffins";
    let invented = "For Bob:\n\n• Bread\n• Muffins";
    assert!(
        accept_rewrite(original, invented, "high").is_none(),
        "preserving the items does not permit adding a recipient or header"
    );
}

#[test]
fn group_headers_preserve_source_and_destination_roles() {
    for (original, reversed) in [
        (
            "Send from Sarah to Pat:\n\n• Bread\n• Muffins",
            "Send from Pat to Sarah:\n\n• Bread\n• Muffins",
        ),
        ("Send from Sarah to Pat:", "Send from Pat to Sarah:"),
    ] {
        assert!(
            accept_rewrite(original, reversed, "high").is_none(),
            "the same names in a different order change the direction of the request"
        );
        assert_eq!(
            accept_rewrite(original, original, "high").as_deref(),
            Some(original)
        );
    }
}

#[test]
fn prose_preserves_source_and_destination_roles() {
    let original = "Send from Sarah to Pat.";
    for reversed in ["Send from Pat to Sarah.", "Send to Sarah from Pat."] {
        assert!(
            accept_rewrite(original, reversed, "high").is_none(),
            "changing either name order or the directional prepositions reverses the request"
        );
    }
}

#[test]
fn only_an_initial_and_may_be_removed_from_a_group_header() {
    for (original, polished) in [
        (
            "And from the bakery shop you will bring me:",
            "From the bakery shop you will bring me:",
        ),
        (
            "And send the bread and muffins from Sarah to Pat:",
            "Send the bread and muffins from Sarah to Pat:",
        ),
    ] {
        assert_eq!(
            accept_rewrite(original, polished, "high").as_deref(),
            Some(polished)
        );
    }
    assert!(
        accept_rewrite(
            "And send the bread and muffins from Sarah to Pat:",
            "Send the bread muffins from Sarah to Pat:",
            "high"
        )
        .is_none(),
        "an internal conjunction is not the introductory connector"
    );
}

#[test]
fn dot_command_arguments_remain_content_when_rewriting_list_items() {
    for (original, missing_argument) in [("• git add .", "• git add"), ("• cd ..", "• cd")]
    {
        assert!(
            accept_rewrite(original, missing_argument, "high").is_none(),
            "a standalone dot path is a command argument: {original}"
        );
        assert_eq!(
            accept_rewrite(original, original, "high").as_deref(),
            Some(original)
        );
    }
}

#[test]
fn dot_command_arguments_cannot_move_to_a_different_command() {
    for (original, moved_argument) in [
        (
            "Run git add . and git status",
            "Run git add and git status .",
        ),
        (
            "• Run git add . and git status",
            "• Run git add and git status .",
        ),
    ] {
        assert!(
            accept_rewrite(original, moved_argument, "high").is_none(),
            "retaining a dot argument does not permit moving it to the other command"
        );
    }
}

/// Only the model is replaced: formatting, generation transport, validation,
/// and the final fallback all use the same implementation as live dictation.
struct LocalModel {
    url: String,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl LocalModel {
    fn responding_with(candidate: &str) -> Self {
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
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    let app = Router::new()
                        .route(
                            "/apply-template",
                            post(|| async { Json(json!({"prompt": "rendered prompt"})) }),
                        )
                        .route(
                            "/tokenize",
                            post(|| async { Json(json!({"tokens": vec![1; 128]})) }),
                        )
                        .route(
                            "/v1/chat/completions",
                            post(move |Json(request): Json<Value>| {
                                let response = response.clone();
                                async move {
                                    let user =
                                        request["messages"].as_array().unwrap().last().unwrap()
                                            ["content"]
                                            .as_str()
                                            .unwrap();
                                    let (_, transcript) = user.split_once('\n').unwrap();
                                    let candidate = response(transcript);
                                    Json(json!({
                                        "choices": [{
                                            "message": {"content": candidate},
                                            "finish_reason": "stop"
                                        }]
                                    }))
                                }
                            }),
                        );
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                    ready
                        .send(format!("http://{}", listener.local_addr().unwrap()))
                        .unwrap();
                    axum::serve(listener, app)
                        .with_graceful_shutdown(async {
                            let _ = stopped.await;
                        })
                        .await
                        .unwrap();
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

fn polish_request(text: &str) -> RewriteRequest {
    RewriteRequest {
        text: text.into(),
        cleanup_level: "medium".into(),
        style: "faithful".into(),
        dictation_mode: "normal".into(),
        vocabulary: vec![],
        app_process: String::new(),
        model_id: "qwen3.5-2b".into(),
    }
}

#[test]
fn rejected_model_output_falls_back_to_the_complete_formatted_dictation() {
    let model = LocalModel::responding_with(BAD_LIST_REWRITE);
    let formatted = format_transcript(
        SPOKEN_LIST,
        "flow",
        "normal",
        true,
        true,
        &CustomReplacements::new(vec![]),
    );
    let outcome = polish_or_fallback(&model.client(), &formatted, &polish_request(&formatted));

    assert_eq!(outcome.final_text, formatted);
    assert!(
        !outcome.used,
        "an unsafe model answer must not reach injection"
    );
    assert!(
        outcome.error.is_some(),
        "the rejected answer must be reported"
    );
    for item in ["mobile", "power bank", "earbuds"] {
        assert!(outcome.final_text.contains(item));
    }
}

#[test]
fn valid_model_cleanup_reaches_the_final_dictation() {
    let model = LocalModel::responding_to(str::to_owned);
    let formatted = format_transcript(
        SPOKEN_LIST,
        "flow",
        "normal",
        true,
        true,
        &CustomReplacements::new(vec![]),
    );
    let outcome = polish_or_fallback(&model.client(), &formatted, &polish_request(&formatted));

    assert_eq!(outcome.final_text, formatted);
    assert_eq!(
        outcome.final_text,
        "I want you to bring me three things:\n\n1. The mobile\n2. The power bank\n3. The earbuds"
    );
    assert!(outcome.used, "{:?}", outcome.error);
    assert!(outcome.error.is_none());
}

#[test]
fn truncating_a_list_item_preserves_the_complete_structured_inventory() {
    let model = LocalModel::responding_to(|part| {
        if part == "My power bank" {
            "My power".into()
        } else {
            part.to_owned()
        }
    });
    let mut request = polish_request(NUMBERED_INVENTORY);
    request.cleanup_level = "high".into();
    request.dictation_mode = "notes".into();
    let outcome = polish_or_fallback(&model.client(), NUMBERED_INVENTORY, &request);

    assert_eq!(outcome.final_text, NUMBERED_INVENTORY);
    assert!(
        !outcome.used,
        "a partial object name must not reach injection"
    );
    assert!(outcome.error.is_some());
    for item in ["My mobile", "My earbuds", "My power bank"] {
        assert_eq!(outcome.final_text.matches(item).count(), 1);
    }
    assert_eq!(outcome.final_text.lines().count(), 5);
    assert!(!outcome.final_text.contains("4. "));
}

#[test]
fn an_unrelated_model_inventory_preserves_the_complete_shop_dictation() {
    let model = LocalModel::responding_with(UNRELATED_SHOP_REWRITE);
    let formatted = format_transcript(
        SHOP_DICTATION,
        "high",
        "notes",
        true,
        true,
        &CustomReplacements::new(vec![]),
    );
    let mut request = polish_request(&formatted);
    request.cleanup_level = "high".into();
    request.dictation_mode = "notes".into();
    let outcome = polish_or_fallback(&model.client(), &formatted, &request);

    assert_eq!(outcome.final_text, formatted);
    assert!(!outcome.used);
    assert!(outcome.error.is_some());
    let preserved = outcome.final_text.to_lowercase();
    let mut previous = 0;
    for detail in [
        "bakery shop",
        "chocolate cake",
        "strawberry cake",
        "electronics shop",
        "one charger",
        "one power bank",
        "one new smartphone",
        "laptop",
    ] {
        let position = preserved.find(detail).expect("dictated detail preserved");
        assert!(
            position >= previous,
            "a request moved to the wrong group: {detail}"
        );
        previous = position;
    }
    assert!(!preserved.contains("earbuds"));
    assert!(!preserved.contains("one laptop"));
}

#[test]
fn complex_shop_dictation_prepares_both_groups_before_polishing() {
    let model = LocalModel::responding_to(str::to_owned);
    let formatted = format_transcript(
        SHOP_DICTATION,
        "high",
        "notes",
        true,
        true,
        &CustomReplacements::new(vec![]),
    );
    let mut request = polish_request(&formatted);
    request.cleanup_level = "high".into();
    request.dictation_mode = "notes".into();
    let outcome = polish_or_fallback(&model.client(), &formatted, &request);

    assert_eq!(outcome.final_text, formatted);
    assert!(outcome.used, "{:?}", outcome.error);
    assert!(outcome.error.is_none());
    assert_eq!(
        outcome
            .final_text
            .lines()
            .filter(|line| line.trim_start().starts_with("• "))
            .count(),
        5,
        "the ambiguous smartphone-and-laptop phrase must remain a single complete item"
    );
    let prepared = outcome.final_text.to_lowercase();
    for detail in [
        "bakery shop",
        "chocolate cake",
        "strawberry cake",
        "electronics shop",
        "one charger",
        "one power bank",
        "one new smartphone",
        "laptop",
    ] {
        assert!(
            prepared.contains(detail),
            "missing dictated detail: {detail}"
        );
    }
    assert!(!prepared.contains("one laptop"));
    assert!(prepared.contains("one new smartphone and laptop"));
    assert_eq!(
        prepared
            .split_whitespace()
            .filter(|word| word.trim_matches(|ch: char| !ch.is_alphanumeric()) == "one")
            .count(),
        3,
        "only the three explicitly dictated quantities may be retained"
    );
}
