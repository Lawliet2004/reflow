//! Opt-in paired measurements of the production cleanup pipeline; never installs weights.
use reflow_lib::{
    rewrite::{flow_gguf_path, llama_server_bin, FlowClient, FlowRuntime},
    session::postprocess_transcript,
    settings::AppSettings,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    id: String,
    category: String,
    text: String,
    required: Vec<Vec<String>>,
    #[serde(default)]
    forbidden: Vec<String>,
    #[serde(default)]
    layout: String,
}

fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!("fixtures/refinement_cases.json")).unwrap()
}

fn server_peak_rss_bytes(pid: Option<u32>) -> Option<u64> {
    let pid = pid?;
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        status
            .lines()
            .find_map(|line| {
                line.strip_prefix("VmHWM:")?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
            .map(|kb| kb * 1024)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut system = sysinfo::System::new();
        let pid = sysinfo::Pid::from_u32(pid);
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        system.process(pid).map(sysinfo::Process::memory)
    }
}

fn device_vram_used_mib() -> Option<u64> {
    let output = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.used", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}

#[test]
fn benchmark_corpus_covers_fifty_distinct_spoken_inputs() {
    let cases = cases();
    assert!(cases.len() >= 50);
    let ids = cases
        .iter()
        .map(|case| &case.id)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), cases.len());
    assert!(cases.iter().any(|case| case.category == "long"));
    assert!(cases.iter().any(|case| case.category == "literal"));
    assert!(cases.iter().any(|case| case.category == "multilingual"));
}

#[test]
#[ignore = "prints raw installed-model candidates separately from safety decisions; no downloads"]
fn diagnose_model_or_validator_rejections() {
    use reflow_lib::rewrite::{
        prompt::{build_messages_with_format, formatting_instruction},
        RewriteRequest,
    };
    let runtime = FlowRuntime::default();
    runtime
        .ensure("qwen3.5-0.8b", "vulkan", None, 0, 4096)
        .unwrap();
    let client = runtime.client.read().clone();
    let settings = AppSettings {
        intelligence_tier: "raw_verbatim".into(),
        cleanup_level: "high".into(),
        auto_style_from_app: false,
        ..AppSettings::default()
    };
    for case in cases().into_iter().take(14) {
        let prepared = postprocess_transcript(&case.text, &settings, "preview", &client).smart;
        let req = RewriteRequest {
            text: prepared.clone(),
            cleanup_level: "high".into(),
            style: "faithful".into(),
            dictation_mode: "normal".into(),
            vocabulary: vec![],
            app_process: "preview".into(),
            model_id: "qwen3.5-0.8b".into(),
        };
        let messages =
            build_messages_with_format(&req, formatting_instruction("auto", "").as_deref());
        let candidate = client.rewrite_messages(&req, messages);
        eprintln!("{} source={prepared:?} candidate={candidate:?}", case.id);
    }
    runtime.shutdown();
}

#[test]
#[ignore = "runs the installed Qwen3.5-0.8B against 50 production-pipeline cases; no downloads"]
fn installed_model_quality_and_context_measurements() {
    let output = std::env::var("REFLOW_REFINEMENT_BENCH_OUTPUT").expect("set output JSON path");
    assert!(
        flow_gguf_path("qwen3.5-0.8b").is_file(),
        "installed model required"
    );
    assert!(llama_server_bin().is_file(), "installed runtime required");
    let contexts = std::env::var("REFLOW_REFINEMENT_CONTEXTS").unwrap_or_else(|_| "4096".into());
    let backend = std::env::var("REFLOW_REFINEMENT_BACKEND").unwrap_or_else(|_| "vulkan".into());
    let reuse = std::env::var("REFLOW_REFINEMENT_REUSE_SERVER")
        .ok()
        .inspect(|base| {
            let url = reqwest::Url::parse(base).unwrap();
            assert_eq!(url.scheme(), "http");
            assert!(matches!(url.host_str(), Some("127.0.0.1" | "[::1]")));
            assert_eq!(url.path(), "/");
            assert!(
                url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
            );
        });
    let runtime = FlowRuntime::default();
    let mut results = Vec::new();
    for context in contexts
        .split(',')
        .map(|value| value.parse::<u32>().unwrap())
    {
        let started = std::time::Instant::now();
        if reuse.is_none() {
            runtime
                .ensure("qwen3.5-0.8b", &backend, None, 0, context)
                .unwrap();
        }
        let startup_ms = started.elapsed().as_millis();
        let mut client = if let Some(base) = &reuse {
            FlowClient::new_url(base.clone(), std::time::Duration::from_secs(12))
                .with_context_size(context)
        } else {
            runtime.warm_prompt_cache("qwen3.5-0.8b");
            runtime.client.read().clone()
        };
        let properties: serde_json::Value = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap()
            .get(format!("{}/props", client.base_url.as_ref().unwrap()))
            .send()
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .unwrap();
        let server_context = properties
            .pointer("/default_generation_settings/n_ctx")
            .and_then(serde_json::Value::as_u64)
            .expect("active per-slot context must be reported");
        assert_eq!(
            u64::from(client.context_size),
            server_context.min(u64::from(context))
        );
        if reuse.is_some() {
            assert_eq!(
                server_context,
                u64::from(context),
                "reuse must measure the existing server's actual context"
            );
        }
        if let Ok(temperature) = std::env::var("REFLOW_REFINEMENT_TEMPERATURE") {
            client.temperature = temperature.parse().unwrap();
            assert!((0.0..=0.2).contains(&client.temperature));
        }
        let settings = AppSettings {
            flow_model: "qwen3.5-0.8b".into(),
            intelligence_tier: "smart_flow".into(),
            cleanup_level: "high".into(),
            processing_mode: "flow".into(),
            transcript_format: "auto".into(),
            auto_style_from_app: false,
            ..AppSettings::default()
        };
        for case in cases() {
            let started = std::time::Instant::now();
            let out = postprocess_transcript(&case.text, &settings, "preview", &client);
            let total_ms = started.elapsed().as_millis();
            let lower = out.final_text.to_lowercase();
            let missing = case
                .required
                .iter()
                .filter(|alternatives| {
                    !alternatives
                        .iter()
                        .any(|word| lower.contains(&word.to_lowercase()))
                })
                .collect::<Vec<_>>();
            let forbidden = case
                .forbidden
                .iter()
                .filter(|word| lower.contains(&word.to_lowercase()))
                .collect::<Vec<_>>();
            let list_lines = out
                .final_text
                .lines()
                .filter(|line| {
                    line.starts_with(['•', '-', '*'])
                        || line.chars().next().is_some_and(|ch| ch.is_ascii_digit())
                            && line.contains(". ")
                })
                .count();
            let breaks_preserved =
                out.final_text.matches('\n').count() >= case.text.trim().matches('\n').count();
            let layout_ok = breaks_preserved
                && match case.layout.as_str() {
                    "prose" => list_lines == 0,
                    "list" => list_lines >= 3,
                    "steps" => out.final_text.contains("1. ") && out.final_text.contains("4. "),
                    _ => true,
                };
            eprintln!(
                "context={context} {} used={} contract={} reason={:?}",
                case.id,
                out.rewriter_used,
                missing.is_empty() && forbidden.is_empty() && layout_ok,
                out.rewriter_error
            );
            let server_pid = if reuse.is_some() {
                std::env::var("REFLOW_REFINEMENT_SERVER_PID")
                    .ok()
                    .and_then(|pid| pid.parse().ok())
            } else {
                runtime.process_id()
            };
            results.push(serde_json::json!({"context":context,"reported_context":client.context_size,"server_context":server_context,"startup_ms":startup_ms,"reused_server":reuse.is_some(),"temperature":client.temperature,"backend":runtime.active_mode().map(|mode| format!("{mode:?}")),"server_peak_rss_bytes":server_peak_rss_bytes(server_pid),"device_vram_used_mib":device_vram_used_mib(),"id":case.id,"category":case.category,"input":case.text,"prepared":out.smart,"output":out.final_text,"used":out.rewriter_used,"fallback":out.rewriter_error,"total_ms":total_ms,"formatting_ms":out.formatting_ms,"rewrite_ms":out.rewrite_ms,"generation":client.generation.read().clone(),"missing":missing,"forbidden":forbidden,"layout_ok":layout_ok,"breaks_preserved":breaks_preserved}));
            std::fs::write(&output, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
        }
    }
    runtime.shutdown();
}
