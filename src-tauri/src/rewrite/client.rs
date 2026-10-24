use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use serde_json::{json, Value};

use super::chunker::{self, MAX_CHUNK_WORDS};
use super::cleanup_prompt::{self, CleanupMode};
use super::prompt::build_messages;
use super::script::estimate_tokens;
use super::validate;

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
pub const DEFAULT_CONTEXT_SIZE: u32 = 4096;
/// Smallest `--ctx-size` the cleanup prompt is sized for.
pub const MIN_CONTEXT_SIZE: u32 = 4096;

/// Sampling for cleanup. The task copies its input, so the repeat penalty must
/// stay neutral: raising it makes the model avoid the words it should keep.
pub const SAMPLING_TEMPERATURE: f64 = 0.1;
pub const SAMPLING_TOP_K: u32 = 20;
pub const SAMPLING_TOP_P: f64 = 0.9;
pub const SAMPLING_MIN_P: f64 = 0.0;
pub const SAMPLING_REPEAT_PENALTY: f64 = 1.0;
/// Fixed so identical input gives identical output between runs.
pub const SAMPLING_SEED: u32 = 20_260_723;
/// `</transcript>` stops a model that tries to echo the closing tag.
pub const STOP_SEQUENCES: [&str; 5] = [
    "<|im_end|>",
    "<|endoftext|>",
    "</transcript>",
    "\n\nTranscript:",
    "\nUser:",
];

/// Inputs shorter than this many words skip the LLM: the deterministic
/// cleaner already handles them and a model has nothing to restructure.
pub const SKIP_LLM_BELOW_WORDS: usize = 4;

/// Reason codes surfaced in logs and the latency waterfall. Never text.
pub const REASON_SKIPPED_SHORT: &str = "skipped_short";
pub const REASON_LLM_ERROR: &str = "llm_error";
pub const REASON_CONTEXT_OVERFLOW: &str = "context_overflow";
pub const REASON_BREAKER_OPEN: &str = "breaker_open";
pub const REASON_TIMEOUT: &str = "timeout";
pub const REASON_TRUNCATED: &str = "truncated";

/// The chat-completions body for one cleanup request.
pub(crate) fn request_body(
    model_name: &str,
    temperature: f64,
    max_tokens: usize,
    messages: Vec<Value>,
) -> Value {
    json!({
        "model": model_name,
        "temperature": temperature,
        "top_k": SAMPLING_TOP_K,
        "top_p": SAMPLING_TOP_P,
        "min_p": SAMPLING_MIN_P,
        "repeat_penalty": SAMPLING_REPEAT_PENALTY,
        "seed": SAMPLING_SEED,
        "cache_prompt": true,
        "chat_template_kwargs": { "enable_thinking": false },
        "max_tokens": max_tokens,
        "stop": STOP_SEQUENCES,
        "messages": messages,
    })
}

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
    /// `SAMPLING_TEMPERATURE` by default; the opt-in quality benchmark compares low temperatures.
    pub temperature: f64,
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
            temperature: SAMPLING_TEMPERATURE,
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
            temperature: SAMPLING_TEMPERATURE,
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
            temperature: SAMPLING_TEMPERATURE,
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
        self.wait_for_completion(rx, timeout, "Local task timed out")
    }

    pub fn meeting_prompt_tokens(&self, messages: &[Value]) -> Result<usize, String> {
        let base = self.base_url.as_ref().ok_or("Local model unavailable")?;
        self.rendered_prompt_tokens(base, messages, Duration::from_secs(5))
            .ok_or_else(|| "The installed runtime must support /apply-template and /tokenize to summarize meetings safely".into())
    }

    pub fn complete_meeting(&self, messages: Vec<Value>) -> Result<String, String> {
        let tokens = self.meeting_prompt_tokens(&messages)?;
        if tokens + self.max_completion_tokens + 128 > self.context_size as usize {
            return Err("Meeting request exceeds effective model context".into());
        }
        self.complete(messages)
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
        // A cleanup rewrite is about as long as its input, so the budget scales
        // with the input: `ceil(1.3 x input tokens) + 32`. The slack covers
        // list markers and line breaks; the cap keeps a runaway generation from
        // consuming the window. A cut-off answer is rejected (`finish_reason:
        // length`), never injected.
        let max_tokens = chunker::completion_tokens(input_tokens);
        // Prompt prefix + input + a full-length answer must fit the window
        // whole. If they cannot, llama-server either refuses the request or
        // cuts the generation at the window edge — a half-rewrite is worse
        // than no rewrite, so this rejects early and lets the caller fall back
        // to the deterministic Stage-1 text.
        if max_tokens > self.max_completion_tokens
            || prompt_tokens.saturating_add(max_tokens).saturating_add(32)
                > self.context_size as usize
        {
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
        let body = request_body(model_name, self.temperature, max_tokens, messages);

        let (tx, rx) = std::sync::mpsc::channel();
        let generation = self.generation.clone();
        std::thread::spawn(move || {
            let result = run_chat_completion(url, body, timeout, &generation);
            let _ = tx.send(result);
        });

        self.wait_for_completion(rx, timeout, "flow rewrite timed out")
    }

    fn wait_for_completion(
        &self,
        rx: std::sync::mpsc::Receiver<Result<String, String>>,
        timeout: Duration,
        timeout_error: &str,
    ) -> Result<String, String> {
        let outcome = rx
            .recv_timeout(timeout)
            .map_err(|_| timeout_error.to_string())
            .and_then(|outcome| outcome);

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
            json!({ "messages": messages, "add_generation_prompt": true, "chat_template_kwargs": { "enable_thinking": false } }),
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
                .no_proxy()
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
    if payload["choices"][0]["finish_reason"].as_str() != Some("stop")
        || payload["truncated"].as_bool() == Some(true)
        || payload["choices"][0]["truncated"].as_bool() == Some(true)
    {
        return Err(
            "flow completion did not confirm a complete response; preserving the full transcript"
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

// Conservative sentence boundaries: a false negative keeps the full transcript
// through fallback, while a false positive can detach a name or instruction.
pub(crate) fn is_sentence_boundary(source: &str, index: usize, ch: char) -> bool {
    if matches!(ch, '。' | '！' | '？' | '।' | '॥') {
        return true;
    }
    let end = index + ch.len_utf8();
    if !matches!(ch, '.' | '!' | '?') || !source[end..].starts_with(char::is_whitespace) {
        return false;
    }
    if ch != '.' {
        return true;
    }
    let token = source[..index]
        .split_whitespace()
        .next_back()
        .unwrap_or_default()
        .trim_start_matches(['(', '[', '\'', '"']);
    let initial = token.chars().count() == 1 && token.chars().all(char::is_alphabetic);
    let dotted_abbreviation =
        token.contains('.') && token.chars().all(|c| c.is_ascii_alphabetic() || c == '.');
    let abbreviation = matches!(
        token.to_ascii_lowercase().as_str(),
        "dr" | "mr" | "mrs" | "ms" | "prof" | "st" | "sr" | "jr" | "etc" | "vs"
    );
    !initial && !dotted_abbreviation && !abbreviation
}

/// Reason code for a request-level failure. Codes only; never text.
fn failure_code(error: &str) -> &'static str {
    if error.contains("too long") || error.contains("exceeds the local model context") {
        REASON_CONTEXT_OVERFLOW
    } else if error.contains("circuit breaker") {
        REASON_BREAKER_OPEN
    } else if error.contains("timed out") || error.contains("deadline") {
        REASON_TIMEOUT
    } else if error.contains("token or context limit") {
        REASON_TRUNCATED
    } else {
        REASON_LLM_ERROR
    }
}

/// Tokens the chat template adds around the system and user messages.
const TEMPLATE_OVERHEAD_TOKENS: usize = 64;

/// One request: build, send, validate. `Err` carries the reason code and a
/// message safe to show the user (it never contains transcript text).
fn clean_chunk(
    client: &FlowClient,
    request: &RewriteRequest,
    system: &str,
    chunk: &str,
    timeout: Duration,
) -> Result<String, (&'static str, String)> {
    let mut bounded = client.clone();
    bounded.timeout = timeout;
    let mut request = request.clone();
    request.text = chunk.to_owned();
    let messages = cleanup_prompt::messages(system.to_owned(), chunk);
    let raw = bounded
        .rewrite_messages(&request, messages)
        .map_err(|error| (failure_code(&error), error))?;
    let cleaned = validate::validate(chunk, &raw).map_err(|rejection| {
        (
            rejection.code(),
            format!(
                "LLM output rejected ({}); deterministic text kept",
                rejection.code()
            ),
        )
    })?;
    Ok(restore_explicit_paragraphs(chunk, &cleaned))
}

fn ends_with_list_line(text: &str) -> bool {
    let last = text.lines().next_back().unwrap_or_default().trim_start();
    last.ends_with(':')
        || last.starts_with("- ")
        || last
            .split_once(". ")
            .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

///
/// `final_text` is the text to inject (or display). `used` indicates whether
/// the LLM actually rewrote the input. `error` is `Some` when the rewriter
/// was attempted but failed, including the validator rejecting an
/// otherwise-OK response. Callers are expected to surface `error` to the
/// user instead of silently degrading.
pub fn polish_or_fallback(
    client: &FlowClient,
    smart_text: &str,
    req: &RewriteRequest,
) -> PolishOutcome {
    polish_with_context_or_fallback(client, smart_text, req, None, &[])
}

/// The one cleanup pipeline: live dictation, Retry transcript and file import
/// all arrive here with the deterministic-cleaned text.
///
/// Long input is split into chunks (never inside a spoken list) that are
/// cleaned sequentially. Each chunk is validated on its own; a chunk that
/// fails keeps its deterministic text, so one bad chunk never costs the rest.
pub(crate) fn polish_with_context_or_fallback(
    client: &FlowClient,
    smart_text: &str,
    req: &RewriteRequest,
    formatting: Option<(&str, &str)>,
    inputs: &[(&str, String)],
) -> PolishOutcome {
    *client.generation.write() = GenerationMetrics::default();
    let level = req.cleanup_level.trim().to_ascii_lowercase();
    // `raw` is the one level that genuinely forbids Stage 2: the user asked for
    // their words back verbatim. `light` is not a Stage 2 switch;
    // `ResolvedIntent::run_llm` is the authority and the caller already checked it.
    if level == "raw" || smart_text.trim().is_empty() {
        return PolishOutcome::skipped(smart_text, None);
    }
    if smart_text.split_whitespace().count() < SKIP_LLM_BELOW_WORDS {
        return PolishOutcome::skipped(smart_text, Some(REASON_SKIPPED_SHORT));
    }

    let preference =
        formatting.and_then(|(mode, custom)| cleanup_prompt::format_preference(mode, custom));
    // Only minimal cleanup may skip already-punctuated short text. Casing and
    // terminal punctuation cannot prove grammar or repeated phrases are right.
    if level == "light"
        && inputs.is_empty()
        && formatting.is_none()
        && req.dictation_mode == "normal"
        && !req.style.eq_ignore_ascii_case("email")
        && super::safety::polish_would_add_nothing(smart_text)
    {
        log::debug!("Skipping polish: Stage 1 output is already clean");
        return PolishOutcome::skipped(smart_text, None);
    }

    let mut effective = req.clone();
    effective.text = smart_text.to_owned();
    let system = cleanup_prompt::system_prompt(
        CleanupMode::for_request(&effective),
        &req.vocabulary,
        preference.as_deref(),
        inputs,
    );
    let prompt_tokens = estimate_tokens(&system) + TEMPLATE_OVERHEAD_TOKENS;
    let max_input = chunker::max_chunk_input_tokens(client.context_size, prompt_tokens);
    let chunks = chunker::split(smart_text, MAX_CHUNK_WORDS, max_input);

    let started = Instant::now();
    let budget = client
        .timeout
        .saturating_mul(chunks.len() as u32)
        .min(FlowClient::DEADLINE_CEILING);
    let mut text = String::new();
    let mut applied = 0usize;
    let mut first_failure: Option<(&'static str, String)> = None;
    for chunk in &chunks {
        let remaining = budget.saturating_sub(started.elapsed());
        let result = if remaining.is_zero() {
            Err((REASON_TIMEOUT, "cleanup deadline expired".to_owned()))
        } else {
            clean_chunk(
                client,
                &effective,
                &system,
                &chunk.text,
                client.timeout.min(remaining),
            )
        };
        let piece = match result {
            Ok(cleaned) => {
                applied += 1;
                cleaned
            }
            Err(failure) => {
                first_failure.get_or_insert(failure);
                chunk.text.clone()
            }
        };
        if !text.is_empty() {
            let separator = if chunk.separator_before == " " && ends_with_list_line(&text) {
                "\n"
            } else {
                chunk.separator_before
            };
            text.push_str(separator);
        }
        text.push_str(&piece);
    }

    if let Some((code, _)) = &first_failure {
        log::info!(
            "Cleanup fell back to deterministic text: reason={code}, chunks_applied={applied}/{}",
            chunks.len()
        );
    }
    PolishOutcome {
        final_text: text,
        used: applied > 0,
        // A partial result is still a result; only a total failure is an error.
        error: first_failure
            .as_ref()
            .filter(|_| applied == 0)
            .map(|(_, message)| message.clone()),
        fallback_reason: first_failure.map(|(code, _)| code),
    }
}

/// Restore dictated paragraph boundaries only at unique, unchanged anchors.
/// Content still passes the complete safety gate after this presentation fix.
fn restore_explicit_paragraphs(source: &str, candidate: &str) -> String {
    if !source.contains("\n\n") || candidate.matches('\n').count() >= source.matches('\n').count() {
        return candidate.to_owned();
    }
    let words = regex::Regex::new(r"\S+").unwrap();
    let tokens = words.find_iter(candidate).collect::<Vec<_>>();
    let key = |word: &str| {
        word.trim_matches(|ch: char| !ch.is_alphanumeric())
            .to_lowercase()
    };
    let mut boundaries = Vec::new();
    for paragraph in source
        .split("\n\n")
        .skip(1)
        .filter(|part| !part.trim().is_empty())
    {
        let anchor = paragraph
            .split_whitespace()
            .take(4)
            .map(key)
            .collect::<Vec<_>>();
        if anchor.len() < 3 {
            return candidate.to_owned();
        }
        let matches = tokens
            .windows(anchor.len())
            .filter(|window| {
                window
                    .iter()
                    .map(|token| key(token.as_str()))
                    .eq(anchor.iter().cloned())
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return candidate.to_owned();
        }
        let at = matches[0][0].start();
        if boundaries.last().is_some_and(|previous| *previous >= at)
            || crate::formatting::literals::overlaps_literal(candidate, at..at + 1)
        {
            return candidate.to_owned();
        }
        boundaries.push(at);
    }
    let mut text = candidate.to_owned();
    for at in boundaries.into_iter().rev() {
        let start = text[..at].trim_end().len();
        text.replace_range(start..at, "\n\n");
    }
    text
}

#[derive(Debug, Clone)]
pub struct PolishOutcome {
    pub final_text: String,
    pub used: bool,
    pub error: Option<String>,
    /// Why the deterministic text was kept, wholly or in part, or why the LLM
    /// was skipped. A code only, never text; feeds the latency waterfall.
    pub fallback_reason: Option<&'static str>,
}

impl PolishOutcome {
    fn skipped(text: &str, reason: Option<&'static str>) -> Self {
        Self {
            final_text: text.to_owned(),
            used: false,
            error: None,
            fallback_reason: reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraph_restoration_preserves_content_and_refuses_ambiguous_or_literal_anchors() {
        let source = "The first topic is settled.\n\nOn a different topic we need boxes.";
        let flat = "The first topic is settled. On a different topic we need boxes.";
        assert_eq!(restore_explicit_paragraphs(source, flat), source);
        assert_eq!(
            restore_explicit_paragraphs(
                "One topic.\n\nI want to build it.",
                "I want to build it. I want to build it."
            ),
            "I want to build it. I want to build it."
        );
        assert_eq!(
            restore_explicit_paragraphs(
                "One topic.\n\nKeep this exact phrase.",
                "One topic. \"Keep this exact phrase.\""
            ),
            "One topic. \"Keep this exact phrase.\""
        );
    }

    use axum::{routing::post, Json, Router};
    use serde_json::Value;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn captured_context_is_not_skipped_for_a_short_clean_sentence() {
        let text = "Ship it on Thursday.";
        let outcome = polish_with_context_or_fallback(
            &FlowClient::new_missing(),
            text,
            &sample_req("light", "normal", text),
            None,
            &[("Clipboard", "Release requirements".into())],
        );
        assert!(outcome.error.is_some());
        assert_eq!(outcome.final_text, text);
    }
    use tokio::net::TcpListener;

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
                                        },
                                        "finish_reason": "stop"
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
                                            },
                                            "finish_reason": "stop"
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
        let smart = "hello world today my friend";
        let outcome = polish_or_fallback(&client, smart, &sample_req("medium", "normal", smart));
        assert_eq!(outcome.final_text, smart);
        assert!(!outcome.used);
        // A missing rewriter is an attempted rewrite that failed; callers
        // surface the error while keeping the original transcript.
        assert!(outcome.error.is_some());
    }

    /// Minimal cleanup may skip a finished sentence without requiring a model.
    #[test]
    fn a_clean_transcript_reports_no_error_even_with_no_runtime() {
        let client = FlowClient::new_missing();
        let smart = "Ship it on Thursday.";
        let outcome = polish_or_fallback(&client, smart, &sample_req("light", "normal", smart));
        assert_eq!(outcome.final_text, smart);
        assert!(!outcome.used);
        assert!(
            outcome.error.is_none(),
            "skipping unnecessary work must not surface an error: {:?}",
            outcome.error
        );
    }

    #[test]
    fn writing_and_developer_tasks_do_not_skip_their_requested_edit() {
        for (level, mode) in [
            ("high", "notes"),
            ("medium", "developer_prompt"),
            ("medium", "normal"),
        ] {
            let source = "Please fix the login.";
            let result = polish_or_fallback(
                &FlowClient::new_missing(),
                source,
                &sample_req(level, mode, source),
            );
            assert_eq!(result.final_text, source);
            assert!(!result.used);
            assert!(
                result.error.is_some(),
                "requested task must attempt its edit, then report missing runtime"
            );
        }
    }

    #[test]
    fn raw_and_light_skip_llm_but_coding_does_not() {
        let client = FlowClient {
            generation: Default::default(),
            base_url: None,
            timeout: Duration::from_millis(50),
            breaker: CircuitBreaker::default(),
            max_completion_tokens: FlowClient::completion_budget(1024),
            context_size: 1024,
            temperature: 0.0,
        };
        let smart = "Keep this exact sentence.";
        let raw = polish_or_fallback(&client, smart, &sample_req("raw", "normal", smart));
        assert!(!raw.used && raw.error.is_none() && raw.fallback_reason.is_none());
        let light = polish_or_fallback(&client, smart, &sample_req("light", "normal", smart));
        assert!(!light.used && light.error.is_none());
        // Coding reaches the (here missing) model: an attempted, failed cleanup.
        let coding = polish_or_fallback(&client, smart, &sample_req("medium", "coding", smart));
        assert!(!coding.used);
        assert!(coding.error.is_some());
        assert_eq!(coding.final_text, smart);
    }

    #[test]
    fn completion_metrics_come_from_the_server_and_reset_for_each_polish() {
        let client = FlowClient::new_url(
            spawn_stub("Hello world today my friend."),
            Duration::from_secs(3),
        );
        let request = sample_req("medium", "normal", "hello world today my friend");
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
        let orig = "hello world today my friend";
        let url = spawn_stub("Hello world today my friend.");
        let client = FlowClient::new_url(url, Duration::from_secs(3));
        let rewritten = client
            .rewrite(&sample_req("medium", "normal", orig))
            .expect("rewrite");
        assert_eq!(rewritten, "Hello world today my friend.");
        let outcome = polish_or_fallback(&client, orig, &sample_req("medium", "normal", orig));
        assert_eq!(outcome.final_text, "Hello world today my friend.");
        assert!(outcome.used);
        assert!(outcome.error.is_none());
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
        for reason in [
            Value::Null,
            json!("tool_calls"),
            json!("content_filter"),
            json!("cancelled"),
        ] {
            let mut incomplete = finished.clone();
            incomplete["choices"][0]["finish_reason"] = reason;
            assert!(extract_content(&incomplete)
                .unwrap_err()
                .contains("complete response"));
        }
        let mut truncated = finished.clone();
        truncated["truncated"] = json!(true);
        assert!(extract_content(&truncated).is_err());
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
    fn stalled_generation_trips_the_shared_breaker_and_a_late_reply_cannot_reset_it() {
        let client = FlowClient::new_url("http://127.0.0.1:9".into(), Duration::from_secs(1));
        let mut workers = Vec::new();
        for _ in 0..3 {
            let (tx, rx) = std::sync::mpsc::channel();
            workers.push(tx);
            assert_eq!(
                client.wait_for_completion(rx, Duration::from_millis(5), "timed out"),
                Err("timed out".into())
            );
        }
        assert_eq!(client.breaker.failure_count(), 3);
        assert!(client.breaker.is_open());
        for worker in workers {
            assert!(worker.send(Ok("late rewrite".into())).is_err());
        }
        assert!(client.breaker.is_open());
        let started = Instant::now();
        assert!(client
            .rewrite(&sample_req("medium", "normal", "hello today"))
            .unwrap_err()
            .contains("circuit breaker"));
        assert!(started.elapsed() < Duration::from_millis(100));
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
        let client = FlowClient::new_url("http://127.0.0.1:9".into(), Duration::from_millis(500))
            .with_context_size(1024);
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
    type Captured = Arc<std::sync::Mutex<Vec<Value>>>;

    /// A stub that records every completion request and answers with
    /// `reply(transcript)`, where `transcript` is the user turn's text.
    fn spawn_transform_stub(reply: fn(&str) -> String) -> (String, Captured) {
        let captured: Captured = Default::default();
        let sink = Arc::clone(&captured);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("stub runtime");
            rt.block_on(async move {
                let app = Router::new().route(
                    "/v1/chat/completions",
                    post(move |Json(req): Json<Value>| {
                        let sink = Arc::clone(&sink);
                        async move {
                            let user = req["messages"][1]["content"].as_str().unwrap_or_default();
                            let transcript = user
                                .strip_prefix("<transcript>")
                                .and_then(|t| t.strip_suffix("</transcript>"))
                                .unwrap_or_default()
                                .to_owned();
                            sink.lock().unwrap().push(req);
                            Json(json!({"choices": [{
                                "message": {"role": "assistant", "content": reply(&transcript)},
                                "finish_reason": "stop"
                            }]}))
                        }
                    }),
                );
                let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
                let _ = tx.send(format!("http://{}", listener.local_addr().unwrap()));
                axum::serve(listener, app).await.ok();
            });
        });
        (rx.recv().expect("stub addr"), captured)
    }

    fn echo(transcript: &str) -> String {
        transcript.to_owned()
    }

    #[test]
    fn request_body_uses_the_named_sampling_constants() {
        let body = request_body("Qwen3.5-0.8B", SAMPLING_TEMPERATURE, 99, vec![]);
        assert_eq!(body["temperature"], 0.1);
        assert_eq!(body["top_k"], 20);
        assert_eq!(body["top_p"], 0.9);
        assert_eq!(body["min_p"], 0.0);
        assert_eq!(body["repeat_penalty"], 1.0);
        assert_eq!(body["seed"], SAMPLING_SEED);
        assert_eq!(body["cache_prompt"], true);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(body["max_tokens"], 99);
        assert!(body["stop"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "</transcript>"));
        assert_eq!(FlowClient::new_missing().temperature, SAMPLING_TEMPERATURE);
    }

    #[test]
    fn wire_request_has_the_agreed_layout_dictionary_and_mode() {
        let (url, captured) = spawn_transform_stub(echo);
        let client = FlowClient::new_url(url, Duration::from_secs(3));
        let text = "run cargo build in the src tauri folder";
        let mut request = sample_req("medium", "coding", text);
        request.vocabulary = vec!["Tauri".into()];
        let outcome = polish_with_context_or_fallback(&client, text, &request, None, &[]);
        assert!(outcome.used, "{:?}", outcome.error);
        let sent = captured.lock().unwrap();
        assert_eq!(sent.len(), 1);
        let messages = sent[0]["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        let system = messages[0]["content"].as_str().unwrap();
        assert!(system.starts_with(cleanup_prompt::static_prompt()));
        assert!(system.contains("\nDICTIONARY: Tauri"));
        assert!(system.contains("\nMODE: keep identifiers, commands, paths exactly as spoken"));
        assert_eq!(
            messages[1]["content"],
            format!("<transcript>{text}</transcript>")
        );
        assert_eq!(sent[0]["seed"], SAMPLING_SEED);
        assert_eq!(sent[0]["top_k"], SAMPLING_TOP_K);
        let expected = chunker::completion_tokens(estimate_tokens(text));
        assert_eq!(sent[0]["max_tokens"], expected);
    }

    #[test]
    fn the_static_prefix_is_byte_identical_between_calls() {
        let (url, captured) = spawn_transform_stub(echo);
        let client = FlowClient::new_url(url, Duration::from_secs(3));
        for (mode, text) in [
            ("normal", "send the report to the whole team today"),
            ("email", "please confirm the meeting for tomorrow morning"),
        ] {
            let request = sample_req("medium", mode, text);
            assert!(polish_with_context_or_fallback(&client, text, &request, None, &[]).used);
        }
        let sent = captured.lock().unwrap();
        let prefix = cleanup_prompt::static_prompt();
        for request in sent.iter() {
            let system = request["messages"][0]["content"].as_str().unwrap();
            assert_eq!(&system.as_bytes()[..prefix.len()], prefix.as_bytes());
        }
        assert_ne!(
            sent[0]["messages"][0]["content"],
            sent[1]["messages"][0]["content"]
        );
    }

    #[test]
    fn short_input_never_reaches_the_model() {
        let (url, captured) = spawn_transform_stub(echo);
        let client = FlowClient::new_url(url, Duration::from_secs(3));
        for text in ["Okay", "thank you", "see you soon"] {
            let outcome = polish_or_fallback(&client, text, &sample_req("medium", "normal", text));
            assert_eq!(outcome.final_text, text);
            assert!(!outcome.used);
            assert!(outcome.error.is_none());
            assert_eq!(outcome.fallback_reason, Some(REASON_SKIPPED_SHORT));
        }
        assert!(captured.lock().unwrap().is_empty());
        let four = "see you soon friend";
        assert!(polish_or_fallback(&client, four, &sample_req("medium", "normal", four)).used);
    }

    #[test]
    fn a_rejected_answer_falls_back_with_only_a_reason_code() {
        let (url, _) =
            spawn_transform_stub(|_| "The capital of France is Paris, of course.".into());
        let client = FlowClient::new_url(url, Duration::from_secs(3));
        let text = "what is the capital of france";
        let outcome = polish_or_fallback(&client, text, &sample_req("medium", "normal", text));
        assert_eq!(outcome.final_text, text);
        assert!(!outcome.used);
        assert_eq!(outcome.fallback_reason, Some("hallucination"));
        assert!(!outcome.error.unwrap().contains("Paris"));
    }

    #[test]
    fn an_empty_answer_falls_back() {
        let (url, _) = spawn_transform_stub(|_| "  ".into());
        let client = FlowClient::new_url(url, Duration::from_secs(3));
        let text = "hello world today my friend";
        let outcome = polish_or_fallback(&client, text, &sample_req("medium", "normal", text));
        assert_eq!(outcome.final_text, text);
        assert_eq!(outcome.fallback_reason, Some("empty_output"));
    }

    #[test]
    fn thinking_tags_and_preambles_are_stripped_before_validation() {
        let (url, _) = spawn_transform_stub(|_| {
            "<think>plan the edit</think>Here is the cleaned text:\nHello world today my friend."
                .into()
        });
        let client = FlowClient::new_url(url, Duration::from_secs(3));
        let text = "hello world today my friend";
        let outcome = polish_or_fallback(&client, text, &sample_req("medium", "normal", text));
        assert_eq!(outcome.final_text, "Hello world today my friend.");
        assert!(outcome.used);
        assert_eq!(outcome.fallback_reason, None);
    }

    fn long_dictation(words: usize) -> String {
        (0..words / 10)
            .map(|i| format!("Item {i} covers the quarterly plan and the budget review today."))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn long_dictation_is_cleaned_in_chunks_and_nothing_is_dropped() {
        let (url, captured) = spawn_transform_stub(echo);
        let client = FlowClient::new_url(url, Duration::from_secs(10));
        let text = long_dictation(700);
        let outcome = polish_or_fallback(&client, &text, &sample_req("medium", "normal", &text));
        assert!(outcome.used, "{:?}", outcome.error);
        assert_eq!(outcome.final_text, text);
        let sent = captured.lock().unwrap();
        assert!(sent.len() >= 3, "{} requests", sent.len());
        for request in sent.iter() {
            let user = request["messages"][1]["content"].as_str().unwrap();
            assert!(user.split_whitespace().count() <= MAX_CHUNK_WORDS + 2);
        }
    }

    #[test]
    fn one_failing_chunk_keeps_its_text_and_the_rest_are_cleaned() {
        let (url, _) = spawn_transform_stub(|transcript| {
            if transcript.contains("Item 0 ") {
                String::new()
            } else {
                transcript.replace("quarterly", "Quarterly")
            }
        });
        let client = FlowClient::new_url(url, Duration::from_secs(10));
        let text = long_dictation(700);
        let outcome = polish_or_fallback(&client, &text, &sample_req("medium", "normal", &text));
        assert!(outcome.used);
        assert!(outcome.error.is_none(), "a partial result is not an error");
        assert_eq!(outcome.fallback_reason, Some("empty_output"));
        assert!(outcome
            .final_text
            .starts_with("Item 0 covers the quarterly plan"));
        assert!(outcome.final_text.contains("Quarterly"));
        assert_eq!(
            outcome.final_text.split_whitespace().count(),
            text.split_whitespace().count()
        );
    }

    #[test]
    fn chunks_are_joined_by_a_blank_line_only_where_the_speaker_paused_for_one() {
        let (url, _) = spawn_transform_stub(echo);
        let client = FlowClient::new_url(url, Duration::from_secs(10));
        let text = format!("{}\n\n{}", long_dictation(320), long_dictation(320));
        let outcome = polish_or_fallback(&client, &text, &sample_req("medium", "normal", &text));
        assert!(outcome.used, "{:?}", outcome.error);
        assert!(outcome.final_text.contains("\n\n"));
        assert_eq!(outcome.final_text, text);
    }

    #[test]
    fn a_missing_server_reports_a_reason_and_keeps_every_chunk() {
        let client = FlowClient::new_url("http://127.0.0.1:9".into(), Duration::from_millis(300));
        let text = long_dictation(700);
        let outcome = polish_or_fallback(&client, &text, &sample_req("medium", "normal", &text));
        assert_eq!(outcome.final_text, text);
        assert!(!outcome.used);
        assert!(outcome.error.is_some());
        assert!(outcome.fallback_reason.is_some());
    }
}
