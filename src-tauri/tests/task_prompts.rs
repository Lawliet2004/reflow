use reflow_lib::rewrite::{
    prompt::{build_task_messages, cacheable_prefix},
    safety::accept_task_output,
};

#[test]
fn task_prefix_stays_stable_and_private_input_stays_in_user_turn() {
    let a = build_task_messages("Edit text.", "Shorten", &[("Text", "private source")]);
    let b = build_task_messages(
        "Edit text.",
        "Translate",
        &[("Clipboard", "private clipboard")],
    );
    assert_eq!(a[0], b[0]);
    assert!(!a[0].to_string().contains("private"));
    assert!(a[1]["content"].as_str().unwrap().contains("private source"));
    assert_ne!(a[0], cacheable_prefix()[0]);
    assert!(!a[0].to_string().contains("Never translate"));
}

#[test]
fn tasks_accept_transformations_and_reject_refusals_and_meta() {
    assert_eq!(
        accept_task_output("Translate hello", "Bonjour").as_deref(),
        Some("Bonjour")
    );
    assert!(accept_task_output("shorten", "Here is the result: Hi").is_none());
    assert!(accept_task_output("shorten", "I cannot help with this.").is_none());
    assert!(accept_task_output("shorten", " ").is_none());
}
