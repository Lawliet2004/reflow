use std::time::Duration;

use serde_json::{json, Value};

use super::prompt::build_prompts;
use super::safety::accept_rewrite;

/// Canonical chat-completions `model` field for each shipped GGUF. The name
/// must match what `llama-server` (with `--jinja` reading the GGUF metadata)
/// expects; mismatched names cause 4xx errors or routing to a non-existent
/// model slot.
const MODEL_NAME_TABLE: &[(&str, &str)] = &[
    ("qwen3.5-2b", "Qwen3.5-2B-Instruct"),
    ("lfm2.5-1.2b", "LFM2.5-1.2B-Instruct"),
];

fn model_name_for(id: &str) -> &str {
    MODEL_NAME_TABLE
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, v)| *v)
        .unwrap_or(id)
}

#[derive(Clone, Debug)]
pub struct FlowClient {
    pub base_url: Option<String>,
    pub timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct RewriteRequest {
    pub text: String,
    pub cleanup_level: String,
    pub style: String,
    pub dictation_mode: String,
    pub vocabulary: Vec<String>,
    pub app_process: String,
    pub model_id: String,
}

impl FlowClient {
    pub fn new_missing() -> Self {
        Self {
            base_url: None,
            timeout: Duration::from_secs(12),
        }
    }

    pub fn new_url(url: String, timeout: Duration) -> Self {
        Self {
            base_url: Some(url.trim_end_matches('/').to_string()),
            timeout,
        }
    }

    /// Blocking HTTP POST `{url}/v1/chat/completions` using the OpenAI schema.
    pub fn rewrite(&self, req: &RewriteRequest) -> Result<String, String> {
        let Some(base) = self.base_url.as_ref() else {
            return Err("flow rewriter is not available".into());
        };
        let url = format!("{base}/v1/chat/completions");
        let timeout = self.timeout;
        let (system, user) = build_prompts(req);
        let word_count = req.text.split_whitespace().count();
        let max_tokens = ((word_count * 3).max(32)).min(256);
        let model_name = model_name_for(&req.model_id);
        let body = json!({
            "model": model_name,
            "temperature": 0.0,
            "top_p": 1.0,
            "repeat_penalty": 1.1,
            "max_tokens": max_tokens,
            "stop": [
                "<|im_end|>",
                "<|endoftext|>",
                "\n\nTranscript:",
                "\nUser:",
            ],
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
        });

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = run_chat_completion(url, body, timeout);
            let _ = tx.send(result);
        });

        rx.recv_timeout(timeout + Duration::from_secs(2))
            .map_err(|_| "flow rewrite timed out".to_string())?
    }
}

fn run_chat_completion(url: String, body: Value, timeout: Duration) -> Result<String, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("flow runtime: {e}"))?;
    rt.block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| format!("flow http client: {e}"))?;
        let response = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("flow request failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("flow HTTP {}", response.status()));
        }
        let payload: Value = response
            .json()
            .await
            .map_err(|e| format!("flow invalid json: {e}"))?;
        extract_content(&payload)
    })
}

fn extract_content(payload: &Value) -> Result<String, String> {
    let content = &payload["choices"][0]["message"]["content"];
    let raw = if let Some(text) = content.as_str() {
        text.to_string()
    } else if let Some(parts) = content.as_array() {
        let mut out = String::new();
        for part in parts {
            if let Some(text) = part.as_str() {
                out.push_str(text);
            } else if let Some(text) = part.get("text").and_then(Value::as_str) {
                out.push_str(text);
            }
        }
        if out.is_empty() {
            return Err("missing completion content".into());
        }
        out
    } else {
        return Err("missing completion content".into());
    };
    Ok(strip_think(&raw))
}

fn strip_think(text: &str) -> String {
    let mut out = text.to_string();
    if let Some(start) = out.find("<think>") {
        if let Some(rel_end) = out[start..].find("</think>") {
            let end = start + rel_end + "</think>".len();
            out.replace_range(start..end, "");
        }
    }
    out.trim().to_string()
}

/// Outcome of a polishing attempt.
///
/// `final_text` is the text to inject (or display). `used` indicates whether
/// the LLM actually rewrote the input. `error` is `Some` when the rewriter
/// was attempted but failed, including the safety gate rejecting an
/// otherwise-OK response. Callers are expected to surface `error` to the
/// user instead of silently degrading.
pub fn polish_or_fallback(
    client: &FlowClient,
    smart_text: &str,
    req: &RewriteRequest,
) -> PolishOutcome {
    let level = req.cleanup_level.trim().to_ascii_lowercase();
    if level == "raw" || level == "light" || level.is_empty() {
        return PolishOutcome {
            final_text: smart_text.to_string(),
            used: false,
            error: None,
        };
    }
    if req.dictation_mode.trim().eq_ignore_ascii_case("coding") {
        return PolishOutcome {
            final_text: smart_text.to_string(),
            used: false,
            error: None,
        };
    }
    if smart_text.trim().is_empty() {
        return PolishOutcome {
            final_text: smart_text.to_string(),
            used: false,
            error: None,
        };
    }

    let mut effective = req.clone();
    effective.text = smart_text.to_string();
    match client.rewrite(&effective) {
        Ok(candidate) => match accept_rewrite(smart_text, &candidate, level.as_str()) {
            Some(safe) => PolishOutcome {
                final_text: safe,
                used: true,
                error: None,
            },
            None => PolishOutcome {
                final_text: smart_text.to_string(),
                used: false,
                error: Some("LLM rewrite rejected by safety gate".into()),
            },
        },
        Err(err) => PolishOutcome {
            final_text: smart_text.to_string(),
            used: false,
            error: Some(err),
        },
    }
}

#[derive(Debug, Clone)]
pub struct PolishOutcome {
    pub final_text: String,
    pub used: bool,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router};
    use serde_json::Value;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use tokio::net::TcpListener;

    fn sample_req(level: &str, mode: &str, text: &str) -> RewriteRequest {
        RewriteRequest {
            text: text.into(),
            cleanup_level: level.into(),
            style: "neutral".into(),
            dictation_mode: mode.into(),
            vocabulary: Vec::new(),
            app_process: String::new(),
            model_id: "lfm2.5-1.2b".into(),
        }
    }

    fn spawn_stub(content: &str) -> String {
        let body = Arc::new(content.to_string());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("stub runtime");
            rt.block_on(async move {
                let app = Router::new().route(
                    "/v1/chat/completions",
                    post({
                        let body = Arc::clone(&body);
                        move |Json(_req): Json<Value>| {
                            let body = Arc::clone(&body);
                            async move {
                                Json(json!({
                                    "choices": [{
                                        "message": {
                                            "role": "assistant",
                                            "content": body.as_str()
                                        }
                                    }]
                                }))
                            }
                        }
                    }),
                );
                let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
                let addr: SocketAddr = listener.local_addr().expect("addr");
                let _ = tx.send(format!("http://{addr}"));
                axum::serve(listener, app).await.ok();
            });
        });
        rx.recv().expect("stub addr")
    }

    #[test]
    fn missing_server_falls_back() {
        let client = FlowClient::new_missing();
        let smart = "Hello world.";
        let outcome =
            polish_or_fallback(&client, smart, &sample_req("medium", "normal", smart));
        assert_eq!(outcome.final_text, smart);
        assert!(!outcome.used);
        assert!(outcome.error.is_none());
    }

    #[test]
    fn raw_light_and_coding_skip_llm() {
        let client = FlowClient {
            base_url: None,
            timeout: Duration::from_millis(50),
        };
        let smart = "Keep this.";
        assert!(!polish_or_fallback(&client, smart, &sample_req("raw", "normal", smart)).used);
        assert!(!polish_or_fallback(&client, smart, &sample_req("light", "normal", smart)).used);
        assert!(!polish_or_fallback(&client, smart, &sample_req("medium", "coding", smart)).used);
    }

    #[test]
    fn stub_server_safe_rewrite_is_accepted() {
        let orig = "hello world today";
        let url = spawn_stub("Hello world today.");
        let client = FlowClient::new_url(url, Duration::from_secs(3));
        let rewritten = client
            .rewrite(&sample_req("medium", "normal", orig))
            .expect("rewrite");
        assert_eq!(rewritten, "Hello world today.");
        let outcome =
            polish_or_fallback(&client, orig, &sample_req("medium", "normal", orig));
        assert_eq!(outcome.final_text, "Hello world today.");
        assert!(outcome.used);
        assert!(outcome.error.is_none());
    }

    #[test]
    fn stub_meta_output_falls_back() {
        let orig = "Hello";
        let url = spawn_stub("Sure, here is the rewritten text:\n\nHello");
        let client = FlowClient::new_url(url, Duration::from_secs(3));
        let outcome =
            polish_or_fallback(&client, orig, &sample_req("medium", "normal", orig));
        assert_eq!(outcome.final_text, orig);
        assert!(!outcome.used);
        assert!(outcome.error.is_some());
    }

    #[test]
    fn model_name_table_maps_known_ids() {
        assert_eq!(model_name_for("qwen3.5-2b"), "Qwen3.5-2B-Instruct");
        assert_eq!(model_name_for("lfm2.5-1.2b"), "LFM2.5-1.2B-Instruct");
        // Unknown IDs are passed through unchanged so the user sees the
        // raw id in any error message instead of getting silently
        // rewritten to something misleading.
        assert_eq!(model_name_for("custom-model"), "custom-model");
    }
}
