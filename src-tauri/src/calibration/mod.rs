//! Explicit, local quality-gated calibration. No downloads, history or insertion.
#[cfg(test)]
mod live_tests;
pub mod policy;
#[cfg(test)]
thread_local! { static SMOKE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
#[cfg(test)]
fn smoke() -> bool {
    SMOKE.get()
}
#[cfg(not(test))]
fn smoke() -> bool {
    false
}
use crate::{
    asr::AsrHandle,
    capability::Capabilities,
    context::AppContext,
    profile::Device,
    rewrite::{FlowRuntime, RewriteRequest},
    settings::AppSettings,
};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    OnceLock,
};
use std::time::{Duration, Instant, UNIX_EPOCH};

struct SpeechLease(AsrHandle);
impl Drop for SpeechLease {
    fn drop(&mut self) {
        let _ = self.0.unload_model_blocking();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub runtime: String,
    pub model: String,
    pub device: String,
    pub precision: String,
    pub refinement_model: String,
    pub gpu_layers: u32,
    pub context_size: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub label: String,
    pub median_ms: f64,
    pub p95_ms: f64,
    pub wer: f64,
    pub cer: f64,
    pub eligible: bool,
    pub error: Option<String>,
    pub choice: Choice,
    pub observed_vram_mb: f32,
    pub minimum_ram_mb: f32,
    pub transcript: String,
    pub output: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CalibrationStatus {
    pub running: bool,
    pub phase: String,
    pub candidates: Vec<Candidate>,
    pub winner_id: Option<String>,
    pub error: Option<String>,
    pub language: String,
    pub reference: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cache {
    version: u32,
    fingerprint: String,
    language: String,
    reference_sha256: String,
    execution: String,
    winner: Candidate,
    results: CalibrationStatus,
}
#[derive(Default)]
struct Controller {
    running: AtomicBool,
    cancel: AtomicBool,
    status: RwLock<CalibrationStatus>,
    pending: Mutex<Option<Cache>>,
    active: Mutex<Option<(AsrHandle, u64)>>,
}
fn controller() -> &'static Controller {
    static VALUE: OnceLock<Controller> = OnceLock::new();
    VALUE.get_or_init(Controller::default)
}
fn cache_path() -> PathBuf {
    crate::platform::PlatformSys::get_app_dir().join("calibration-v1.json")
}
fn phase(message: impl Into<String>) {
    controller().status.write().phase = message.into();
}
fn check_cancel() -> Result<(), String> {
    if controller().cancel.load(Ordering::Acquire) {
        Err("Calibration cancelled. Previous settings were retained.".into())
    } else {
        Ok(())
    }
}
pub fn status() -> CalibrationStatus {
    let live = controller().status.read().clone();
    if !live.phase.is_empty() {
        return live;
    }
    read_cache().map(|c| c.results).unwrap_or_default()
}
pub fn cancel() {
    controller().cancel.store(true, Ordering::Release);
    let active = controller().active.lock().clone();
    if let Some((handle, id)) = active {
        tauri::async_runtime::spawn(async move {
            let _ = handle.cancel_stream(id).await;
        });
    }
}
fn read_cache() -> Option<Cache> {
    serde_json::from_slice(&std::fs::read(cache_path()).ok()?).ok()
}
fn stamp_tree(path: &Path, output: &mut Vec<(String, u64, u128)>) {
    let mut pending = vec![path.to_path_buf()];
    let mut seen = std::collections::HashSet::new();
    while let Some(path) = pending.pop() {
        if seen.len() >= 4096 || output.len() >= 4096 {
            break;
        }
        let Ok(canonical) = path.canonicalize() else {
            continue;
        };
        if !seen.insert(canonical.clone()) {
            continue;
        }
        if let Ok(meta) = canonical.metadata() {
            if meta.is_file() {
                output.push((
                    canonical.to_string_lossy().into_owned(),
                    meta.len(),
                    meta.modified()
                        .ok()
                        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos())
                        .unwrap_or(0),
                ));
            } else if let Ok(entries) = canonical.read_dir() {
                pending.extend(entries.flatten().map(|entry| entry.path()));
            }
        }
    }
}
fn execution(settings: &AppSettings) -> String {
    serde_json::json!({"intent":format!("{:?}",settings.resolve_intent()),"style":settings.style,"dictionary":settings.dictionary_terms,"replacements":settings.custom_replacements,"profiles":settings.application_profiles}).to_string()
}
pub fn is_running() -> bool {
    controller().running.load(Ordering::Acquire)
}
fn python_files() -> Vec<PathBuf> {
    static ROOTS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    let roots = ROOTS.get_or_init(|| {
        let Some(python) = crate::asr::Qwen3AsrSidecar::find_python() else { return vec![]; };
        let code = "import sys,site,json; print(json.dumps([sys.executable]+site.getsitepackages()+[site.getusersitepackages()]))";
        let Some(output) = crate::asr::Qwen3AsrSidecar::bounded_python_probe(&python, code) else { return vec![]; };
        serde_json::from_slice::<Vec<String>>(&output).unwrap_or_default().into_iter().map(PathBuf::from).collect()
    });
    let mut files = Vec::new();
    for root in roots {
        if root.is_file() {
            files.push(root.clone());
            continue;
        }
        if let Ok(entries) = root.read_dir() {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.ends_with(".dist-info")
                    && [
                        "torch-",
                        "transformers-",
                        "qwen_asr-",
                        "bitsandbytes-",
                        "fermion_research-",
                    ]
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
                {
                    files.push(entry.path().join("METADATA"));
                }
            }
        }
    }
    files
}
fn fingerprint(ctx: &AppContext, caps: &Capabilities) -> String {
    let mut files = Vec::new();
    for path in python_files() {
        stamp_tree(&path, &mut files);
    }
    for runtime in ["python", "native"] {
        for model in ["0.6b", "1.7b"] {
            stamp_tree(
                &ctx.model_manager
                    .get_model_dir(&crate::model::manager::runtime_model_id(model, runtime)),
                &mut files,
            );
        }
    }
    for model in ["qwen3.5-0.8b", "qwen3.5-2b"] {
        stamp_tree(&crate::rewrite::flow_gguf_path(model), &mut files);
    }
    stamp_tree(&ctx.model_manager.get_model_dir("phonon-2"), &mut files);
    let binary = crate::rewrite::llama_server_bin();
    if let Some(parent) = binary.parent() {
        stamp_tree(parent, &mut files);
    }
    files.sort();
    let gpus: Vec<_> = caps
        .gpus
        .iter()
        .map(|g| (&g.name, &g.driver_version, g.total_vram_mb))
        .collect();
    let value = serde_json::json!({"app":env!("CARGO_PKG_VERSION"),"cpu":caps.cpu.model,"cores":caps.cpu.physical_cores,"gpus":gpus,"cuda":caps.cuda,"vulkan":caps.vulkan,"files":files,"python":std::env::var("REFLOW_PYTHON").ok(),"asr_code":include_str!("../../../model-runtime/qwen3_asr_runtime.py"),"phonon_code":include_str!("../../../model-runtime/phonon_runtime.py"),"rules":include_str!("../rewrite/safety.rs"),"prompt":include_str!("../rewrite/prompt.rs")});
    hex::encode(Sha256::digest(value.to_string().as_bytes()))
}
pub fn cached_choice(
    ctx: &AppContext,
    caps: &Capabilities,
    settings: &AppSettings,
) -> Option<Choice> {
    if settings.preset != "auto" || settings.auto_detect_language {
        return None;
    }
    let saved = read_cache()?;
    if saved.version != 1
        || saved.language != settings.language
        || !saved.winner.eligible
        || saved.execution != execution(settings)
        || saved.fingerprint != fingerprint(ctx, caps)
    {
        return None;
    }
    let choice = saved.winner.choice;
    if choice.runtime != settings.asr.runtime {
        return None;
    }
    // A saved Qwen calibration must not override an explicitly selected Phonon.
    if settings.asr.model == "phonon-2" && choice.model != "phonon-2" {
        return None;
    }
    if settings.resolve_intent().run_llm != (choice.refinement_model != "none") {
        return None;
    }
    Some(choice)
}
pub fn apply(ctx: &AppContext, id: &str) -> Result<bool, String> {
    if controller().running.load(Ordering::Acquire) {
        return Err("Wait for calibration to finish.".into());
    }
    let _operation = ctx
        .session_operation
        .try_lock()
        .map_err(|_| "Finish dictation before applying calibration.")?;
    let cache = controller()
        .pending
        .lock()
        .clone()
        .or_else(read_cache)
        .ok_or("No qualified calibration is available.")?;
    if cache.winner.id != id
        || !cache.winner.eligible
        || cache.execution != execution(&ctx.settings_store.get())
        || cache.fingerprint != fingerprint(ctx, &crate::capability::capabilities_uncached())
    {
        return Err("Calibration is stale or this candidate did not qualify. Run it again.".into());
    }
    let bytes = serde_json::to_vec_pretty(&cache).map_err(|e| e.to_string())?;
    let path = cache_path();
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&temporary, &path).map_err(|e| e.to_string())?;
    let mut settings = ctx.settings_store.get();
    settings.preset = "auto".into();
    settings.language = cache.language;
    settings.auto_detect_language = false;
    settings.asr.runtime = cache.winner.choice.runtime;
    settings.asr.model = cache
        .winner
        .choice
        .model
        .trim_start_matches("native-")
        .into();
    ctx.settings_store.update(settings)?;
    Ok(true)
}
pub async fn run(
    ctx: AppContext,
    resource_dir: Option<PathBuf>,
    reference: String,
    language: String,
    seconds: u64,
) -> Result<CalibrationStatus, String> {
    if reference.chars().count() > 2048 || policy::error_rates(&reference, &reference).is_none() {
        return Err("Enter the exact words you will speak (at most 2048 characters).".into());
    }
    crate::asr::languages::language_name(&language)?;
    if language == "auto" {
        return Err("Choose a language for the quality test.".into());
    }
    if !(3..=10).contains(&seconds) {
        return Err("Record between three and ten seconds.".into());
    }
    if ctx.settings_store.get().asr.model == "phonon-2" && language != "en" {
        return Err("Phonon-2 supports English only. Choose English for calibration.".into());
    }
    if !speech_candidates(&ctx.settings_store.get().asr.model, &language)
        .iter()
        .any(|(_, model)| ctx.model_manager.is_installed(model))
    {
        return Err("Install a speech model before calibrating.".into());
    }
    if ctx.settings_store.get().resolve_intent().run_llm
        && (!crate::rewrite::llama_server_bin().is_file()
            || !["qwen3.5-0.8b", "qwen3.5-2b"]
                .iter()
                .any(|model| crate::rewrite::flow_gguf_path(model).is_file()))
    {
        return Err("Install the writing runtime and a writing model before calibrating.".into());
    }
    let _operation = ctx
        .session_operation
        .try_lock()
        .map_err(|_| "Finish the current operation before calibrating.")?;
    if !matches!(
        *ctx.state_enum.read(),
        crate::state::AppStateEnum::Ready | crate::state::AppStateEnum::Idle
    ) {
        return Err("Finish the current operation before calibrating.".into());
    }
    if controller().running.swap(true, Ordering::AcqRel) {
        return Err("Calibration is already running.".into());
    }
    controller().cancel.store(false, Ordering::Release);
    *controller().pending.lock() = None;
    *controller().status.write() = CalibrationStatus {
        running: true,
        phase: "Recording sample — speak your reference now".into(),
        language: language.clone(),
        reference: reference.clone(),
        ..Default::default()
    };
    let settings = ctx.settings_store.get();
    let result = async {
        let (health, samples) = crate::audio::preflight::capture_sample(&settings, seconds).await?;
        if health.peak < 0.001 || health.dropped_chunks > 0 || health.clipped_pct > 5.0 {
            return Err(format!(
                "Sample failed the microphone check: {}",
                health.assessment
            ));
        }
        check_cancel()?;
        let was_loaded = ctx.asr_handle.is_model_loaded();
        let original_load = ctx.last_asr_load.read().clone();
        ctx.flow_runtime.shutdown();
        ctx.asr_handle.unload_model().await?;
        let worker_ctx = ctx.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            measure(&worker_ctx, resource_dir, samples, &reference, &language)
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|result| result);
        phase("Restoring speech model");
        if was_loaded {
            if let Some((model, device, precision)) = original_load {
                let directory = ctx.model_manager.get_model_dir(&model);
                if let Err(error) = ctx
                    .asr_handle
                    .load_model_with_precision(&directory.to_string_lossy(), &device, &precision)
                    .await
                {
                    return Err(format!(
                        "Calibration ended, but speech needs reloading: {error}"
                    ));
                }
            }
        }
        result
    }
    .await;
    controller().running.store(false, Ordering::Release);
    *controller().active.lock() = None;
    let mut status = controller().status.write();
    status.running = false;
    status.phase = if result.is_ok() {
        "complete"
    } else {
        "stopped"
    }
    .into();
    if let Err(error) = &result {
        status.error = Some(error.clone());
        status.winner_id = None;
    }
    if result.is_ok() && status.winner_id.is_none() {
        status.error = Some("No candidate met the accuracy, memory and latency gates. Improve the sample or keep the hardware defaults.".into());
    }
    Ok(status.clone())
}
fn speech_candidates(selected: &str, language: &str) -> Vec<(&'static str, String)> {
    if selected == "phonon-2" {
        return if language == "en" {
            vec![("python", "phonon-2".into())]
        } else {
            vec![]
        };
    }
    let mut candidates = Vec::new();
    for runtime in ["python", "native"] {
        for model in ["0.6b", "1.7b"] {
            candidates.push((
                runtime,
                crate::model::manager::runtime_model_id(model, runtime),
            ));
        }
    }
    if language == "en" {
        candidates.push(("python", "phonon-2".into()));
    }
    candidates
}

#[cfg(test)]
mod phonon_tests {
    use super::speech_candidates;

    #[test]
    fn phonon_calibration_preserves_selected_family() {
        assert_eq!(
            speech_candidates("phonon-2", "en"),
            vec![("python", "phonon-2".into())]
        );
        assert!(speech_candidates("phonon-2", "hi").is_empty());
    }

    #[test]
    fn english_comparison_includes_phonon_but_hindi_does_not() {
        assert!(speech_candidates("0.6b", "en")
            .iter()
            .any(|(_, model)| model == "phonon-2"));
        assert!(!speech_candidates("0.6b", "hi")
            .iter()
            .any(|(_, model)| model == "phonon-2"));
    }
}

fn measure(
    ctx: &AppContext,
    resource_dir: Option<PathBuf>,
    samples: Vec<f32>,
    reference: &str,
    language: &str,
) -> Result<(), String> {
    let settings = ctx.settings_store.get();
    let intent = settings.resolve_intent();
    let caps = crate::capability::capabilities_uncached();
    let mut assessments = Vec::new();
    let mut choices = Vec::new();
    for (runtime, model_id) in speech_candidates(&settings.asr.model, language) {
        if !ctx.model_manager.is_installed(&model_id) {
            continue;
        }
        let Some(manifest) = crate::profile::asr_manifest(&model_id) else {
            continue;
        };
        for device in [Device::Cuda, Device::Vulkan, Device::Cpu] {
            if !manifest.devices.contains(&device) {
                continue;
            }
            if device == Device::Cuda && !caps.can_use_cuda() {
                continue;
            }
            if device == Device::Vulkan && !caps.can_use_vulkan() {
                continue;
            }
            for precision in manifest.supported_precisions_on(device) {
                let room = if device == Device::Cpu {
                    caps.ram.available_mb > manifest.estimated_cpu_ram_mb() + 1024.0
                } else {
                    caps.free_vram_mb() > manifest.estimated_vram_mb(precision) + 384.0
                };
                if !room {
                    continue;
                }
                choices.push((runtime, model_id.clone(), device, precision));
            }
        }
    }
    if smoke() {
        choices.retain(|(runtime, model, device, precision)| {
            *runtime == "python"
                && model == "0.6b"
                && *device == Device::Cuda
                && *precision == crate::profile::Precision::Bf16
        });
    }
    if choices.is_empty() {
        return Err("No installed speech model fits the current hardware headroom.".into());
    }
    let use_llm = intent.run_llm;
    let llm_models: Vec<_> = if use_llm {
        ["qwen3.5-0.8b", "qwen3.5-2b"]
            .into_iter()
            .filter(|model| crate::rewrite::flow_gguf_path(model).is_file())
            .collect()
    } else {
        vec!["none"]
    };
    if llm_models.is_empty() {
        return Err("Install a writing model before calibrating ASR and LLM together.".into());
    }
    for (runtime, model, device, precision) in choices {
        check_cancel()?;
        let handle = AsrHandle::new_runtime(runtime);
        let _lease = SpeechLease(handle.clone());
        if let Some(path) = &resource_dir {
            handle.set_resource_dir_blocking(path.clone())?;
        }
        let initialized = handle.initialize_blocking();
        let directory = ctx.model_manager.get_model_dir(&model);
        let loaded = initialized.and_then(|_| {
            handle.load_model_with_precision_blocking(
                &directory.to_string_lossy(),
                device.as_str(),
                precision.as_str(),
            )
        });
        let loaded = loaded.and_then(|_| {
            let deadline = Instant::now() + Duration::from_secs(240);
            while Instant::now() < deadline {
                check_cancel()?;
                // Refresh through the actor using its blocking reply bridge.
                let status = tauri::async_runtime::block_on(handle.refresh_status())?;
                if let Some(error) = status.error {
                    return Err(error);
                }
                if status.loaded && !status.is_loading {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err("Speech model loading timed out.".into())
        });
        for llm_model in &llm_models {
            if smoke() && *llm_model != "qwen3.5-0.8b" {
                continue;
            }
            let resident_caps = crate::capability::capabilities_uncached();
            let admitted = crate::rewrite::server::latency_optimized_gpu_layers(
                llm_model,
                &resident_caps,
                384,
                1024,
            );
            let mut layers = vec![0];
            if *llm_model != "none" && admitted > 0 {
                layers.push(admitted);
                if admitted == 99 {
                    layers.push(
                        crate::rewrite::server::flow_model_spec(llm_model).gpu_layer_count / 2,
                    );
                }
            }
            for gpu_layers in layers {
                if smoke() && gpu_layers != 0 && gpu_layers != admitted {
                    continue;
                }
                check_cancel()?;
                let choice = Choice {
                    runtime: runtime.into(),
                    model: model.clone(),
                    device: device.as_str().into(),
                    precision: precision.as_str().into(),
                    refinement_model: (*llm_model).into(),
                    gpu_layers,
                    context_size: 1024,
                };
                let id = format!(
                    "{runtime}-{model}-{}-{}-{llm_model}-{gpu_layers}",
                    device.as_str(),
                    precision.as_str()
                );
                let label = format!(
                    "{runtime} {model} {} {} + {llm_model} {}",
                    device.as_str(),
                    precision.as_str(),
                    if gpu_layers == 0 {
                        "CPU".into()
                    } else {
                        format!("GPU {gpu_layers} layers")
                    }
                );
                phase(format!("Measuring {label}"));
                let mut row = Candidate {
                    id,
                    label,
                    choice,
                    median_ms: 0.0,
                    p95_ms: 0.0,
                    wer: 1.0,
                    cer: 1.0,
                    eligible: false,
                    error: None,
                    observed_vram_mb: 0.0,
                    minimum_ram_mb: f32::MAX,
                    transcript: String::new(),
                    output: String::new(),
                };
                let flow = FlowRuntime::default();
                let result = loaded.clone().and_then(|_| {
                    if *llm_model != "none" { flow.ensure(llm_model, if gpu_layers == 0 {"cpu"} else {"auto"}, Some(gpu_layers), 384, 1024)?; flow.set_deadline_ms(15_000); if flow.active_n_gpu_layers() != Some(gpu_layers) { return Err("Requested offload fell back; candidate excluded.".into()); } }
                    let mut times = Vec::new(); let mut worst_wer: f64 = 0.0; let mut worst_cer: f64 = 0.0;
                    for repetition in 0..4 {
                        check_cancel()?;
                        let started = Instant::now();
                        let id = handle.next_session_id();
                        *controller().active.lock() = Some((handle.clone(), id));
                        handle.start_stream_blocking(id, language, &crate::session::session_vocabulary(&settings))?;
                        handle.push_audio_blocking(id, 0, &samples)?;
                        let raw = handle.stop_stream_blocking(id)?;
                        *controller().active.lock() = None;
                        let output = if *llm_model == "none" { raw.clone() } else {
                            let candidate = flow.client.read().rewrite(&RewriteRequest {text:raw.clone(), cleanup_level:intent.cleanup_level.clone(), style:settings.style.clone(), dictation_mode:settings.dictation_mode.clone(), vocabulary:crate::session::session_vocabulary(&settings), app_process:String::new(), model_id:(*llm_model).into()})?;
                            crate::rewrite::accept_rewrite(&raw, &candidate, intent.cleanup_level.as_str()).ok_or("Writing changed protected transcript content; candidate excluded.")?
                        };
                        let duration = started.elapsed().as_secs_f64() * 1000.0;
                        let (wer, cer) = policy::error_rates(reference, &raw).ok_or("Reference has no words.")?;
                        let (final_wer, final_cer) = policy::error_rates(reference, &output).ok_or("Reference has no words.")?;
                        let live = crate::capability::capabilities_uncached();
                        row.minimum_ram_mb = row.minimum_ram_mb.min(live.ram.available_mb);
                        row.observed_vram_mb = row.observed_vram_mb.max(live.primary_gpu().map(|g| g.used_vram_mb).unwrap_or(0.0));
                        let status = handle.engine_status();
                        if status.spill_detected || live.ram.available_mb < 1024.0 || (gpu_layers > 0 && live.free_vram_mb() < 128.0) { return Err("Memory spill or insufficient co-resident headroom; candidate excluded.".into()); }
                        if status.device != device.as_str() || status.precision != precision.as_str() { return Err("Speech runtime substituted the requested configuration; candidate excluded.".into()); }
                        if repetition > 0 { times.push(duration); worst_wer = worst_wer.max(wer).max(final_wer); worst_cer = worst_cer.max(cer).max(final_cer); }
                        row.transcript = raw; row.output = output;
                    }
                    (row.median_ms, row.p95_ms) = policy::median_p95(&times).ok_or("No valid timings")?;
                    row.wer = worst_wer; row.cer = worst_cer;
                    Ok(())
                });
                flow.shutdown();
                row.error = result.err();
                let assessment = policy::Assessment {
                    median_ms: row.median_ms,
                    p95_ms: row.p95_ms,
                    wer: row.wer,
                    cer: row.cer,
                    character_language: matches!(language, "zh" | "yue" | "ja" | "ko" | "th"),
                    repeats: 3,
                    spill: row.error.is_some(),
                    labelled: true,
                };
                row.eligible = policy::select_winner(std::slice::from_ref(&assessment)).is_some();
                if !row.eligible && row.error.is_none() {
                    row.error = Some(
                        "Transcript accuracy or tail latency did not meet the quality gate.".into(),
                    );
                }
                assessments.push(assessment);
                controller().status.write().candidates.push(row);
            }
        }
        let _ = handle.unload_model_blocking();
    }
    check_cancel()?;
    if let Some(index) = policy::select_winner(&assessments) {
        let mut status = controller().status.write();
        let winner = status.candidates[index].clone();
        status.winner_id = Some(winner.id.clone());
        let mut results = status.clone();
        results.running = false;
        results.phase = "complete".into();
        *controller().pending.lock() = Some(Cache {
            version: 1,
            fingerprint: fingerprint(ctx, &caps),
            language: language.into(),
            reference_sha256: hex::encode(Sha256::digest(reference.as_bytes())),
            execution: execution(&settings),
            winner,
            results,
        });
    }
    Ok(())
}
