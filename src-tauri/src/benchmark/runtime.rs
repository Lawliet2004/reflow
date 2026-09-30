use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::wer::calculate_wer;
use crate::asr::native::NativeAsrEngine;
use crate::asr::{ASREngine, Qwen3AsrSidecar};
use crate::model::manager::runtime_model_id;
use crate::model::ModelManager;
use crate::platform::PlatformSys;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct AudioSample {
    pub language: String,
    pub audio: PathBuf,
    pub reference: String,
}

#[derive(Serialize)]
pub struct RuntimeBenchmarkRow {
    pub model: String,
    pub runtime: String,
    pub language: String,
    pub wer_percent: Option<f64>,
    pub mean_latency_ms: Option<f64>,
    pub resident_vram_mb: Option<f32>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct RuntimeBenchmarkReport {
    pub rows: Vec<RuntimeBenchmarkRow>,
    pub native_within_gate: bool,
    pub default_runtime: &'static str,
}

pub fn run_runtime_benchmark(corpus_path: &Path) -> Result<RuntimeBenchmarkReport, String> {
    let bytes = std::fs::read(corpus_path).map_err(|e| e.to_string())?;
    let samples: Vec<AudioSample> = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if samples.is_empty() {
        return Err("The audio corpus is empty".into());
    }
    let manager = ModelManager::new(PlatformSys::get_models_dir());
    let mut rows = Vec::new();
    for model in ["0.6b", "1.7b"] {
        for runtime in ["python", "native"] {
            let id = runtime_model_id(model, runtime);
            let before = crate::capability::capabilities_uncached();
            let mut engine: Box<dyn ASREngine> = if runtime == "native" {
                Box::new(NativeAsrEngine::default())
            } else {
                Box::new(Qwen3AsrSidecar::new())
            };
            let ready = if manager.is_installed(&id) {
                engine
                    .initialize()
                    .and_then(|_| {
                        engine.load_model_with_precision(
                            &manager.get_model_dir(&id).to_string_lossy(),
                            "auto",
                            "int8",
                        )
                    })
                    .and_then(|_| wait_loaded(&mut *engine))
            } else {
                Err(format!("Model files for {id} are not installed"))
            };
            for language in ["en", "hi"] {
                let mut row = RuntimeBenchmarkRow {
                    model: model.into(),
                    runtime: runtime.into(),
                    language: language.into(),
                    wer_percent: None,
                    mean_latency_ms: None,
                    resident_vram_mb: None,
                    error: None,
                };
                let selected: Vec<&AudioSample> =
                    samples.iter().filter(|s| s.language == language).collect();
                if selected.is_empty() {
                    row.error = Some(format!("No {language} audio references in the corpus"));
                } else if let Err(error) = &ready {
                    row.error = Some(error.clone());
                } else {
                    match evaluate_audio(
                        &mut *engine,
                        &selected,
                        corpus_path.parent().unwrap_or(Path::new(".")),
                    ) {
                        Ok((wer, latency)) => {
                            row.wer_percent = Some(wer * 100.0);
                            row.mean_latency_ms = Some(latency);
                            let after = crate::capability::capabilities_uncached();
                            if engine.engine_status().device != "cpu"
                                && before.free_vram_mb() > 0.0
                                && after.free_vram_mb() > 0.0
                            {
                                row.resident_vram_mb =
                                    Some((before.free_vram_mb() - after.free_vram_mb()).max(0.0));
                            }
                        }
                        Err(error) => row.error = Some(error),
                    }
                }
                rows.push(row);
            }
            let _ = engine.unload_model();
        }
    }
    let native_within_gate = native_within_gate(&rows);
    Ok(RuntimeBenchmarkReport {
        rows,
        native_within_gate,
        default_runtime: "python",
    })
}

fn wait_loaded(engine: &mut dyn ASREngine) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(240);
    while Instant::now() < deadline {
        let status = engine.engine_status();
        if status.loaded {
            return Ok(());
        }
        if let Some(error) = status.error {
            return Err(error);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err("ASR benchmark model load timed out".into())
}

fn evaluate_audio(
    engine: &mut dyn ASREngine,
    samples: &[&AudioSample],
    root: &Path,
) -> Result<(f64, f64), String> {
    let mut errors = 0;
    let mut words = 0;
    let mut latency = 0.0;
    for sample in samples {
        if sample.reference.trim().is_empty() {
            return Err("Audio reference is empty".into());
        }
        let mut reader =
            hound::WavReader::open(root.join(&sample.audio)).map_err(|e| e.to_string())?;
        let spec = reader.spec();
        if spec.channels != 1
            || spec.sample_rate != 16000
            || spec.bits_per_sample != 16
            || spec.sample_format != hound::SampleFormat::Int
        {
            return Err("Benchmark audio must be 16 kHz, mono PCM16 WAV".into());
        }
        let audio = reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / 32768.0))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        engine.start_stream(&sample.language, &[])?;
        for chunk in audio.chunks(16000) {
            engine.push_audio(chunk)?;
        }
        let started = Instant::now();
        let hypothesis = engine.stop_stream()?;
        latency += started.elapsed().as_secs_f64() * 1000.0;
        if let Some(warning) = engine.take_last_warning() {
            return Err(format!("Benchmark transcript has a warning: {warning}"));
        }
        let result = calculate_wer(&sample.reference, &hypothesis);
        errors += result.words.substitutions + result.words.insertions + result.words.deletions;
        words += result.words.total_ref;
    }
    Ok((
        errors as f64 / words.max(1) as f64,
        latency / samples.len() as f64,
    ))
}

fn native_within_gate(rows: &[RuntimeBenchmarkRow]) -> bool {
    ["0.6b", "1.7b"].iter().all(|model| ["en", "hi"].iter().all(|language| {
        let find = |runtime| rows.iter().find(|r| r.model == *model && r.language == *language && r.runtime == runtime).and_then(|r| r.wer_percent);
        matches!((find("python"), find("native")), (Some(python), Some(native)) if (native - python).abs() <= 0.5)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gate_requires_both_sizes_and_both_languages() {
        assert!(!native_within_gate(&[]));
        let mut rows = Vec::new();
        for model in ["0.6b", "1.7b"] {
            for language in ["en", "hi"] {
                for runtime in ["python", "native"] {
                    rows.push(RuntimeBenchmarkRow {
                        model: model.into(),
                        runtime: runtime.into(),
                        language: language.into(),
                        wer_percent: Some(if runtime == "native" { 5.4 } else { 5.0 }),
                        mean_latency_ms: Some(100.0),
                        resident_vram_mb: None,
                        error: None,
                    });
                }
            }
        }
        assert!(native_within_gate(&rows));
        rows.last_mut().unwrap().wer_percent = Some(5.6);
        assert!(!native_within_gate(&rows));
        rows.last_mut().unwrap().wer_percent = None;
        assert!(!native_within_gate(&rows));
    }
}
