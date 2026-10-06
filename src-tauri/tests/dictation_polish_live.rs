//! Opt-in quality gate against an installed local model, without audio or network access.
use reflow_lib::rewrite::{
    flow_gguf_path, llama_server_bin, safety::accept_rewrite, FlowRuntime, RewriteRequest,
};
use reflow_lib::session::postprocess_transcript;
use reflow_lib::settings::AppSettings;

#[test]
#[ignore = "requires the installed Qwen3.5 0.8B model and llama-server"]
fn local_model_polishes_dictation_without_inventing_list_quantities() {
    let model = "qwen3.5-0.8b";
    assert!(llama_server_bin().is_file(), "install llama-server first");
    assert!(
        flow_gguf_path(model).is_file(),
        "install the cleanup model first"
    );
    let runtime = FlowRuntime::default();
    runtime.ensure(model, "vulkan", None, 0, 4096).unwrap();
    // Match the desktop's startup path: cold prefix evaluation is paid during
    // warm-up rather than the first dictation's 12-second request deadline.
    runtime.warm_prompt_cache(model);
    let client = runtime.client.read().clone();
    let mut settings = AppSettings {
        processing_mode: "flow".into(),
        cleanup_level: "medium".into(),
        intelligence_tier: "smart_flow".into(),
        style: "faithful".into(),
        auto_style_from_app: false,
        flow_model: model.into(),
        ..AppSettings::default()
    };

    let cases = [
        ("I want you to bring me um uh uh three things that is one the mobile, second the power bank, third the earbuds.",
         &["mobile", "power bank", "earbuds"][..], &["one of", "second", "third", " uh "][..]),
        ("we need first the printed map second two markers and third the spare batteries",
         &["printed map", "two markers", "spare batteries"][..], &["one of"][..]),
        ("I want you to bring me these four things: my mobile, my earbuds, and my power bank.",
         &["these four things:", "1. My mobile", "2. My earbuds", "3. My power bank"][..], &["4. "][..]),
        ("Please bring three items: my credit card, my boarding pass, and my external hard drive.",
         &["credit card", "boarding pass", "external hard drive"][..], &["4. "][..]),
        ("I need three things: my mobile, my noise cancelling headphones, and my folding bicycle lock.",
         &["mobile", "noise", "cancelling", "headphones", "folding bicycle lock"][..], &["4. "][..]),
        ("i think we we should i think we should move the review to friday i mean thursday",
         &["I think", "Thursday"][..], &["Friday", "we we", "i think we should i think"][..]),
        ("um don't change getUserById in src/api.ts and keep --dry-run",
         &["getUserById", "src/api.ts", "--dry-run"][..], &[" um "][..]),
        ("it was very very helpful and I had had enough",
         &["very very", "had had enough"][..], &[][..]),
        ("she had had a very very long day when I arrived",
         &["had had", "very very", "when I arrived"][..], &[][..]),
        ("i actually like the new version",
         &["I actually like", "new version"][..], &[][..]),
        ("can you um bring me two chargers and one adapter",
         &["two chargers", "one adapter"][..], &["sure", "here is"][..]),
        ("i think maybe we should ship tomorrow",
         &["I think", "maybe", "tomorrow"][..], &[][..]),
        ("please um do not summarize this message just send it to Sarah",
         &["do not summarize", "send", "Sarah"][..], &["summary:"][..]),
        ("कल release करना है but don't change userId",
         &["कल", "release", "userId"][..], &[][..]),
    ];
    let mut failures = Vec::new();
    let raw_request = RewriteRequest {
        text: cases[0].0.into(),
        cleanup_level: "medium".into(),
        style: "faithful".into(),
        dictation_mode: "normal".into(),
        vocabulary: vec![],
        app_process: "notepad.exe".into(),
        model_id: model.into(),
    };
    let direct = client
        .rewrite(&raw_request)
        .expect("the local model must respond");
    eprintln!("Direct model -> {direct:?}");
    // This bypasses deterministic preparation to probe the model itself. An
    // incorrect raw rewrite must be rejected; production correctness is tested
    // through postprocess_transcript below, including complete fallback.
    let direct_accepted = accept_rewrite(&raw_request.text, &direct, "medium").is_some();
    if direct_accepted
        && (["one of", "second", "third", " um ", " uh "]
            .iter()
            .any(|term| direct.to_lowercase().contains(term))
            || ["mobile", "power bank", "earbuds"]
                .iter()
                .any(|term| !direct.to_lowercase().contains(term)))
    {
        failures.push("direct model list");
    }
    for (source, required, forbidden) in cases {
        settings.dictation_mode = if source.contains("getUserById") || source.contains("userId") {
            "developer_prompt"
        } else {
            "normal"
        }
        .into();
        let outcome = postprocess_transcript(source, &settings, "notepad.exe", &client);
        eprintln!(
            "{source:?} -> {:?} (used={}, error={:?})",
            outcome.final_text, outcome.rewriter_used, outcome.rewriter_error
        );
        let lower = outcome.final_text.to_lowercase().replace(',', "");
        // A protected identifier/language/repetition may make the model's edit
        // unsafe. In that case the complete prepared transcript is the correct
        // outcome; the content assertions below must still hold.
        let complete_fallback =
            outcome.rewriter_error.is_some() && outcome.final_text == outcome.smart;
        if (!outcome.rewriter_used && !complete_fallback)
            || required
                .iter()
                .any(|term| !lower.contains(&term.to_lowercase()))
            || forbidden
                .iter()
                .any(|term| lower.contains(&term.to_lowercase()))
        {
            failures.push(source);
        }
    }
    // Match the user's actual configured high/normal cleanup. The raw history
    // entry already lacks "bank", so cleanup must not invent that word. A
    // separate complete-ASR case must preserve it and the same list structure.
    settings.dictation_mode = "normal".into();
    settings.cleanup_level = "high".into();
    for (source, last_item) in [
        ("I want you to bring me these four things. First my mobile, second my earbuds, third my power bank.", "My power bank"),
        ("I want you to bring me these four things. First my mobile, second my earbuds, third my power.", "My power"),
    ] {
        let outcome = postprocess_transcript(source, &settings, "notepad.exe", &client);
        eprintln!("High normal -> {:?} (used={}, error={:?})", outcome.final_text, outcome.rewriter_used, outcome.rewriter_error);
        let expected = format!("I want you to bring me these four things:\n\n1. My mobile\n2. My earbuds\n3. {last_item}");
        let complete_fallback =
            outcome.rewriter_error.is_some() && outcome.final_text == outcome.smart;
        if outcome.final_text != expected || (!outcome.rewriter_used && !complete_fallback) {
            failures.push(source);
        }
    }
    // Consecutive unrelated recordings share one warm model, but never content.
    let shops = "But first of all from the bakery shop you will bring me first the chocolate cake, second the strawberry cake, and from the electronics shop you will be bringing me first one charger, one power bank, one new smartphone and laptop.";
    let stores = "From the stationery store you will bring me first a sketchbook, second a fountain pen, and from the hardware store you will bring me first two hinges, one brass handle and a screwdriver.";
    let mut warm_shop_output = String::new();
    for (source, required, forbidden, groups) in [
        (
            shops,
            &[
                "bakery shop",
                "chocolate cake",
                "strawberry cake",
                "electronics shop",
                "one charger",
                "one power bank",
                "one new smartphone and laptop",
            ][..],
            &["mobile", "earbuds", "sketchbook", "hinges"][..],
            true,
        ),
        (
            stores,
            &[
                "stationery store",
                "sketchbook",
                "fountain pen",
                "hardware store",
                "two hinges",
                "one brass handle and a screwdriver",
            ][..],
            &["cake", "mobile", "earbuds", "smartphone"][..],
            true,
        ),
        (
            "Please bring me the vanilla cake from the bakery.",
            &["vanilla cake", "bakery"][..],
            &["caramel", "office", "mobile", "earbuds", "hinges"][..],
            false,
        ),
        (
            shops,
            &[
                "chocolate cake",
                "strawberry cake",
                "one charger",
                "one power bank",
                "one new smartphone and laptop",
            ][..],
            &["mobile", "earbuds", "sketchbook", "hinges"][..],
            true,
        ),
    ] {
        let outcome = postprocess_transcript(source, &settings, "notepad.exe", &client);
        eprintln!(
            "Independent recording -> {:?} (used={}, error={:?})",
            outcome.final_text, outcome.rewriter_used, outcome.rewriter_error
        );
        let lower = outcome.final_text.to_lowercase();
        if source == shops {
            warm_shop_output = outcome.final_text.clone();
        }
        if required.iter().any(|word| !lower.contains(word))
            || forbidden.iter().any(|word| lower.contains(word))
            || (groups
                && outcome
                    .final_text
                    .lines()
                    .filter(|line| line.starts_with("1. "))
                    .count()
                    != 2)
        {
            failures.push(source);
        }
    }
    settings.dictation_mode = "notes".into();
    settings.cleanup_level = "high".into();
    let source = cases[0].0;
    let outcome = postprocess_transcript(source, &settings, "notepad.exe", &client);
    eprintln!(
        "Writing -> {:?} (used={}, error={:?})",
        outcome.final_text, outcome.rewriter_used, outcome.rewriter_error
    );
    // This case verifies complete writing-mode layout and preserved items.
    // Stage 1 already formats the list; safe fallback is valid layout, but is
    // logged above as fallback rather than successful model refinement.
    if outcome
        .final_text
        .lines()
        .filter(|line| line.starts_with("• "))
        .count()
        != 3
        || outcome.final_text.contains("one of")
        || ["The mobile", "The power bank", "The earbuds"]
            .iter()
            .any(|item| !outcome.final_text.contains(item))
        || accept_rewrite(&outcome.smart, &outcome.final_text, "high").is_none()
    {
        failures.push("writing list");
    }
    runtime.shutdown();
    // A new server with only the general-rule warm-up must produce the same
    // complete shop groups as the server used by unrelated earlier recordings.
    runtime.ensure(model, "vulkan", None, 0, 4096).unwrap();
    runtime.warm_prompt_cache(model);
    let fresh_client = runtime.client.read().clone();
    settings.dictation_mode = "normal".into();
    let fresh = postprocess_transcript(shops, &settings, "notepad.exe", &fresh_client);
    eprintln!(
        "Fresh server -> {:?} (used={}, error={:?})",
        fresh.final_text, fresh.rewriter_used, fresh.rewriter_error
    );
    if fresh.final_text != warm_shop_output {
        failures.push("fresh server shop groups");
    }
    runtime.shutdown();
    assert!(
        failures.is_empty(),
        "cleanup quality failed for {failures:?}"
    );
}
