use reflow_lib::assistant_tools::{parse_tool, AssistantTool};
use reflow_lib::audio::meeting::MeetingMixer;
use reflow_lib::rewrite::tasks::summarize_with;

#[test]
fn assistant_commands_are_entire_allowlisted_proposals() {
    assert_eq!(
        parse_tool("SEARCH: Rust ownership"),
        Some(AssistantTool::Search {
            query: "Rust ownership".into()
        })
    );
    assert_eq!(
        parse_tool("NOTE: Buy tea"),
        Some(AssistantTool::Note {
            text: "Buy tea".into()
        })
    );
    for text in [
        "RUN: del *",
        "Here is SEARCH: something",
        "SEARCH: one\nNOTE: two",
        "SEARCH:",
        "NOTE: ",
    ] {
        assert_eq!(parse_tool(text), None, "{text}");
    }
    assert!(parse_tool(&format!("SEARCH: {}", "a".repeat(501))).is_none());
    let Some(AssistantTool::Search { query }) =
        parse_tool("SEARCH: https://evil.test/?private=1 & words")
    else {
        panic!()
    };
    let url = reflow_lib::assistant_tools::search_url(&query).unwrap();
    assert_eq!(url.host_str(), Some("www.google.com"));
    assert_eq!(url.query_pairs().next().unwrap().1, query);
}

#[test]
fn meeting_mix_aligns_sources_and_leaves_silent_gaps() {
    let mut mixer = MeetingMixer::default();
    mixer.push_at(false, 0, &[0.8, 0.6, 0.4, 0.2]);
    mixer.push_at(true, 2, &[0.2, 0.4]);
    assert_eq!(mixer.render(4), vec![0.4, 0.3, 0.3, 0.3]);
    mixer.push_at(true, 0, &[1.0; 4]); // late callbacks cannot move old audio forward
    assert_eq!(mixer.render(2), vec![0.0; 2]);
    mixer.push_at(false, 6, &[f32::NAN, f32::INFINITY]);
    assert!(mixer.render(2).iter().all(|x| x.is_finite()));
}

#[test]
fn meeting_mix_has_a_fixed_memory_bound() {
    let mut mixer = MeetingMixer::default();
    mixer.push_at(true, 0, &vec![1.0; 160_000]);
    assert!(mixer.buffered_samples() <= 32_000);
}

#[test]
fn summaries_split_at_sentence_boundaries_and_keep_all_sections() {
    let source = "Decision alpha.\nOwner beta.\nDeadline gamma.";
    let mut seen = vec![];
    let result = summarize_with(source, 22, &mut |part| {
        assert!(part.len() <= 22);
        seen.push(part.to_owned());
        Ok(part.to_owned())
    })
    .unwrap();
    assert!(result.contains("Decision alpha."));
    assert!(result.contains("Deadline gamma."));
    assert!(seen.iter().any(|x| x.contains("Owner beta.")));
}

#[test]
fn summaries_reject_unbounded_sentences_and_propagate_model_failures() {
    assert!(summarize_with("a".repeat(100).as_str(), 20, &mut |_| Ok("Summary".into())).is_err());
    assert!(
        summarize_with("One. Two.", 100, &mut |_| Err("runtime missing".into()))
            .unwrap_err()
            .contains("runtime missing")
    );
}

#[tokio::test]
async fn meeting_mode_requires_opt_in_before_opening_devices() {
    let directory = std::env::temp_dir().join(format!("reflow-meeting-{}", uuid::Uuid::new_v4()));
    let ctx = reflow_lib::context::AppContext::bootstrap_test(directory.clone());
    let error = reflow_lib::session::start_meeting(&ctx).await.unwrap_err();
    assert!(error.message.contains("Enable meeting mode"));
    assert_eq!(
        *ctx.capture_kind.read(),
        reflow_lib::dory::CaptureKind::None
    );
    drop(ctx);
    let _ = std::fs::remove_dir_all(directory);
}

#[tokio::test]
async fn meeting_pipeline_saves_provenance_and_never_pastes() {
    use reflow_lib::{dory::CaptureKind, session};
    let directory =
        std::env::temp_dir().join(format!("reflow-meeting-stop-{}", uuid::Uuid::new_v4()));
    let ctx = reflow_lib::context::AppContext::bootstrap_test(directory.clone());
    ctx.settings_store
        .merge_update(
            serde_json::json!({"intelligence_tier":"raw_verbatim", "cleanup_level":"light"}),
        )
        .unwrap();
    // Inject deterministic PCM through the actor, bypassing physical devices.
    session::start_external(&ctx, Some("en".into()))
        .await
        .unwrap();
    session::push_f32(&ctx, &vec![0.1; 6400]).unwrap();
    *ctx.capture_kind.write() = CaptureKind::SystemMix;
    assert!(!session::hotkey_owns_session(&ctx, "dictate"));
    let outcome = session::stop(&ctx, true).await.unwrap();
    assert!(!outcome.injected);
    assert!(!outcome.final_text.is_empty());
    let rows = ctx.history_store.get_entries(10, 0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, "meeting");
    assert_eq!(rows[0].source, "meeting");
    assert_eq!(rows[0].application_name, "Meeting");
    drop(ctx);
    let _ = std::fs::remove_dir_all(directory);
}

#[tokio::test]
async fn earlier_meeting_speech_survives_an_overflowed_silent_tail() {
    use reflow_lib::{dory::CaptureKind, session};
    let directory =
        std::env::temp_dir().join(format!("reflow-meeting-tail-{}", uuid::Uuid::new_v4()));
    let ctx = reflow_lib::context::AppContext::bootstrap_test(directory.clone());
    ctx.settings_store.merge_update(serde_json::json!({"intelligence_tier":"raw_verbatim", "cleanup_level":"light", "audio_retention":"disabled"})).unwrap();
    session::start_external(&ctx, Some("en".into()))
        .await
        .unwrap();
    session::push_f32(&ctx, &vec![0.1; 6400]).unwrap();
    session::push_f32(&ctx, &vec![0.0; 960_001]).unwrap();
    *ctx.capture_kind.write() = CaptureKind::SystemMix;
    let outcome = session::stop(&ctx, false).await.unwrap();
    assert!(
        !outcome.final_text.is_empty(),
        "Earlier speech must not be canceled because the retained tail is quiet"
    );
    assert_eq!(ctx.history_store.get_entries(10, 0).unwrap().len(), 1);
    drop(ctx);
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn summaries_accept_the_default_local_context_window() {
    let client = reflow_lib::rewrite::FlowClient::new_missing().with_context_size(
        reflow_lib::settings::AppSettings::default()
            .refinement
            .context_size,
    );
    let error = reflow_lib::rewrite::tasks::summarize_transcript(&client, "Decision: ship today.")
        .unwrap_err();
    assert!(
        error.contains("not available"),
        "Expected to reach the missing-runtime boundary, got: {error}"
    );
}

#[test]
fn meeting_capture_timestamps_preserve_continuous_audio_despite_callback_jitter() {
    use reflow_lib::audio::meeting::CaptureTimeline;
    let mut timeline = CaptureTimeline::default();
    let mut mixer = MeetingMixer::default();
    // Three 20ms buffers have contiguous capture timestamps but irregular
    // callback arrival times (20ms, 60ms, 61ms).
    for (capture, arrival) in [(0, 320), (320, 960), (640, 976)] {
        let start = timeline.place(Some(capture), arrival, 320);
        mixer.push_at(false, start, &[1.0; 320]);
    }
    assert_eq!(mixer.render(960), vec![0.5; 960]);
    // A genuine one-second capture gap is preserved.
    assert_eq!(timeline.place(Some(16_960), 17_280, 320), 16_960);
}
