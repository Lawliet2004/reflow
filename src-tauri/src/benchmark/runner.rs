use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// `None` on a metric means "not measured on this machine". Nothing in this
/// file may fabricate a plausible-looking value: a benchmark that invents its
/// own numbers validates whatever it was meant to check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrBenchmarkMetrics {
    pub model_id: String,
    pub device: String,
    pub precision: String,
    pub load_ms: u64,
    /// Warmup real-time factor, when a load has run one.
    pub warmup_rtf: Option<f64>,
    /// Timed decode on real dictation — only the runtime corpus measures it.
    #[serde(default)]
    pub average_inference_ms: Option<u64>,
    /// WER/CER need a labeled corpus; `benchmark::runtime` is the harness.
    #[serde(default)]
    pub corpus_wer: Option<f64>,
    #[serde(default)]
    pub corpus_cer: Option<f64>,
    pub spill_detected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefinementBenchmarkMetrics {
    pub model_id: String,
    pub device: String,
    #[serde(default)]
    pub load_ms: Option<u64>,
    #[serde(default)]
    pub tokens_per_second: Option<f64>,
    #[serde(default)]
    pub safety_pass_rate: Option<f64>,
    #[serde(default)]
    pub average_latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FullBenchmarkReport {
    pub timestamp: String,
    pub asr: Option<AsrBenchmarkMetrics>,
    pub refinement: Option<RefinementBenchmarkMetrics>,
    pub hardware_summary: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_report_serializes_and_deserializes() {
        let report = FullBenchmarkReport {
            timestamp: "2026-01-01T00:00:00Z".into(),
            asr: Some(AsrBenchmarkMetrics {
                model_id: "0.6b".into(),
                device: "cuda".into(),
                precision: "bf16".into(),
                load_ms: 350,
                warmup_rtf: Some(0.28),
                average_inference_ms: None,
                corpus_wer: None,
                corpus_cer: None,
                spill_detected: false,
            }),
            refinement: None,
            hardware_summary: "test bench".into(),
        };
        let json = serde_json::to_string(&report).unwrap();
        let back: FullBenchmarkReport = serde_json::from_str(&json).unwrap();
        let asr = back.asr.unwrap();
        assert_eq!(asr.corpus_wer, None);
        assert_eq!(asr.load_ms, 350);
        assert_eq!(asr.warmup_rtf, Some(0.28));
    }
}
