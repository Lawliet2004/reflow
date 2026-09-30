use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use serde_json::{json, Value};

use super::prompt::build_messages;
use super::safety::accept_rewrite;

pub const CIRCUIT_BREAKER_MAX_FAILURES: usize = 3;
pub const CIRCUIT_BREAKER_RESET_DURATION: Duration = Duration::from_secs(300); // 5 minutes

#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    failures: Arc<RwLock<usize>>,
    tripped_until: Arc<RwLock<Option<Instant>>>,
    max_failures: usize,
    trip_duration: Duration,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new(CIRCUIT_BREAKER_MAX_FAILURES, CIRCUIT_BREAKER_RESET_DURATION)
    }
}

impl CircuitBreaker {
    pub fn new(max_failures: usize, trip_duration: Duration) -> Self {
        Self {
            failures: Arc::new(RwLock::new(0)),
            tripped_until: Arc::new(RwLock::new(None)),
            max_failures: max_failures.max(1),
            trip_duration,
        }
    }

    pub fn is_open(&self) -> bool {
        let mut tripped = self.tripped_until.write();
        if let Some(deadline) = *tripped {
            if Instant::now() < deadline {
                return true;
            }
            *tripped = None;
            *self.failures.write() = 0;
        }
        false
    }

    pub fn record_success(&self) {
        *self.failures.write() = 0;
        *self.tripped_until.write() = None;
    }

    pub fn record_failure(&self) {
        let mut failures = self.failures.write();
        *failures += 1;
        if *failures >= self.max_failures {
            let deadline = Instant::now() + self.trip_duration;
            *self.tripped_until.write() = Some(deadline);
            log::warn!(
                "Flow refinement circuit breaker tripped after {} consecutive failures; pausing refinement for {:?}",
                *failures,
                self.trip_duration
            );
        }
    }

    pub fn reset(&self) {
        *self.failures.write() = 0;
        *self.tripped_until.write() = None;
    }

    pub fn failure_count(&self) -> usize {
        *self.failures.read()
    }
}

/// Canonical chat-completions `model` field for each shipped GGUF. The name
/// must match what `llama-server` (with `--jinja` reading the GGUF metadata)
/// expects; mismatched names cause 4xx errors or routing to a non-existent
/// model slot.
const MODEL_NAME_TABLE: &[(&str, &str)] = &[
    ("qwen3.5-2b", "Qwen3.5-2B"),
    ("qwen3.5-0.8b", "Qwen3.5-0.8B"),
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
    pub breaker: CircuitBreaker,
    /// Upper bound on generated tokens, derived from the server''s context window.
    ///
    /// Held here because only the runtime that launched `llama-server` knows what
    /// `--ctx-size` it was given, and asking for more completion than the window
    /// can hold makes the server refuse or truncate.
    pub max_completion_tokens: usize,
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
    /// Reserve for the shared prompt prefix when deriving a completion budget.
    /// The prefix is fixed, so this does not need to be exact — only safe.
    const PROMPT_RESERVE_TOKENS: usize = 512;

    /// Largest completion that fits `context_size` alongside the prompt.
    pub fn completion_budget(context_size: u32) -> usize {
        (context_size as usize)
            .saturating_sub(Self::PROMPT_RESERVE_TOKENS)
            .max(128)
    }

    pub fn new_missing() -> Self {
        Self {
            base_url: None,
            timeout: Duration::from_secs(12),
            breaker: CircuitBreaker::default(),
            max_completion_tokens: Self::completion_budget(1024),
        }
    }

    pub fn new_url(url: String, timeout: Duration) -> Self {
        Self {
            base_url: Some(url.trim_end_matches('/').to_string()),
            timeout,
            breaker: CircuitBreaker::default(),
            max_completion_tokens: Self::completion_budget(1024),
        }
    }

    pub fn new_with_breaker(url: String, timeout: Duration, breaker: CircuitBreaker) -> Self {
        Self {
            base_url: Some(url.trim_end_matches('/').to_string()),
            timeout,
            breaker,
            max_completion_tokens: Self::completion_budget(1024),
        }
    }

    /// Same client, with the completion budget matched to a known context window.
    pub fn with_context_size(mut self, context_size: u32) -> Self {
        self.max_completion_tokens = Self::completion_budget(context_size);
        self
    }

    /// Blocking HTTP POST `{url}/v1/chat/completions` using the OpenAI schema.
    pub fn rewrite(&self, req: &RewriteRequest) -> Result<String, String> {
        if self.breaker.is_open() {
            return Err(
                "flow refinement circuit breaker is open (disabled after consecutive failures)"
                    .into(),
            );
        }

        let Some(base) = self.base_url.as_ref() else {
            return Err("flow rewriter is not available".into());
        };
        let url = format!("{base}/v1/chat/completions");
        let timeout = self.timeout;
        let messages = build_messages(req);
        let word_count = req.text.split_whitespace().count();
        // A cleanup rewrite is about as long as its input, so the budget has to
        // scale with the input rather than sit at a constant.
        //
        // The old ceiling was 256 tokens — roughly 190 words — which silently cut
        // off the polished text of any longer dictation mid-sentence. The floor
        // matters too: a very short utterance still needs room for punctuation and
        // capitalisation. `3x words` is generous headroom for a model that is
        // meant to return the same content, and the upper bound keeps a runaway
        // generation from consuming the whole context window.
        let max_tokens = (word_count * 3).clamp(64, self.max_completion_tokens);
        let model_name = model_name_for(&req.model_id);
        let body = json!({
            "model": model_name,
            "temperature": 0.0,
            "top_p": 1.0,
            "repeat_penalty": 1.0,
            "max_tokens": max_tokens,
            "stop": [
                "<|im_end|>",
                "<|endoftext|>",
                "\n\nTranscript:",
                "\nUser:",
            ],
            "messages": messages,
        });

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = run_chat_completion(url, body, timeout);
            let _ = tx.send(result);
        });

        let outcome = rx
            .recv_timeout(timeout + Duration::from_secs(2))
            .map_err(|_| "flow rewrite timed out".to_string())?;

        match outcome {
            Ok(res) => {
                self.breaker.record_success();
                Ok(res)
            }
            Err(err) => {
                self.breaker.record_failure();
                Err(err)
            }
        }
    }
}

/// One HTTP client and one runtime for the whole process.
///
/// `run_chat_completion` used to build a fresh `reqwest::Client` *and* stand up a
/// fresh current-thread tokio runtime on every rewrite. Both are expensive
/// relative to the request they serve — the completion itself measures ~200-420 ms
/// — and both happen on the dictation hot path, after the user has stopped
/// speaking and is waiting for text. Building them once also lets the client keep
/// the keep-alive connection to llama-server between dictations, so the TCP and
/// HTTP handshakes disappear from every rewrite after the first.
static FLOW_HTTP: OnceLock<Result<(tokio::runtime::Runtime, reqwest::Client), String>> =
    OnceLock::new();

fn flow_http() -> Result<&'static (tokio::runtime::Runtime, reqwest::Client), String> {
    FLOW_HTTP
        .get_or_init(|| {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .map_err(|e| format!("flow runtime: {e}"))?;
            // No client-level timeout: the per-request timeout is applied at the
            // call site, which is what makes a single shared client usable by
            // callers with different deadlines.
            let client = reqwest::Client::builder()
                .pool_idle_timeout(Duration::from_secs(300))
                .build()
                .map_err(|e| format!("flow http client: {e}"))?;
            Ok((runtime, client))
        })
        .as_ref()
        .map_err(|e| e.clone())
}

fn run_chat_completion(url: String, body: Value, timeout: Duration) -> Result<String, String> {
    let (runtime, client) = flow_http()?;
    runtime.block_on(async move {
        let response = client
            .post(&url)
            .timeout(timeout)
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
    while let Some(start) = out.find("<think>") {
        if let Some(rel_end) = out[start..].find("</think>") {
            let end = start + rel_end + "</think>".len();
            out.replace_range(start..end, "");
        } else {
            out.truncate(start);
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
    // `raw` is the one level that genuinely forbids Stage 2: the user asked for
    // their words back verbatim.
    //
    // `light` used to be refused here too, which contradicted
    // `ResolvedIntent::run_llm` — the authoritative gate, which permits it. The
    // caller checks `run_llm`, starts llama-server, holds the VRAM, and then
    // this function threw the rewrite away, so `smart_flow` + `light` paid the
    // entire cost of refinement and silently produced none of it. `cleanup_level`
    // describes how aggressive the deterministic Stage 1 pipeline is; it is not
    // the Stage 2 switch.
    if level == "raw" {
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

    // Nothing to gain: skip the round trip rather than spend ~400 ms risking a
    // change to text that is already finished. Not an error — the transcript is
    // returned exactly as Stage 1 produced it.
    if super::safety::polish_would_add_nothing(smart_text) {
        log::debug!("Skipping polish: Stage 1 output is already clean");
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
            model_id: "qwen3.5-0.8b".into(),
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
        // Must be text that actually needs polishing: an already-clean sentence
        // is now skipped before the client is consulted, so it would never
        // exercise the missing-rewriter path.
        let smart = "hello world today";
        let outcome = polish_or_fallback(&client, smart, &sample_req("medium", "normal", smart));
        assert_eq!(outcome.final_text, smart);
        assert!(!outcome.used);
        // A missing rewriter is an attempted rewrite that failed; callers
        // surface the error while keeping the original transcript.
        assert!(outcome.error.is_some());
    }

    /// A finished sentence needs no polish, so a missing runtime is not a
    /// failure. Reporting one would put an error in front of the user about work
    /// that did not need doing.
    #[test]
    fn a_clean_transcript_reports_no_error_even_with_no_runtime() {
        let client = FlowClient::new_missing();
        let smart = "Ship it on Thursday.";
        let outcome = polish_or_fallback(&client, smart, &sample_req("high", "normal", smart));
        assert_eq!(outcome.final_text, smart);
        assert!(!outcome.used);
        assert!(
            outcome.error.is_none(),
            "skipping unnecessary work must not surface an error: {:?}",
            outcome.error
        );
    }

    #[test]
    fn raw_light_and_coding_skip_llm() {
        let client = FlowClient {
            base_url: None,
            timeout: Duration::from_millis(50),
            breaker: CircuitBreaker::default(),
            max_completion_tokens: FlowClient::completion_budget(1024),
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
        let outcome = polish_or_fallback(&client, orig, &sample_req("medium", "normal", orig));
        assert_eq!(outcome.final_text, "Hello world today.");
        assert!(outcome.used);
        assert!(outcome.error.is_none());
    }

    #[test]
    fn stub_meta_output_falls_back() {
        let orig = "Hello";
        let url = spawn_stub("Sure, here is the rewritten text:\n\nHello");
        let client = FlowClient::new_url(url, Duration::from_secs(3));
        let outcome = polish_or_fallback(&client, orig, &sample_req("medium", "normal", orig));
        assert_eq!(outcome.final_text, orig);
        assert!(!outcome.used);
        assert!(outcome.error.is_some());
    }

    #[test]
    fn strip_think_removes_multiple_blocks() {
        assert_eq!(
            strip_think("<think>a</think>Hello<think>b</think> world"),
            "Hello world"
        );
    }

    #[test]
    fn strip_think_drops_unclosed_block() {
        assert_eq!(strip_think("Hello<think>unfinished"), "Hello");
    }

    #[test]
    fn model_name_table_maps_known_ids() {
        assert_eq!(model_name_for("qwen3.5-2b"), "Qwen3.5-2B");
        assert_eq!(model_name_for("qwen3.5-0.8b"), "Qwen3.5-0.8B");
        // Unknown IDs are passed through unchanged so the user sees the
        // raw id in any error message instead of getting silently
        // rewritten to something misleading.
        assert_eq!(model_name_for("custom-model"), "custom-model");
    }

    #[test]
    fn circuit_breaker_trips_after_3_failures_and_fast_fails() {
        let breaker = CircuitBreaker::new(3, Duration::from_secs(300));
        assert!(!breaker.is_open());
        assert_eq!(breaker.failure_count(), 0);

        breaker.record_failure();
        assert!(!breaker.is_open());
        assert_eq!(breaker.failure_count(), 1);

        breaker.record_failure();
        assert!(!breaker.is_open());
        assert_eq!(breaker.failure_count(), 2);

        // 3rd failure trips the breaker
        breaker.record_failure();
        assert!(breaker.is_open());

        let client = FlowClient::new_with_breaker(
            "http://127.0.0.1:9999".into(),
            Duration::from_secs(12),
            breaker.clone(),
        );
        let start = std::time::Instant::now();
        let err = client
            .rewrite(&sample_req("medium", "normal", "test sentence"))
            .unwrap_err();
        assert!(start.elapsed() < Duration::from_millis(50));
        assert!(err.contains("circuit breaker is open"));

        // Manual reset clears the breaker
        breaker.reset();
        assert!(!breaker.is_open());
        assert_eq!(breaker.failure_count(), 0);
    }

    #[test]
    fn circuit_breaker_auto_resets_after_expiry() {
        let breaker = CircuitBreaker::new(2, Duration::from_millis(50));
        breaker.record_failure();
        breaker.record_failure();
        assert!(breaker.is_open());

        std::thread::sleep(Duration::from_millis(60));
        assert!(!breaker.is_open());
        assert_eq!(breaker.failure_count(), 0);
    }
}
