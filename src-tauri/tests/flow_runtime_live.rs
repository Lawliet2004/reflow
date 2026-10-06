//! Live verification of Stage 2 (LLM polish) against the installed
//! `llama-server` runtime.
//!
//! This exists because Stage 2 was silently dead in production while every
//! unit test passed: `FlowRuntime::ensure` refused to launch a perfectly good
//! `llama-server` whenever the flavor marker file was absent, so
//! `FlowClient::base_url` stayed `None` and every dictation fell back to the
//! unpolished text. `rewriter_used` was `0` for all 44 rows of a real user's
//! history. Nothing in the unit suite could catch that, because the unit tests
//! all inject either a stub HTTP server or `FlowClient::new_missing()`.
//!
//! The test skips when no runtime is installed, so CI without the ~700 MB GGUF
//! stays green.

use std::time::{Duration, Instant};

use reflow_lib::rewrite::{
    flow_gguf_path, llama_server_bin, polish_or_fallback, FlowRuntime, RewriteRequest,
};

const FLOW_MODEL: &str = "qwen3.5-0.8b";

/// Serializes the tests in this file.
///
/// Each one starts a real `llama-server` with GPU offload. Run in parallel on a
/// 4 GB card they exhaust VRAM and one of the servers dies mid-request, which
/// looks exactly like a product failure and is not one.
static LIVE_RUNTIME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serialized() -> std::sync::MutexGuard<'static, ()> {
    LIVE_RUNTIME_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn runtime_installed() -> bool {
    llama_server_bin().is_file() && flow_gguf_path(FLOW_MODEL).is_file()
}

#[test]
fn installed_runtime_summarizes_a_meeting_with_its_task_context() {
    let _serial = serialized();
    if !runtime_installed() {
        eprintln!("skipping: no llama-server runtime installed");
        return;
    }
    let runtime = FlowRuntime::default();
    runtime
        .ensure(FLOW_MODEL, "vulkan", None, 0, 4096)
        .expect("summary runtime should start");
    let client = runtime.client.read().clone();
    let result = reflow_lib::rewrite::tasks::summarize_transcript(&client,
        "At today's meeting, the team decided to release on Friday. Mina owns final testing. If the tests fail, the release waits. The next review is Thursday.");
    runtime.shutdown();
    let summary = result.expect("the installed local model should answer the summary task");
    assert!(!summary.trim().is_empty());
    assert!(summary.chars().count() < 2400, "summary must stay concise");
}

fn request(text: &str) -> RewriteRequest {
    RewriteRequest {
        text: text.into(),
        cleanup_level: "high".into(),
        style: "neutral".into(),
        dictation_mode: "normal".into(),
        vocabulary: Vec::new(),
        app_process: "notepad.exe".into(),
        model_id: FLOW_MODEL.into(),
    }
}

#[test]
fn installed_runtime_cleans_writing_tasks_without_losing_details() {
    let _serial = serialized();
    if !runtime_installed() {
        eprintln!("skipping: no llama-server runtime installed");
        return;
    }
    let runtime = FlowRuntime::default();
    runtime
        .ensure(FLOW_MODEL, "vulkan", None, 0, 4096)
        .expect("cleanup runtime should start");
    runtime.warm_prompt_cache(FLOW_MODEL);
    let client = runtime.client.read().clone();
    let natural = "I was thinking we could we could ship on Thursday.";
    let mut req = request(natural);
    req.cleanup_level = "medium".into();
    req.style = "faithful".into();
    let outcome = polish_or_fallback(&client, natural, &req);
    eprintln!(
        "Natural cleanup -> {:?} (used={}, error={:?})",
        outcome.final_text, outcome.used, outcome.error
    );
    if !outcome.used {
        eprintln!("Rejected natural candidate: {:?}", client.rewrite(&req));
    }
    assert!(
        outcome.used,
        "natural cleanup should apply: {:?}",
        outcome.error
    );
    assert!(!outcome
        .final_text
        .to_lowercase()
        .contains("we could we could"));
    assert!(outcome.final_text.contains("Thursday"));

    let developer = "fix the login um don't change getUserById in src/api.ts and keep --dry-run";
    req.text = developer.into();
    req.dictation_mode = "developer_prompt".into();
    let outcome = polish_or_fallback(&client, developer, &req);
    eprintln!(
        "Developer cleanup -> {:?} (used={}, error={:?})",
        outcome.final_text, outcome.used, outcome.error
    );
    // A rejected edit must return the complete original, never partial text.
    assert!(outcome.used || outcome.final_text == developer);
    for term in ["getUserById", "src/api.ts", "--dry-run"] {
        assert!(
            outcome.final_text.contains(term),
            "missing {term}: {}",
            outcome.final_text
        );
    }
    let lower = outcome.final_text.to_lowercase();
    assert!(lower.contains("don't change") || lower.contains("do not change"));
    runtime.shutdown();
}

/// `ensure()` must produce a usable client for the runtime that is actually on
/// disk. Before the fix this returned `RuntimeFlavorMismatch` on any install
/// whose `llama-server.kind` marker was missing — which is the state an
/// interrupted install leaves behind, since the marker is written after the
/// binary.
#[test]
fn flow_runtime_starts_and_serves_a_rewrite() {
    let _serial = serialized();
    if !runtime_installed() {
        eprintln!(
            "skipping: no llama-server runtime installed at {}",
            llama_server_bin().display()
        );
        return;
    }

    let runtime = FlowRuntime::default();

    // "vulkan" is the shipped GPU flavor and the value this project's settings
    // default to; it is also the exact backend that used to be refused.
    let ensured = runtime.ensure(FLOW_MODEL, "vulkan", None, 0, 1024);
    assert!(
        ensured.is_ok(),
        "ensure() must start the installed runtime, got: {:?} (last_error: {:?})",
        ensured,
        runtime.last_error()
    );

    let client = runtime.client.read().clone();
    assert!(
        client.base_url.is_some(),
        "a started runtime must hand out a client with a base URL; \
         a None here is exactly the silent failure that disabled polishing"
    );
    assert!(runtime.status_ready(), "runtime should report ready");

    // The raw call: proves the server is reachable and answering.
    let raw = client.rewrite(&request("i think we should ship this on friday"));
    assert!(
        raw.is_ok(),
        "the running llama-server must answer a chat completion, got: {raw:?}\nstderr: {}",
        runtime.stderr_log()
    );
    let completion = raw.expect("checked above");
    assert!(
        !completion.trim().is_empty(),
        "completion must not be empty"
    );
    eprintln!("llama-server rewrote -> {completion:?}");

    runtime.shutdown();
}

/// The whole point of the fix: a real dictation must come out of
/// `polish_or_fallback` having actually been through the LLM and had the result
/// applied. Two separate bugs made this fail in production — the runtime never
/// launched, and then, once it did, the system prompt fed the model a quoted
/// specimen answer that it emitted instead of cleaning the transcript, so the
/// safety gate rejected everything. Both show up here as `used == false`.
#[test]
fn polish_is_actually_applied_to_real_dictation() {
    let _serial = serialized();
    if !runtime_installed() {
        eprintln!("skipping: no llama-server runtime installed");
        return;
    }

    let runtime = FlowRuntime::default();
    runtime
        .ensure(FLOW_MODEL, "vulkan", None, 0, 1024)
        .expect("runtime should start");
    let client = runtime.client.read().clone();

    // Word-choice preservation deliberately rejects some grammatical paraphrases.
    // A rejection must preserve every original byte and must come from the
    // safety gate, never a timeout, unavailable runtime or truncated response.
    let original = "i need the key, for opening the door";
    let outcome = polish_or_fallback(&client, original, &request(original));
    if outcome.used {
        assert!(outcome.error.is_none());
        assert_eq!(outcome.final_text, "I need the key for opening the door.");
    } else {
        assert_eq!(outcome.final_text, original);
        assert_eq!(
            outcome.error.as_deref(),
            Some("LLM rewrite rejected by safety gate")
        );
    }

    // Each safe ordinary dictation must actually use the reachable local model.
    // This still fails if startup, the prompt or the safety gate rejects all work.
    for (smart, expected) in [
        (
            "can you review the code for me and start, like doing the task of code review",
            "Can you review the code for me and start, like doing the task of code review?",
        ),
        (
            "i think we should ship this on friday",
            "I think we should ship this on Friday.",
        ),
    ] {
        let outcome = polish_or_fallback(&client, smart, &request(smart));
        eprintln!(
            "{smart:?} -> {:?} (used={}, error={:?})",
            outcome.final_text, outcome.used, outcome.error
        );
        assert!(
            outcome.used,
            "ordinary safe dictation must apply: {:?}",
            outcome.error
        );
        assert!(outcome.error.is_none());
        assert_eq!(outcome.final_text, expected);
    }

    runtime.shutdown();
}

/// Refusing a rewrite is a feature, and it has to keep working.
///
/// This input is real ASR output whose negation the model reliably drops:
/// "dont you know about chatgpt what chatgpt is" comes back as
/// "Do you know about ChatGPT, what it is?" — an incredulous question turned
/// into a plain one, which is a meaning change. Token distance alone waves it
/// through, so the negation rule in the safety gate is the only thing standing
/// between the user and a rewrite that says something they did not.
#[test]
fn a_meaning_changing_rewrite_is_refused_and_the_original_kept() {
    let _serial = serialized();
    if !runtime_installed() {
        eprintln!("skipping: no llama-server runtime installed");
        return;
    }

    let runtime = FlowRuntime::default();
    runtime
        .ensure(FLOW_MODEL, "vulkan", None, 0, 1024)
        .expect("runtime should start");
    let client = runtime.client.read().clone();

    let smart = "dont you know about chatgpt what chatgpt is";
    let candidate = client.rewrite(&request(smart));
    eprintln!("raw candidate for the negation case: {candidate:?}");

    let outcome = polish_or_fallback(&client, smart, &request(smart));
    if outcome.used {
        // The model kept the negation, which is a legitimate good outcome.
        eprintln!("model preserved the negation: {:?}", outcome.final_text);
        assert!(
            outcome.final_text.to_lowercase().contains("n't")
                || outcome.final_text.to_lowercase().contains("not"),
            "an accepted rewrite of a negated sentence must still negate: {:?}",
            outcome.final_text
        );
    } else {
        // The expected path: rejected, original preserved, reason reported.
        assert_eq!(
            outcome.final_text, smart,
            "a refused rewrite must leave the transcript untouched"
        );
        assert!(
            outcome.error.is_some(),
            "a refusal must be reported, not silent"
        );
    }

    runtime.shutdown();
}

/// A warm runtime must not relaunch on every dictation; the second `ensure()`
/// has to be a cache hit. A relaunch per dictation would add seconds of
/// latency to the stop path.
#[test]
fn a_warm_runtime_is_reused() {
    let _serial = serialized();
    if !runtime_installed() {
        eprintln!("skipping: no llama-server runtime installed");
        return;
    }
    let runtime = FlowRuntime::default();
    runtime
        .ensure(FLOW_MODEL, "vulkan", None, 0, 1024)
        .expect("first ensure should start the runtime");
    let first_url = runtime.client.read().base_url.clone();

    let started = std::time::Instant::now();
    runtime
        .ensure(FLOW_MODEL, "vulkan", None, 0, 1024)
        .expect("second ensure should be a cache hit");
    let elapsed = started.elapsed();

    assert_eq!(
        first_url,
        runtime.client.read().base_url.clone(),
        "a cache hit must keep the same server"
    );
    assert!(
        elapsed < Duration::from_millis(500),
        "second ensure took {elapsed:?}; it should not relaunch llama-server"
    );

    runtime.shutdown();
}

/// Warming the shared prompt prefix must make a rewrite fast on CPU.
///
/// CPU is not a fallback here, it is the recommended configuration on a 4 GB
/// card: measured on an RTX 2050 with the 0.6B ASR model resident, running the
/// polish model on CPU costs 256 ms more per rewrite but frees 575 MiB and lets
/// ASR run 18% faster for lack of GPU contention.
///
/// The hazard this pins down: prompt evaluation on CPU measures ~28 tokens per
/// second, so the shared system + few-shot prefix costs seconds every time it
/// is not cached. An over-long prompt combined with a cold cache exceeded the
/// refinement deadline outright — the rewrite was abandoned and the dictation
/// went out unpolished, with the cost paid and no benefit delivered.
#[test]
fn warming_the_prompt_cache_makes_cpu_rewrites_fast() {
    let _serial = serialized();
    if !runtime_installed() {
        eprintln!("skipping: no llama-server runtime installed");
        return;
    }

    let runtime = FlowRuntime::default();
    runtime
        .ensure(FLOW_MODEL, "cpu", None, 0, 1024)
        .expect("runtime should start on CPU");
    let client = runtime.client.read().clone();

    let cold_started = Instant::now();
    runtime.warm_prompt_cache(FLOW_MODEL);
    let warmup = cold_started.elapsed();

    // Now a real rewrite, which should reuse the cached prefix.
    let text = "i think we should uh ship this on friday i mean thursday";
    let started = Instant::now();
    let outcome = polish_or_fallback(&client, text, &request(text));
    let warm = started.elapsed();

    eprintln!(
        "CPU prompt-cache warmup {} ms, subsequent rewrite {} ms -> {:?} (used={})",
        warmup.as_millis(),
        warm.as_millis(),
        outcome.final_text,
        outcome.used
    );

    assert!(
        outcome.error.is_none() || !outcome.error.as_deref().unwrap_or("").contains("timed out"),
        "a warmed CPU rewrite must not exceed the refinement deadline: {:?}",
        outcome.error
    );
    assert!(
        warm < Duration::from_secs(4),
        "a warmed CPU rewrite took {warm:?}; the prefix cache is not being reused"
    );

    runtime.shutdown();
}
