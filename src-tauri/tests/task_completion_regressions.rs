//! Task completion and context cleanup contracts against an isolated local model.

use axum::{http::StatusCode, response::IntoResponse, routing::post, Json, Router};
use reflow_lib::rewrite::{prompt::cacheable_prefix, FlowClient};
use reflow_lib::session::{postprocess_with_context, SessionContext};
use reflow_lib::settings::{AppSettings, Mode, Snippet};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct ModelStub {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl ModelStub {
    fn new(content: &str, prompt_tokens: Option<usize>) -> Self {
        let response = content.to_owned();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let (ready, address) = std::sync::mpsc::channel();
        let (shutdown, stopped) = tokio::sync::oneshot::channel();
        let worker = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    let app = Router::new()
                        .route("/apply-template", post(move || async move {
                            if prompt_tokens.is_some() {
                                Json(json!({"prompt": "rendered prompt"})).into_response()
                            } else {
                                StatusCode::NOT_FOUND.into_response()
                            }
                        }))
                        .route("/tokenize", post(move || async move {
                            Json(json!({"tokens": vec![1; prompt_tokens.unwrap_or(0)]}))
                        }))
                        .route("/v1/chat/completions", post(move |Json(body): Json<Value>| {
                            let response = response.clone();
                            let recorded = recorded.clone();
                            async move {
                                recorded.lock().unwrap().push(body);
                                Json(json!({"choices": [{"message": {"content": response}, "finish_reason": "stop"}]}))
                            }
                        }));
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                    ready.send(format!("http://{}", listener.local_addr().unwrap())).unwrap();
                    axum::serve(listener, app)
                        .with_graceful_shutdown(async { let _ = stopped.await; })
                        .await
                        .unwrap();
                });
        });
        Self {
            url: address.recv_timeout(Duration::from_secs(5)).unwrap(),
            requests,
            shutdown: Some(shutdown),
            worker: Some(worker),
        }
    }

    fn client(&self, context_size: u32) -> FlowClient {
        FlowClient::new_url(self.url.clone(), Duration::from_secs(3))
            .with_context_size(context_size)
    }

    fn requests(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for ModelStub {
    fn drop(&mut self) {
        let _ = self.shutdown.take().unwrap().send(());
        self.worker.take().unwrap().join().unwrap();
    }
}

fn cleanup_settings() -> AppSettings {
    AppSettings {
        intelligence_tier: "smart_flow".into(),
        cleanup_level: "medium".into(),
        ..AppSettings::default()
    }
}

#[test]
fn custom_mode_instruction_and_private_context_reach_the_final_user_turn() {
    let server = ModelStub::new("Bonjour.", Some(100));
    let mode = Mode {
        custom_instructions: "Translate to French and keep the greeting.".into(),
        ..Mode::default()
    };
    let context = SessionContext {
        selected_text: Some("private selected greeting".into()),
        ..SessionContext::default()
    };
    let outcome = postprocess_with_context(
        "hello",
        &cleanup_settings(),
        "",
        &server.client(1024),
        Some(&mode),
        &context,
    );

    assert_eq!(outcome.final_text, "Bonjour.");
    assert!(outcome.rewriter_used);
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let messages = requests[0]["messages"].as_array().unwrap();
    let user = messages.last().unwrap();
    assert_eq!(user["role"], "user");
    assert!(user["content"]
        .as_str()
        .unwrap()
        .contains(&mode.custom_instructions));
    assert!(user["content"]
        .as_str()
        .unwrap()
        .contains("private selected greeting"));
    assert!(!messages[0]
        .to_string()
        .contains("private selected greeting"));
}

#[test]
fn cleanup_with_context_keeps_the_cacheable_prefix_unchanged() {
    let server = ModelStub::new("I will send the report tomorrow.", Some(200));
    let context = SessionContext {
        window_title: Some("private report editor".into()),
        ..SessionContext::default()
    };
    let outcome = postprocess_with_context(
        "i will send the report tomorrow",
        &cleanup_settings(),
        "",
        &server.client(1024),
        None,
        &context,
    );

    assert!(outcome.rewriter_used, "{:?}", outcome.rewriter_error);
    let requests = server.requests();
    let messages = requests[0]["messages"].as_array().unwrap();
    let prefix = cacheable_prefix();
    assert_eq!(&messages[..prefix.len()], prefix.as_slice());
    assert!(messages.last().unwrap()["content"]
        .as_str()
        .unwrap()
        .contains("private report editor"));
}

#[test]
fn refused_or_meta_task_results_preserve_the_entire_dictation() {
    for rejected in ["I cannot help with this.", "Here is the result: done", " "] {
        let server = ModelStub::new(rejected, Some(100));
        let mode = Mode {
            custom_instructions: "Shorten the dictation.".into(),
            ..Mode::default()
        };
        let outcome = postprocess_with_context(
            "Keep every detail from this dictation.",
            &cleanup_settings(),
            "",
            &server.client(1024),
            Some(&mode),
            &SessionContext::default(),
        );

        assert_eq!(outcome.final_text, outcome.smart, "rejected: {rejected}");
        assert!(!outcome.rewriter_used);
        assert!(outcome.rewriter_error.is_some());
    }
}

#[test]
fn coding_dictation_skips_tasks_even_when_a_custom_mode_requests_transformation() {
    let server = ModelStub::new("This output must never be used.", Some(100));
    let mode = Mode {
        custom_instructions: "Rewrite as prose.".into(),
        ..Mode::default()
    };
    let mut settings = cleanup_settings();
    settings.dictation_mode = "coding".into();
    let outcome = postprocess_with_context(
        "const sample = 42;",
        &settings,
        "",
        &server.client(1024),
        Some(&mode),
        &SessionContext::default(),
    );

    assert!(!outcome.rewriter_used);
    assert!(outcome.rewrite_ms.is_none());
    assert!(server.requests().is_empty());
}

#[test]
fn task_completion_uses_only_the_remaining_context_for_output() {
    let server = ModelStub::new("Result.", Some(160));
    let client = server.client(256);
    assert_eq!(
        client
            .complete(vec![json!({"role":"user", "content":"Shorten this text."})])
            .unwrap(),
        "Result."
    );

    let requests = server.requests();
    assert_eq!(requests[0]["max_tokens"], 96);
}

#[test]
fn task_that_cannot_fit_the_minimum_answer_never_dispatches_completion() {
    let server = ModelStub::new("Truncated result.", Some(240));
    let client = server.client(256);
    assert!(client
        .complete(vec![json!({"role":"user", "content":"Shorten this text."})])
        .is_err());
    assert!(server.requests().is_empty());
}

#[test]
fn cleanup_fallback_accounting_includes_all_private_context() {
    // Older llama-server builds do not expose /apply-template. A conservative
    // fallback must still account for the context included in the user message.
    let server = ModelStub::new("I will send the report tomorrow.", None);
    let context = SessionContext {
        selected_text: Some("a".repeat(10_000)),
        clipboard: Some("b".repeat(10_000)),
        window_title: Some("c".repeat(10_000)),
        ..SessionContext::default()
    };
    let _ = postprocess_with_context(
        "i will send the report tomorrow",
        &cleanup_settings(),
        "",
        &server.client(700),
        None,
        &context,
    );

    for request in server.requests() {
        let prompt_estimate: usize = request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["content"].as_str().unwrap().chars().count().div_ceil(4))
            .sum::<usize>()
            + 64;
        let output_budget = request["max_tokens"].as_u64().unwrap() as usize;
        assert!(
            prompt_estimate + output_budget <= 700,
            "oversized request dispatched: {prompt_estimate} prompt + {output_budget} answer > 700"
        );
    }
}

#[test]
fn snippets_are_inserted_after_task_generation_and_preserve_exact_stored_text() {
    let server = ModelStub::new("Send to insert my address.", Some(100));
    let mode = Mode {
        custom_instructions: "Write a delivery sentence.".into(),
        ..Mode::default()
    };
    let expansion = "42 ruE de PARIS\nFRANCE\ninsert my address";
    let mut settings = cleanup_settings();
    settings.snippets.push(Snippet {
        id: "address".into(),
        trigger: "insert my address".into(),
        expansion: expansion.into(),
        enabled: true,
    });
    let outcome = postprocess_with_context(
        "send to insert my address",
        &settings,
        "",
        &server.client(1024),
        Some(&mode),
        &SessionContext::default(),
    );

    assert!(outcome.rewriter_used);
    assert_eq!(outcome.final_text, format!("Send to {expansion}."));
    assert!(!server.requests()[0].to_string().contains("42 ruE de PARIS"));
}
