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
    state: Arc<RwLock<BreakerState>>,
    max_failures: usize,
    trip_duration: Duration,
}

#[derive(Debug, Default)]
struct BreakerState {
    failures: usize,
    tripped_until: Option<Instant>,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new(CIRCUIT_BREAKER_MAX_FAILURES, CIRCUIT_BREAKER_RESET_DURATION)
    }
}

impl CircuitBreaker {
    pub fn new(max_failures: usize, trip_duration: Duration) -> Self {
        Self {
            state: Arc::new(RwLock::new(BreakerState::default())),
            max_failures: max_failures.max(1),
            trip_duration,
        }
    }

    pub fn is_open(&self) -> bool {
        let mut state = self.state.write();
        if let Some(deadline) = state.tripped_until {
            if Instant::now() < deadline {
                return true;
            }
            state.tripped_until = None;
            state.failures = 0;
        }
        false
    }

    pub fn record_success(&self) {
        self.reset();
    }

    pub fn record_failure(&self) {
        let mut state = self.state.write();
        state.failures = state.failures.saturating_add(1);
        if state.failures >= self.max_failures {
            let deadline = Instant::now() + self.trip_duration;
            state.tripped_until = Some(deadline);
            log::warn!(
                "Flow refinement circuit breaker tripped after {} consecutive failures; pausing refinement for {:?}",
                state.failures,
                self.trip_duration
            );
        }
    }

    pub fn reset(&self) {
        *self.state.write() = BreakerState::default();
    }

    pub fn failure_count(&self) -> usize {
        self.state.read().failures
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

/// `--ctx-size` used when the caller never declares one.
const DEFAULT_CONTEXT_SIZE: u32 = 1024;

/// ~4 characters per token is the prose average — the fallback when the
/// server's `/tokenize` endpoint is unavailable.
const CHARS_PER_TOKEN: usize = 4;

#[derive(Clone, Debug)]
pub struct FlowClient {
    pub generation: std::sync::Arc<parking_lot::RwLock<GenerationMetrics>>,
    pub base_url: Option<String>,
    pub timeout: Duration,
    pub breaker: CircuitBreaker,
    /// Upper bound on generated tokens, derived from the server''s context window.
    ///
    /// Held here because only the runtime that launched `llama-server` knows what
    /// `--ctx-size` it was given, and asking for more completion than the window
    /// can hold makes the server refuse or truncate.
    pub max_completion_tokens: usize,
    /// The `--ctx-size` the live server was launched with. The input fit check
    /// needs the window itself, not just the completion budget derived from it.
    pub context_size: u32,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct GenerationMetrics {
    pub prompt_tokens: u64,
    pub output_tokens: u64,
    pub prompt_ms: Option<f64>,
    pub decode_ms: Option<f64>,
}
fn generation_metrics(payload: &Value) -> GenerationMetrics {
    let timing = |key: &str| {
        payload["timings"][key]
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 0.0)
    };
    GenerationMetrics {
        prompt_tokens: payload["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
        output_tokens: payload["usage"]["completion_tokens"].as_u64().unwrap_or(0),
        prompt_ms: timing("prompt_ms"),
        decode_ms: timing("predicted_ms"),
    }
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
            generation: Default::default(),
            base_url: None,
            timeout: Duration::from_secs(12),
            breaker: CircuitBreaker::default(),
            max_completion_tokens: Self::completion_budget(DEFAULT_CONTEXT_SIZE),
            context_size: DEFAULT_CONTEXT_SIZE,
        }
    }

    pub fn new_url(url: String, timeout: Duration) -> Self {
        Self {
            generation: Default::default(),
            base_url: Some(url.trim_end_matches('/').to_string()),
            timeout,
            breaker: CircuitBreaker::default(),
            max_completion_tokens: Self::completion_budget(DEFAULT_CONTEXT_SIZE),
            context_size: DEFAULT_CONTEXT_SIZE,
        }
    }

    pub fn new_with_breaker(url: String, timeout: Duration, breaker: CircuitBreaker) -> Self {
        Self {
            generation: Default::default(),
            base_url: Some(url.trim_end_matches('/').to_string()),
            timeout,
            breaker,
            max_completion_tokens: Self::completion_budget(DEFAULT_CONTEXT_SIZE),
            context_size: DEFAULT_CONTEXT_SIZE,
        }
    }

    /// Same client, with the completion budget matched to a known context window.
    pub fn with_context_size(mut self, context_size: u32) -> Self {
        self.max_completion_tokens = Self::completion_budget(context_size);
        self.context_size = context_size;
        self
    }

    /// Local task completion. Transformative output is validated by the caller.
    pub fn complete(&self, messages: Vec<Value>) -> Result<String, String> {
        if self.breaker.is_open() {
            return Err("flow refinement circuit breaker is open".into());
        }
        let base = self
            .base_url
            .as_ref()
            .ok_or("Local refinement model is not available")?;
        let started = Instant::now();
        let deadline = self.timeout.min(Self::DEADLINE_CEILING);
        let estimate = messages
            .iter()
            .filter_map(|m| m["content"].as_str())
            .map(estimate_tokens)
            .sum::<usize>()
            .saturating_add(64);
        let prompt_tokens = self
            .rendered_prompt_tokens(base, &messages, deadline)
            .unwrap_or(estimate);
        let available = (self.context_size as usize).saturating_sub(prompt_tokens);
        if available < 64 {
            return Err("Task input exceeds the local model context window".into());
        }
        let max_tokens = available.min(self.max_completion_tokens);
        let timeout = deadline.saturating_sub(started.elapsed());
        if timeout.is_zero() {
            return Err("Task deadline expired during context accounting".into());
        }
        let body = json!({"temperature": 0.0, "top_p": 1.0, "repeat_penalty": 1.0, "max_tokens": max_tokens,
            "stop": ["<|im_end|>", "<|endoftext|>", "\nUser:"], "messages": messages});
        let url = format!("{base}/v1/chat/completions");
        let generation = self.generation.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(run_chat_completion(url, body, timeout, &generation));
        });
        let result = rx
            .recv_timeout(timeout)
            .map_err(|_| "Local task timed out".to_string())
            .and_then(|result| result);
        if result.is_ok() {
            self.breaker.record_success();
        } else {
            self.breaker.record_failure();
        }
        result
    }

    /// Blocking HTTP POST `{url}/v1/chat/completions` using the OpenAI schema.
    pub fn rewrite(&self, req: &RewriteRequest) -> Result<String, String> {
        self.rewrite_messages(req, build_messages(req))
    }

    pub fn rewrite_messages(
        &self,
        req: &RewriteRequest,
        messages: Vec<Value>,
    ) -> Result<String, String> {
        let started = Instant::now();
        let deadline = self.timeout.min(Self::DEADLINE_CEILING);
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
        let word_count = req.text.split_whitespace().count();
        let input_tokens =
            self.input_token_count(base, &req.text, deadline.saturating_sub(started.elapsed()));
        let prompt_tokens = self
            .rendered_prompt_tokens(base, &messages, deadline.saturating_sub(started.elapsed()))
            .unwrap_or_else(|| {
                let supplied = messages
                    .iter()
                    .filter_map(|m| m["content"].as_str())
                    .map(estimate_tokens)
                    .sum::<usize>()
                    .saturating_add(64);
                supplied.max(input_tokens.saturating_add(Self::PROMPT_RESERVE_TOKENS))
            });
        // A cleanup rewrite is about as long as its input, so the budget has to
        // scale with the input rather than sit at a constant.
        //
        // The old ceiling was 256 tokens — roughly 190 words — which silently cut
        // off the polished text of any longer dictation mid-sentence. The floor
        // matters too: a very short utterance still needs room for punctuation and
        // capitalisation. `3x words` is generous headroom for a model that is
        // meant to return the same content, and the upper bound keeps a runaway
        // generation from consuming the whole context window.
        //
        // `input_tokens * 1.5` joins the floor: whitespace-free input (CJK
        // dictation transcribes without spaces) counts as a single "word",
        // which would otherwise cap a real answer at 64 tokens and hand back a
        // truncated rewrite the safety gate can accept.
        let max_tokens = (word_count * 3)
            .max(input_tokens.saturating_mul(3) / 2)
            .clamp(64, self.max_completion_tokens);
        // Prompt prefix + input + a full-length answer must fit the window
        // whole. If they cannot, llama-server either refuses the request or
        // cuts the generation at the window edge — a half-rewrite is worse
        // than no rewrite, so this rejects early and lets the caller fall back
        // to the deterministic Stage-1 text.
        if prompt_tokens.saturating_add(max_tokens) > self.context_size as usize {
            return Err(format!(
                "flow rewrite input too long: ~{input_tokens} input tokens plus the prompt prefix \
                 and a {max_tokens}-token answer exceed the {}-token context",
                self.context_size
            ));
        }
        let timeout = self
            .request_deadline(input_tokens, max_tokens)
            .saturating_sub(started.elapsed());
        if timeout.is_zero() {
            return Err("flow rewrite deadline expired during context accounting".into());
        }
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
        let generation = self.generation.clone();
        std::thread::spawn(move || {
            let result = run_chat_completion(url, body, timeout, &generation);
            let _ = tx.send(result);
        });

        let outcome = rx
            .recv_timeout(timeout)
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

    /// Hard ceiling so a pathological input cannot park the dictation pipeline.
    const DEADLINE_CEILING: Duration = Duration::from_secs(90);

    /// The configured deadline is a total budget, including context accounting.
    fn request_deadline(&self, _input_tokens: usize, _completion_tokens: usize) -> Duration {
        self.timeout.min(Self::DEADLINE_CEILING)
    }

    /// Context tokens `text` will occupy, measured by the server's `/tokenize`
    /// endpoint when it answers and estimated from length otherwise. The
    /// estimate only has to catch inputs that cannot fit, and it fails safe in
    /// both directions: overestimating rejects into the Stage-1 fallback
    /// early, underestimating lets llama-server refuse the request, which
    /// lands in the same fallback after one wasted round trip.
    fn input_token_count(&self, base: &str, text: &str, remaining: Duration) -> usize {
        let estimate = estimate_tokens(text);
        // 128 bytes cannot approach a 1024-token window under any real
        // tokenizer, so the common short dictation skips the round trip; and
        // an estimate already larger than the usable window decides the fit
        // check on its own, making the call pointless.
        if text.len() <= 128 {
            return estimate;
        }
        match tokenize_len(base, text, remaining.min(Duration::from_secs(2))) {
            Ok(tokens) => tokens,
            Err(err) => {
                log::warn!("flow /tokenize failed ({err}); estimating input size from length");
                estimate
            }
        }
    }
    fn rendered_prompt_tokens(
        &self,
        base: &str,
        messages: &[Value],
        remaining: Duration,
    ) -> Option<usize> {
        let started = Instant::now();
        let result = post_json(
            format!("{base}/apply-template"),
            json!({ "messages": messages }),
            remaining.min(Duration::from_secs(2)),
        )
        .and_then(|value| {
            value["prompt"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "flow /apply-template returned no prompt".into())
        })
        .and_then(|prompt| {
            tokenize_len(
                base,
                &prompt,
                remaining
                    .saturating_sub(started.elapsed())
                    .min(Duration::from_secs(2)),
            )
        });
        match result {
            Ok(tokens) => Some(tokens),
            Err(error) => {
                log::debug!("Full-template accounting unavailable: {error}; using conservative prefix reserve");
                None
            }
        }
    }
}

fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(CHARS_PER_TOKEN)
}

/// POST `{base}/tokenize` and count the returned token ids.
///
/// The endpoint is a plain encode — no inference — so a two-second ceiling is
/// generous even for long input, and any failure degrades to the
/// chars→tokens estimate rather than blocking the rewrite.
fn tokenize_len(base: &str, text: &str, timeout: Duration) -> Result<usize, String> {
    let body = json!({ "content": text });
    if timeout.is_zero() {
        return Err("flow context-accounting deadline expired".into());
    }
    let payload = post_json(format!("{base}/tokenize"), body, timeout)?;
    payload["tokens"]
        .as_array()
        .map(Vec::len)
        .ok_or_else(|| "flow /tokenize response carried no tokens array".into())
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
                .redirect(reqwest::redirect::Policy::none())
                .pool_idle_timeout(Duration::from_secs(300))
                .build()
                .map_err(|e| format!("flow http client: {e}"))?;
            Ok((runtime, client))
        })
        .as_ref()
        .map_err(|e| e.clone())
}

fn run_chat_completion(
    url: String,
    body: Value,
    timeout: Duration,
    generation: &parking_lot::RwLock<GenerationMetrics>,
) -> Result<String, String> {
    post_json(url, body, timeout).and_then(|payload| {
        let current = generation_metrics(&payload);
        let mut total = generation.write();
        total.prompt_tokens += current.prompt_tokens;
        total.output_tokens += current.output_tokens;
        if let Some(ms) = current.prompt_ms {
            total.prompt_ms = Some(total.prompt_ms.unwrap_or(0.0) + ms);
        }
        if let Some(ms) = current.decode_ms {
            total.decode_ms = Some(total.decode_ms.unwrap_or(0.0) + ms);
        }
        extract_content(&payload)
    })
}

/// POST JSON to `url`, requiring a 2xx and a JSON body back.
fn post_json(url: String, body: Value, timeout: Duration) -> Result<Value, String> {
    if timeout.is_zero() {
        return Err("flow request deadline expired".into());
    }
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
        response
            .json()
            .await
            .map_err(|e| format!("flow invalid json: {e}"))
    })
}

fn extract_content(payload: &Value) -> Result<String, String> {
    // A token/context limit can cut an otherwise plausible rewrite mid-sentence.
    // The similarity gate cannot detect every omitted suffix, so preserve Stage 1.
    if payload["choices"][0]["finish_reason"].as_str() == Some("length") {
        return Err(
            "flow completion reached its token or context limit; preserving the full transcript"
                .into(),
        );
    }
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
fn rewrite_segments(
    source: &str,
    level: &str,
    budget: Duration,
    rewrite: &mut impl FnMut(&str, Duration) -> Result<String, String>,
) -> Result<String, String> {
    fn visit(
        source: &str,
        level: &str,
        started: Instant,
        budget: Duration,
        attempts: &mut usize,
        rewrite: &mut impl FnMut(&str, Duration) -> Result<String, String>,
    ) -> Result<String, String> {
        let remaining = budget.saturating_sub(started.elapsed());
        if remaining.is_zero() || *attempts >= 64 {
            return Err("Paragraph refinement reached its total deadline or segment limit; original transcript preserved.".into());
        }
        *attempts += 1;
        match rewrite(source, remaining) {
            Ok(candidate) => accept_rewrite(source, &candidate, level)
                .ok_or_else(|| "LLM rewrite rejected by safety gate".into()),
            Err(error) if error.starts_with("flow rewrite input too long") => {
                // Split only at paragraph/sentence boundaries. Never split a long
                // sentence arbitrarily: that could detach a negation from its verb.
                let middle = source.len() / 2;
                let boundary = source
                    .char_indices()
                    .filter_map(|(index, ch)| {
                        let end = index + ch.len_utf8();
                        (matches!(ch, '\n' | '.' | '!' | '?' | '。' | '！' | '？')
                            && end < source.len()
                            && !source[end..].trim().is_empty())
                        .then_some(end)
                    })
                    .min_by_key(|end| end.abs_diff(middle));
                let Some(boundary) = boundary else {
                    return Err(error);
                };
                let left = &source[..boundary];
                let right = &source[boundary..];
                let leading = right.len() - right.trim_start().len();
                let left_trimmed = left.trim_end();
                let separator = &source[left_trimmed.len()..boundary + leading];
                let first = visit(left_trimmed, level, started, budget, attempts, rewrite)?;
                let second = visit(&right[leading..], level, started, budget, attempts, rewrite)?;
                Ok(format!("{first}{separator}{second}"))
            }
            Err(error) => Err(error),
        }
    }
    visit(source, level, Instant::now(), budget, &mut 0, rewrite)
}

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
    *client.generation.write() = GenerationMetrics::default();
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
    let result = rewrite_segments(
        smart_text,
        &level,
        client.timeout.min(FlowClient::DEADLINE_CEILING),
        &mut |part, remaining| {
            effective.text = part.to_owned();
            let mut bounded = client.clone();
            bounded.timeout = remaining;
            bounded.rewrite(&effective)
        },
    );
    match result {
        Ok(candidate) => PolishOutcome {
            final_text: candidate,
            used: true,
            error: None,
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::net::TcpListener;

    #[test]
    fn segmented_rewrite_preserves_every_paragraph_and_its_separator() {
        let source = "first paragraph has several words.\n\nsecond paragraph has several words.\n\nthird paragraph has several words.";
        let mut requests = 0;
        let result = rewrite_segments(source, "medium", Duration::from_secs(1), &mut |part, _| {
            requests += 1;
            if part.len() > 50 {
                Err("flow rewrite input too long".into())
            } else {
                Ok(part.to_owned())
            }
        })
        .unwrap();
        assert_eq!(result, source);
        assert!(requests > 1);
    }

    #[test]
    fn segmented_rewrite_failure_never_returns_only_completed_paragraphs() {
        let source = "first paragraph has several words.\n\nsecond paragraph has several words.";
        let result = rewrite_segments(source, "medium", Duration::from_secs(1), &mut |part, _| {
            if part.len() > 50 {
                Err("flow rewrite input too long".into())
            } else if part.starts_with("second") {
                Err("generation failed".into())
            } else {
                Ok(part.to_owned())
            }
        });
        assert!(result.is_err());
    }

    #[test]
    fn compressible_input_uses_the_rendered_template_instead_of_early_estimate_rejection() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
                let app = Router::new()
                    .route("/apply-template", post(|| async { Json(json!({"prompt":"rendered template including assistant prefix"})) }))
                    .route("/tokenize", post(|Json(value): Json<Value>| async move {
                        let count = if value["content"].as_str().unwrap_or_default().starts_with("rendered") { 400 } else { 10 };
                        Json(json!({"tokens":vec![1; count]}))
                    }))
                    .route("/v1/chat/completions", post(|| async { Json(json!({"choices":[{"message":{"content":"Cleaned."},"finish_reason":"stop"}]})) }));
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                tx.send(format!("http://{}", listener.local_addr().unwrap())).unwrap();
                axum::serve(listener, app).await.unwrap();
            });
        });
        let client = FlowClient::new_url(rx.recv().unwrap(), Duration::from_secs(2));
        assert_eq!(
            client
                .rewrite(&sample_req("medium", "normal", &"a".repeat(4000)))
                .unwrap(),
            "Cleaned."
        );
    }

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
                                    "usage": {"prompt_tokens": 25, "completion_tokens": 7},
                                    "timings": {"prompt_ms": 12.0, "predicted_ms": 35.0},
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

    /// A stub serving both `/tokenize` — answering `content.len() / 2` as a
    /// deterministic fake count — and `/v1/chat/completions`, which records
    /// each hit so a test can prove a rejected input never consumed a
    /// completion.
    fn spawn_stub_counting(content: &str) -> (String, Arc<AtomicUsize>) {
        let body = Arc::new(content.to_string());
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_out = Arc::clone(&hits);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("stub runtime");
            rt.block_on(async move {
                let app = Router::new()
                    .route(
                        "/tokenize",
                        post(|Json(req): Json<Value>| async move {
                            let len = req["content"].as_str().map(str::len).unwrap_or(0);
                            Json(json!({ "tokens": vec![0; len / 2] }))
                        }),
                    )
                    .route(
                        "/v1/chat/completions",
                        post({
                            let body = Arc::clone(&body);
                            move |Json(_req): Json<Value>| {
                                let body = Arc::clone(&body);
                                let hits = Arc::clone(&hits);
                                async move {
                                    hits.fetch_add(1, Ordering::SeqCst);
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
        (rx.recv().expect("stub addr"), hits_out)
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
            generation: Default::default(),
            base_url: None,
            timeout: Duration::from_millis(50),
            breaker: CircuitBreaker::default(),
            max_completion_tokens: FlowClient::completion_budget(1024),
            context_size: 1024,
        };
        let smart = "Keep this.";
        assert!(!polish_or_fallback(&client, smart, &sample_req("raw", "normal", smart)).used);
        assert!(!polish_or_fallback(&client, smart, &sample_req("light", "normal", smart)).used);
        assert!(!polish_or_fallback(&client, smart, &sample_req("medium", "coding", smart)).used);
    }

    #[test]
    fn completion_metrics_come_from_the_server_and_reset_for_each_polish() {
        let client = FlowClient::new_url(spawn_stub("Hello world today."), Duration::from_secs(3));
        let request = sample_req("medium", "normal", "hello world today");
        assert!(polish_or_fallback(&client, &request.text, &request).used);
        let metrics = client.generation.read().clone();
        assert_eq!((metrics.prompt_tokens, metrics.output_tokens), (25, 7));
        assert_eq!(
            (metrics.prompt_ms, metrics.decode_ms),
            (Some(12.0), Some(35.0))
        );
        assert!(polish_or_fallback(&client, &request.text, &request).used);
        assert_eq!(client.generation.read().output_tokens, 7);
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
    fn truncated_completion_is_rejected_even_when_the_text_looks_valid() {
        let payload = json!({"choices": [{
            "finish_reason": "length",
            "message": {"content": "I would like to ship this"}
        }]});
        let error = extract_content(&payload).expect_err("truncated rewrite must fall back");
        assert!(error.contains("limit"));
        let finished = json!({"choices": [{
            "finish_reason": "stop",
            "message": {"content": "I would like to ship this."}
        }]});
        assert_eq!(
            extract_content(&finished).unwrap(),
            "I would like to ship this."
        );
    }

    #[test]
    fn concurrent_breaker_expiry_and_failures_complete_without_lock_inversion() {
        let breaker = CircuitBreaker::new(1, Duration::ZERO);
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let (tx, rx) = std::sync::mpsc::channel();
        for failing in [true, false] {
            let breaker = breaker.clone();
            let barrier = Arc::clone(&barrier);
            let tx = tx.clone();
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..10_000 {
                    if failing {
                        breaker.record_failure();
                    } else {
                        breaker.is_open();
                    }
                }
                tx.send(()).unwrap();
            });
        }
        for _ in 0..2 {
            rx.recv_timeout(Duration::from_secs(5))
                .expect("breaker operation deadlocked");
        }
        breaker.reset();
        assert_eq!(breaker.failure_count(), 0);
        assert!(!breaker.is_open());
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

    /// A request that cannot fit the window must be refused before the wire:
    /// the deterministic Stage-1 fallback is the honest outcome, not a
    /// completion cut at the context edge.
    #[test]
    fn oversized_input_is_rejected_before_any_request() {
        let client = FlowClient::new_url("http://127.0.0.1:9".into(), Duration::from_secs(3));
        // ~10 KB: ~2500 estimated tokens against the default 1024-token
        // window, so the estimate alone settles the check without a server.
        let err = client
            .rewrite(&sample_req("medium", "normal", &"word ".repeat(2000)))
            .expect_err("oversized input must be rejected");
        assert!(err.contains("too long"), "unexpected error: {err}");
    }

    /// When the estimate alone cannot settle the fit, a dead `/tokenize`
    /// endpoint must degrade to the chars→tokens estimate — still rejecting
    /// an input that cannot fit, without spending a completion.
    #[test]
    fn tokenize_failure_falls_back_to_the_length_estimate() {
        let client = FlowClient::new_url("http://127.0.0.1:9".into(), Duration::from_millis(500));
        // 1000 bytes > 128, so the client tries /tokenize; the dead port
        // fails it and the estimate (~250 tokens) still overflows the window
        // once the answer budget is added.
        let err = client
            .rewrite(&sample_req("medium", "normal", &"a".repeat(1000)))
            .expect_err("an input that cannot fit must still be rejected");
        assert!(err.contains("too long"), "unexpected error: {err}");
    }

    /// The fit check may only gate genuinely oversized text: an input that
    /// fits must reach the server and fail there (dead port), not here.
    #[test]
    fn an_input_that_fits_is_sent_to_the_server() {
        let client = FlowClient::new_url("http://127.0.0.1:9".into(), Duration::from_millis(300));
        let err = client
            .rewrite(&sample_req("medium", "normal", "hello world today"))
            .expect_err("dead port still fails at the wire");
        assert!(
            err.contains("request failed") || err.contains("deadline expired"),
            "fit input must fail at the wire, not the fit check: {err}"
        );
    }

    /// The real `/tokenize` response drives the fit check when the endpoint
    /// answers.
    #[test]
    fn measured_tokens_reject_without_spending_a_completion() {
        let (url, hits) = spawn_stub_counting("ok");
        let client = FlowClient::new_url(url, Duration::from_secs(3));
        // ~4 KB: the stub reports len/2 = 2000 tokens — far over the
        // 1024-token window.
        let err = client
            .rewrite(&sample_req("medium", "normal", &"word ".repeat(800)))
            .expect_err("oversized input must be rejected");
        assert!(err.contains("too long"), "unexpected error: {err}");
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "a rejected input must not reach /v1/chat/completions"
        );
    }

    #[test]
    fn measured_tokens_admit_an_input_that_fits() {
        let (url, hits) = spawn_stub_counting("Refined.");
        let client = FlowClient::new_url(url, Duration::from_secs(3));
        // 200 bytes forces the /tokenize path; the stub reports 100 tokens,
        // leaving the default window room for prefix + answer.
        let out = client
            .rewrite(&sample_req("medium", "normal", &"word ".repeat(40)))
            .expect("an input that fits must be sent");
        assert_eq!(out, "Refined.");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn token_estimate_is_about_four_chars_per_token() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("Ship"), 1);
        // 20 characters -> 5 tokens. Real BPE on this sentence is ~6, well
        // inside the 512-token prompt reserve the budget already holds back.
        assert_eq!(estimate_tokens("Ship it on Thursday."), 5);
    }

    /// A longer input must not silently override the user's time budget.
    #[test]
    fn the_deadline_scales_with_input_size_and_is_capped() {
        let client = FlowClient::new_url("http://127.0.0.1:9".into(), Duration::from_secs(12));
        assert_eq!(client.request_deadline(0, 0), Duration::from_secs(12));
        let mid = client.request_deadline(150, 450);
        assert_eq!(mid, Duration::from_secs(12));
        assert_eq!(
            client.request_deadline(1_000_000, 1_000_000),
            Duration::from_secs(12),
            "the user's deadline must be honored"
        );
    }
}
