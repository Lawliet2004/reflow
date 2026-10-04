//! Hardware-aware execution policy. Preferences remain on disk; this plan is
//! recomputed at load/launch boundaries and never changes a running recording.
use serde::{Deserialize, Serialize};

use crate::capability::Capabilities;
use crate::profile::{Device, MeasuredPeaks, Precision, Preset, ProfileReason};
use crate::settings::AppSettings;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimePlan {
    pub preset: Preset,
    pub asr_model: String,
    pub asr_device: String,
    pub asr_precision: String,
    pub refinement_model: Option<String>,
    pub refinement_device: String,
    pub refinement_gpu_layers: u32,
    pub context_size: u32,
    pub inference_threads: usize,
    pub keep_asr_loaded: bool,
    pub keep_refinement_warm: bool,
    pub reasons: Vec<ProfileReason>,
    pub error: Option<String>,
}

fn reason(code: &str, detail: impl Into<String>) -> ProfileReason {
    ProfileReason {
        code: code.into(),
        detail: detail.into(),
    }
}

/// `asr_resident` means free VRAM already excludes the live ASR weights.
/// `refinement_resident` means the target refinement model is already loaded,
/// preventing admission of a second copy of its RAM estimate.
pub fn plan(
    settings: &AppSettings,
    caps: &Capabilities,
    installed: &dyn Fn(&str) -> bool,
    measured: &dyn Fn(&str, Device, Precision) -> MeasuredPeaks,
    asr_resident: bool,
    refinement_resident: bool,
) -> RuntimePlan {
    let preset = Preset::parse(&settings.preset);
    let automatic = preset != Preset::Custom;
    let selection = crate::profile::select_asr_load_for_runtime(
        &settings.asr.runtime,
        preset,
        &settings.asr.model,
        if automatic {
            "auto"
        } else {
            &settings.asr.device
        },
        if automatic {
            "auto"
        } else {
            &settings.asr.precision
        },
        installed,
        caps,
        measured,
    );
    let mut reasons = Vec::new();
    if let Some(detail) = selection.downgrade {
        reasons.push(reason("asr_selection", detail));
    }
    let intent = settings.resolve_intent();
    let run_llm = intent.run_llm && preset != Preset::Fast;
    let model = run_llm.then_some(intent.flow_model);
    let mut refinement_caps = caps.clone();
    if !asr_resident && selection.device.is_gpu() {
        let observed = measured(selection.model_id, selection.device, selection.precision);
        let peak = observed.vram_mb.unwrap_or_else(|| {
            crate::profile::asr_manifest(selection.model_id)
                .map(|m| m.estimated_vram_mb(selection.precision))
                .unwrap_or(0.0)
        });
        if let Some(index) = caps.primary_gpu().map(|g| g.index) {
            if let Some(gpu) = refinement_caps.gpus.iter_mut().find(|g| g.index == index) {
                gpu.free_vram_mb = (gpu.free_vram_mb - peak).max(0.0);
            }
        }
    }
    let context_size = if automatic {
        // Capacity-based tiers are stable between dictations; transient free
        // memory must not repeatedly invalidate the warm server's cache.
        if caps.total_vram_mb() >= 16_384.0 {
            4096
        } else if caps.total_vram_mb() >= 8192.0 {
            2048
        } else {
            1024
        }
    } else {
        settings.refinement.context_size
    };
    let layers = if !run_llm {
        0
    } else if !automatic {
        if settings.refinement.device == "cpu" || !settings.memory_policy.allow_gpu_refinement {
            0
        } else if settings.refinement.gpu_layers >= 0 {
            settings.refinement.gpu_layers as u32
        } else {
            crate::rewrite::server::latency_optimized_gpu_layers(
                model.as_deref().unwrap_or("none"),
                &refinement_caps,
                settings.memory_policy.vram_reserve_mb,
                context_size,
            )
        }
    } else {
        crate::rewrite::server::latency_optimized_gpu_layers(
            model.as_deref().unwrap_or("none"),
            &refinement_caps,
            settings.memory_policy.vram_reserve_mb,
            context_size,
        )
    };
    let device = if layers > 0 {
        if automatic {
            "auto".to_string()
        } else {
            settings.refinement.device.clone()
        }
    } else {
        "cpu".to_string()
    };
    let mode = if layers > 0 {
        crate::rewrite::LlamaMode::Gpu(String::new())
    } else {
        crate::rewrite::LlamaMode::Cpu
    };
    let threads = crate::rewrite::server::llama_server_threads(
        &mode,
        layers,
        crate::rewrite::server::flow_model_spec(model.as_deref().unwrap_or("none")).gpu_layer_count,
        caps,
    )
    .unwrap_or_else(|| {
        caps.cpu
            .physical_cores
            .unwrap_or_else(|| caps.cpu.inference_threads())
            .max(1)
    });
    let pressure = caps.ram.available_mb < 1536.0;
    let model = model.filter(|id| {
        if !automatic || refinement_resident { return true; }
        // GGUF remains quantized on CPU. ASR's fp32 estimate is inappropriate.
        let needed = crate::rewrite::server::flow_model_spec(id).approx_bytes as f32
            / (1024.0 * 1024.0) + 512.0;
        // GGUF is mmap-backed even with GPU offload. Reserve room for the app,
        // audio and OS instead of starting an additional runtime into paging.
        if caps.ram.available_mb < needed + 1024.0 {
            reasons.push(reason("refinement_ram_pressure", "Refinement is paused because available RAM is too low; deterministic cleanup remains available."));
            false
        } else { true }
    });
    if preset == Preset::Fast {
        reasons.push(reason(
            "fast_skips_llm",
            "Fast uses speech recognition and deterministic cleanup without the LLM wait.",
        ));
    }
    if automatic {
        reasons.push(reason("hardware_runtime", format!(
            "{} GPU layers, {} CPU threads and a {} token context are selected from current hardware and memory headroom. Oversized rewrites preserve the deterministic transcript.",
            if model.is_some() { layers } else { 0 }, threads, context_size,
        )));
    }
    RuntimePlan {
        preset,
        asr_model: selection.model_id.into(),
        asr_device: selection.device.as_str().into(),
        asr_precision: selection.precision.as_str().into(),
        refinement_gpu_layers: if model.is_some() { layers } else { 0 },
        refinement_model: model,
        refinement_device: device,
        context_size,
        inference_threads: threads,
        keep_asr_loaded: if automatic {
            true
        } else {
            settings.asr.keep_loaded
        },
        keep_refinement_warm: if automatic {
            !pressure
        } else {
            settings.refinement.keep_warm
        },
        reasons,
        error: selection.error.map(|e| e.detail),
    }
}

/// Apply execution choices to a temporary copy; never rewrite user preferences.
pub fn effective_settings(settings: &AppSettings, plan: &RuntimePlan) -> AppSettings {
    let mut effective = settings.clone();
    if plan.preset == Preset::Custom {
        return effective;
    }
    effective.asr.keep_loaded = plan.keep_asr_loaded;
    effective.refinement.context_size = plan.context_size;
    effective.refinement.keep_warm = plan.keep_refinement_warm;
    effective.refinement.device = if plan
        .reasons
        .iter()
        .any(|reason| reason.code == "calibrated_runtime")
    {
        plan.refinement_device.clone()
    } else {
        "auto".into()
    };
    // Retain automatic offload at launch: the runtime checks free memory again
    // after ASR has loaded and caches the successful launch until invalidated.
    effective.refinement.gpu_layers = if plan
        .reasons
        .iter()
        .any(|reason| reason.code == "calibrated_runtime")
    {
        plan.refinement_gpu_layers as i32
    } else {
        -1
    };
    if let Some(model) = &plan.refinement_model {
        effective.refinement.model = model.clone();
        effective.flow_model = model.clone();
        if plan
            .reasons
            .iter()
            .any(|reason| reason.code == "calibrated_runtime")
        {
            effective.intelligence_tier = if model == "qwen3.5-2b" {
                "deep_context"
            } else {
                "smart_flow"
            }
            .into();
        }
    }
    effective.memory_policy.allow_gpu_refinement = true;
    if plan.refinement_model.is_none() {
        effective.intelligence_tier = "raw_verbatim".into();
    }
    effective
}

pub fn current_plan(ctx: &crate::context::AppContext) -> RuntimePlan {
    let caps = crate::capability::capabilities();
    let store = crate::profile::measurements::Measurements::load();
    let measured = crate::profile::measurements::lookup_fn(&store, &caps);
    let mut settings = ctx.settings_store.get();
    if crate::platform::media::on_battery() == Some(true) {
        if let Some(preset) = &settings.power_policy.battery_preset {
            settings.preset = preset.clone();
        }
    }
    let engine = ctx.asr_handle.engine_status();
    let mut result = plan(
        &settings,
        &caps,
        &|id| ctx.model_manager.is_installed(id),
        &measured,
        engine.loaded,
        refinement_is_resident(ctx, &settings),
    );
    if let Some(choice) = crate::calibration::cached_choice(ctx, &caps, &settings) {
        let resident = ctx.flow_runtime.active_model().as_deref()
            == Some(choice.refinement_model.as_str())
            && ctx.flow_runtime.status_ready();
        let safe_layers = crate::rewrite::server::latency_optimized_gpu_layers(
            &choice.refinement_model,
            &caps,
            settings.memory_policy.vram_reserve_mb,
            choice.context_size,
        );
        let ram_fits = resident
            || caps.ram.available_mb
                > crate::rewrite::server::flow_model_spec(&choice.refinement_model).approx_bytes
                    as f32
                    / (1024.0 * 1024.0)
                    + 1024.0;
        if ram_fits && (choice.gpu_layers == 0 || resident || safe_layers >= choice.gpu_layers) {
            result.refinement_model =
                (choice.refinement_model != "none").then_some(choice.refinement_model);
            result.refinement_gpu_layers = choice.gpu_layers;
            result.refinement_device = if choice.gpu_layers == 0 {
                "cpu"
            } else {
                "auto"
            }
            .into();
            result.context_size = choice.context_size;
            result.reasons.push(reason("calibrated_runtime", format!("Measured co-resident winner for {}. Files, drivers and runtime match the saved quality test.", settings.language)));
        }
    }
    // Live status is authoritative while a model is resident. Free memory at
    // this point excludes its weights and cannot be used to select its replacement.
    if engine.loaded {
        if let Some((model, _, _)) = ctx.last_asr_load.read().as_ref() {
            result.asr_model = model.clone();
        }
        if !engine.device.is_empty() {
            result.asr_device = engine.device;
        }
        if !engine.precision.is_empty() {
            result.asr_precision = engine.precision;
        }
        result.reasons.retain(|r| r.code != "asr_selection");
    }
    if ctx.flow_runtime.status_ready() && result.refinement_model == ctx.flow_runtime.active_model()
    {
        result.refinement_gpu_layers = ctx.flow_runtime.active_n_gpu_layers().unwrap_or(0);
        result.refinement_device = if ctx.flow_runtime.active_mode().is_some_and(|m| m.is_gpu()) {
            "gpu".into()
        } else {
            "cpu".into()
        };
    }
    result
}

pub fn current_settings(ctx: &crate::context::AppContext) -> AppSettings {
    let settings = ctx.settings_store.get();
    effective_settings(&settings, &current_plan(ctx))
}

pub(crate) fn refinement_is_resident(
    ctx: &crate::context::AppContext,
    settings: &AppSettings,
) -> bool {
    ctx.flow_runtime.status_ready()
        && ctx.flow_runtime.active_model().as_deref()
            == Some(settings.resolve_intent().flow_model.as_str())
}

pub fn has_installed_asr(ctx: &crate::context::AppContext, settings: &AppSettings) -> bool {
    let installed = |id: &str| {
        ctx.model_manager
            .is_installed(&crate::model::manager::runtime_model_id(
                id,
                &settings.asr.runtime,
            ))
    };
    if settings.preset == "custom" || settings.asr.model == "phonon-2" {
        installed(&settings.asr.model)
    } else {
        ["0.6b", "1.7b"].iter().any(|id| installed(id))
    }
}
