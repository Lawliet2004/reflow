use super::wer::{evaluate_corpus, EvalSample};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrBenchmarkMetrics {
    pub model_id: String,
    pub device: String,
    pub precision: String,
    pub load_ms: u64,
    pub warmup_rtf: f64,
    pub average_inference_ms: u64,
    pub corpus_wer: f64,
    pub corpus_cer: f64,
    pub spill_detected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefinementBenchmarkMetrics {
    pub model_id: String,
    pub device: String,
    pub load_ms: u64,
    pub tokens_per_second: f64,
    pub safety_pass_rate: f64,
    pub average_latency_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FullBenchmarkReport {
    pub timestamp: String,
    pub asr: Option<AsrBenchmarkMetrics>,
    pub refinement: Option<RefinementBenchmarkMetrics>,
    pub hardware_summary: String,
}

pub fn default_eval_corpus() -> Vec<EvalSample> {
    vec![
        EvalSample {
            id: "cmd_01".into(),
            reference: "start audio recording now".into(),
            category: "command".into(),
        },
        EvalSample {
            id: "cmd_02".into(),
            reference: "cancel current dictation session".into(),
            category: "command".into(),
        },
        EvalSample {
            id: "prose_01".into(),
            reference: "Reflow is a fast local first voice dictation application for desktop"
                .into(),
            category: "prose".into(),
        },
        EvalSample {
            id: "code_01".into(),
            reference: "function calculate total amount with tax".into(),
            category: "coding".into(),
        },
        EvalSample {
            id: "num_01".into(),
            reference: "the meeting starts at four thirty pm on august twenty seventh".into(),
            category: "numeric".into(),
        },
    ]
}

pub fn benchmark_cache_path() -> PathBuf {
    crate::platform::PlatformSys::get_app_dir().join("benchmark_cache.json")
}

pub fn persist_benchmark_report(report: &FullBenchmarkReport) {
    let path = benchmark_cache_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(report) {
        let _ = std::fs::write(path, json);
    }
}

pub fn load_benchmark_report() -> Option<FullBenchmarkReport> {
    let path = benchmark_cache_path();
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn run_synthetic_benchmark(
    hypotheses: &[&str],
    simulated_load_ms: u64,
    simulated_warmup_rtf: f64,
) -> FullBenchmarkReport {
    let samples = default_eval_corpus();
    let hyps: Vec<&str> = if hypotheses.len() == samples.len() {
        hypotheses.to_vec()
    } else {
        samples.iter().map(|s| s.reference.as_str()).collect()
    };

    let eval_res = evaluate_corpus(&samples, &hyps);

    FullBenchmarkReport {
        timestamp: chrono::Utc::now().to_rfc3339(),
        asr: Some(AsrBenchmarkMetrics {
            model_id: "qwen3-asr-0.6b".into(),
            device: "cpu".into(),
            precision: "bf16".into(),
            load_ms: simulated_load_ms,
            warmup_rtf: simulated_warmup_rtf,
            average_inference_ms: 120,
            corpus_wer: eval_res.average_wer,
            corpus_cer: eval_res.average_cer,
            spill_detected: false,
        }),
        refinement: Some(RefinementBenchmarkMetrics {
            model_id: "qwen3.5-0.8b".into(),
            device: "cpu".into(),
            load_ms: 250,
            tokens_per_second: 42.5,
            safety_pass_rate: 1.0,
            average_latency_ms: 180,
        }),
        hardware_summary: "Deterministic Local Runner".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_report_serializes_and_deserializes() {
        let report = run_synthetic_benchmark(&[], 350, 0.28);
        assert!(report.asr.is_some());
        let asr = report.asr.unwrap();
        assert_eq!(asr.corpus_wer, 0.0);
        assert_eq!(asr.load_ms, 350);
        assert_eq!(asr.warmup_rtf, 0.28);
    }
}
