//! Milestone 1 / Task 8: `resolve_profile` — a pure hardware-to-configuration
//! function.
//!
//! No I/O, no globals, no clock. Everything it needs arrives as arguments, so
//! every hardware class in the validation matrix can be exercised with a
//! fixture and the answer is reproducible.
//!
//! This subsumes and generalises the Python runtime's `build_attempts` ladder:
//! that function decided precision for one model on one device, this one
//! decides the whole profile (ASR model, device, precision, refinement model,
//! device, offload) and still emits an ordered attempt ladder for the loader to
//! walk.

use serde::{Deserialize, Serialize};

use super::manifest::{
    asr_manifest, refinement_manifest, Device, MeasuredPeaks, ModelManifest, Precision, ASR_MODELS,
    REFINEMENT_MODELS,
};
use crate::capability::probe::Capabilities;

/// User-facing performance intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// Pick for the detected hardware.
    Auto,
    /// Lowest latency. No refinement.
    Fast,
    /// Default: refinement on, latency still the priority.
    Balanced,
    /// Lowest word error rate, latency secondary.
    Accurate,
    /// Every field comes from the user. The resolver must never overwrite this.
    Custom,
}

impl Preset {
    pub fn as_str(&self) -> &'static str {
        match self {
            Preset::Auto => "auto",
            Preset::Fast => "fast",
            Preset::Balanced => "balanced",
            Preset::Accurate => "accurate",
            Preset::Custom => "custom",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "fast" => Preset::Fast,
            "balanced" => Preset::Balanced,
            "accurate" => Preset::Accurate,
            "custom" => Preset::Custom,
            _ => Preset::Auto,
        }
    }
}

/// Explicit user choices. `None` means "let the resolver decide".
///
/// A `Custom` preset with empty overrides is still `Custom`: the distinction is
/// carried by the preset, not by whether any field happens to be set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileOverrides {
    pub asr_model: Option<String>,
    pub asr_device: Option<Device>,
    pub asr_precision: Option<Precision>,
    pub refinement_model: Option<String>,
    pub refinement_device: Option<Device>,
    /// Explicit `--n-gpu-layers`. `Some(0)` forces CPU execution of the
    /// refinement model, which is different from `None`.
    pub refinement_gpu_layers: Option<u32>,
    /// Forced language, or `None` for the preset default.
    pub language: Option<String>,
    /// Enable chunk-on-silence regardless of the measured RTF. Escape hatch for
    /// benchmarking; the resolver still records why it would have said no.
    pub force_streaming: Option<bool>,
}

impl ProfileOverrides {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// One rung of the load ladder: try this, and if it fails move to the next.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LoadAttempt {
    pub device: Device,
    pub precision: Precision,
    /// Estimated or measured peak for this rung, whichever is known.
    pub expected_peak_mb: f32,
    /// `true` when this rung came from a measurement rather than an estimate.
    pub measured: bool,
}

/// Why the resolver chose what it chose.
///
/// Carried in the result rather than logged, so the settings UI can explain a
/// decision the user did not make and the benchmark cache can record it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileReason {
    /// Stable machine-readable code, so UI copy and tests never depend on the
    /// wording of `detail`.
    pub code: String,
    pub detail: String,
}

impl ProfileReason {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            detail: detail.into(),
        }
    }
}

/// A resolver verdict that must be shown to the user rather than worked around.
///
/// Distinct from [`ProfileReason`]: a reason explains a choice the resolver
/// made, while an error names a requested configuration that cannot be loaded
/// at all. `Custom` produces these instead of a corrected answer, because
/// silently rewriting an explicit choice is what the settings page must never
/// do — a substitution it cannot see is the CPU-fallback bug all over again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileError {
    /// Stable machine-readable code, so UI copy and tests never depend on the
    /// wording of `detail`.
    pub code: String,
    pub detail: String,
    /// The value the user asked for, e.g. `"int4"`.
    pub requested: String,
    /// The values that would be accepted, so the UI can offer a real fix
    /// instead of parroting the rejection.
    pub supported: Vec<String>,
}

impl ProfileError {
    fn new(
        code: &'static str,
        detail: impl Into<String>,
        requested: impl Into<String>,
        supported: Vec<String>,
    ) -> Self {
        Self {
            code: code.to_string(),
            detail: detail.into(),
            requested: requested.into(),
            supported,
        }
    }
}

/// The resolved configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedProfile {
    pub preset: Preset,
    pub asr_model: String,
    pub asr_device: Device,
    pub asr_precision: Precision,
    /// Ordered rungs for the ASR loader.
    pub asr_attempts: Vec<LoadAttempt>,
    /// `None` means ASR-only: no refinement model, no `llama-server`, no
    /// download. A first-class mode, not a failure.
    pub refinement_model: Option<String>,
    pub refinement_device: Device,
    /// `--n-gpu-layers` for `llama-server`. `0` means pure CPU.
    pub refinement_gpu_layers: u32,
    /// Forced language, or `"auto"`.
    pub language: String,
    /// Whether chunk-on-silence may run.
    pub streaming_enabled: bool,
    /// Threads inference may use, leaving headroom for audio, UI and the OS.
    pub inference_threads: usize,
    /// Explanations, in the order they were decided.
    pub reasons: Vec<ProfileReason>,
    /// Requests the resolver could not honour at all — e.g. a `Custom`
    /// precision the model does not support. These are errors, not reasons:
    /// the configuration was left as asked, and loading it will fail, so the
    /// UI must show them rather than treat the profile as ready.
    #[serde(default)]
    pub errors: Vec<ProfileError>,
}

impl ResolvedProfile {
    pub fn is_asr_only(&self) -> bool {
        self.refinement_model.is_none()
    }

    pub fn reason_codes(&self) -> Vec<&str> {
        self.reasons.iter().map(|r| r.code.as_str()).collect()
    }

    pub fn has_reason(&self, code: &str) -> bool {
        self.reasons.iter().any(|r| r.code == code)
    }

    pub fn has_error(&self, code: &str) -> bool {
        self.errors.iter().any(|e| e.code == code)
    }
}

/// Lookup for peaks already measured on this machine.
///
/// A trait object would drag I/O into the signature; a plain closure keeps the
/// resolver pure while still letting the benchmark cache feed it.
pub type MeasuredLookup<'a> = &'a dyn Fn(&str, Device, Precision) -> MeasuredPeaks;

/// No measurements available.
pub fn no_measurements(_: &str, _: Device, _: Precision) -> MeasuredPeaks {
    MeasuredPeaks::default()
}

/// The structured verdict for a forced precision the model cannot load.
///
/// `supported` should come from [`ModelManifest::supported_precisions_on`] so
/// the list reflects the device, not just the model — the CPU path only ever
/// offers fp32.
fn unsupported_precision_error(
    manifest: &ModelManifest,
    device: Device,
    precision: Precision,
    supported: &[Precision],
) -> ProfileError {
    let supported_list: Vec<String> = supported.iter().map(|p| p.as_str().to_string()).collect();
    ProfileError::new(
        "asr_precision_unsupported",
        format!(
            "{} does not support {} on {}; it supports {}. The requested \
             configuration cannot be loaded as specified.",
            manifest.label,
            precision.as_str(),
            device.as_str(),
            supported_list.join(", "),
        ),
        precision.as_str(),
        supported_list,
    )
}

/// VRAM that must stay free for the display, the compositor and other apps.
///
/// A floor on *measured free* VRAM, never a subtraction from the nominal total.
/// The percentage term matters at both ends: a flat floor over-reserves on a
/// hybrid-graphics laptop where the iGPU drives the display and the dGPU is
/// doing nothing else, and under-reserves on a desktop driving several
/// high-resolution monitors where WDDM plus compositing can exceed 1 GiB.
///
/// The single-GPU floor was 768 MiB, which is too much on a small card: it
/// refused a configuration that was then measured working. On a 4 GiB RTX 2050
/// with the desktop holding ~1899 MiB, 768 + 256 MiB of margin left a 1040 MiB
/// budget and admitted no ASR model at all, so ASR fell to the CPU and took
/// 17546 ms to transcribe 7.3s of audio. The 0.6B model loaded on that same GPU
/// in that same state using 1362 MiB, left 704 MiB free, transcribed in 2887 ms,
/// and the sidecar's spill check reported none. 704 MiB of remaining headroom is
/// therefore demonstrably safe on this class of machine, so the floor is 512.
pub fn vram_reserve_mb(caps: &Capabilities) -> f32 {
    let total = caps.total_vram_mb();
    if total <= 0.0 {
        return 0.0;
    }
    let proportional = total * 0.12;
    // When another adapter is present it is almost certainly driving the
    // display, so this device does not need to hold back as much.
    let floor = if caps.gpus.len() > 1 { 384.0 } else { 512.0 };
    proportional.max(floor).min(total * 0.5)
}

/// Transient headroom for allocator spikes during load and the first inference.
const TRANSIENT_MARGIN_MB: f32 = 256.0;

/// System RAM held back from a CPU model load for the app, the refinement
/// runtime and the OS.
const CPU_RAM_HEADROOM_MB: f32 = 1024.0;

/// RTF at or below which chunk-on-silence is worth doing.
///
/// Above it, segments queue behind the speaker: the work does not finish before
/// the next segment arrives, so nothing is gained and the GPU is busy
/// mid-utterance for no benefit.
pub const STREAMING_RTF_CEILING: f32 = 0.5;

/// Resolve a complete configuration from measured hardware and user intent.
///
/// Pure: same inputs, same output, no I/O.
pub fn resolve_profile(
    caps: &Capabilities,
    preset: Preset,
    overrides: &ProfileOverrides,
    measured: MeasuredLookup<'_>,
) -> ResolvedProfile {
    let mut reasons = Vec::new();
    let mut errors = Vec::new();

    // --- ASR device -------------------------------------------------------
    let cuda_usable = caps.can_use_cuda();
    let asr_device = match overrides.asr_device {
        Some(Device::Cuda) if !cuda_usable => {
            // Honour the request but record that it cannot be served, so the UI
            // can say so instead of silently running on the CPU.
            reasons.push(ProfileReason::new(
                "asr_cuda_requested_unavailable",
                if caps.cuda.gpu_present_but_unusable() {
                    "CUDA was requested and a GPU is present, but torch cannot use it \
                     (most likely a CPU-only torch build). Falling back to CPU."
                        .to_string()
                } else {
                    "CUDA was requested but no CUDA-capable GPU is available. \
                     Falling back to CPU."
                        .to_string()
                },
            ));
            Device::Cpu
        }
        Some(device) => {
            reasons.push(ProfileReason::new(
                "asr_device_user_choice",
                format!("ASR device set to {} by the user.", device.as_str()),
            ));
            device
        }
        None if cuda_usable => Device::Cuda,
        None => {
            reasons.push(ProfileReason::new(
                "asr_device_cpu",
                if caps.cuda.gpu_present_but_unusable() {
                    "A GPU is present but CUDA is unavailable to the ASR runtime, \
                     so ASR runs on the CPU."
                        .to_string()
                } else {
                    "No CUDA-capable GPU detected, so ASR runs on the CPU.".to_string()
                },
            ));
            Device::Cpu
        }
    };

    // --- ASR budget -------------------------------------------------------
    let reserve = vram_reserve_mb(caps);
    let free_vram = caps.free_vram_mb();
    let asr_budget_mb = if asr_device.is_gpu() {
        (free_vram - reserve - TRANSIENT_MARGIN_MB).max(0.0)
    } else {
        // On CPU the constraint is available RAM. Unlike VRAM this is not a hard
        // wall — the OS will page — so the budget is generous and simply holds
        // back enough for the app, the refinement runtime and the OS.
        (caps.ram.available_mb - CPU_RAM_HEADROOM_MB).max(0.0)
    };

    // --- ASR model --------------------------------------------------------
    let asr_model = match overrides.asr_model.as_deref().and_then(asr_manifest) {
        Some(manifest) => {
            reasons.push(ProfileReason::new(
                "asr_model_user_choice",
                format!("ASR model set to {} by the user.", manifest.id),
            ));
            manifest
        }
        None => {
            let chosen = pick_asr_model(preset, asr_device, asr_budget_mb, caps, measured);
            reasons.push(ProfileReason::new(
                "asr_model_auto",
                format!(
                    "Selected {} for the {} preset with {:.0} MB of budget.",
                    chosen.id,
                    preset.as_str(),
                    asr_budget_mb
                ),
            ));
            chosen
        }
    };

    // --- ASR precision ladder --------------------------------------------
    // An explicit precision the model cannot load used to drop out of the
    // ladder entirely: the resolver echoed it back as `asr_precision`, the
    // sidecar tried it, failed, and fell back to the CPU — a silent 6x
    // slowdown misreported as a memory problem. Now:
    //
    // * under `Custom`, the request is echoed back with a structured error —
    //   Custom is never overwritten, so the verdict is the error, not a
    //   substituted precision;
    // * under every other preset the resolver may adjust, so it steps to a
    //   precision the model actually supports and records the correction.
    let supported_precisions = asr_model.supported_precisions_on(asr_device);
    let unsupported_forced = overrides
        .asr_precision
        .filter(|p| !supported_precisions.contains(p));
    let custom_precision_error = unsupported_forced.is_some() && preset == Preset::Custom;
    let effective_forced = match unsupported_forced {
        // Relax to the automatic ladder, which only walks supported rungs.
        Some(_) if !custom_precision_error => None,
        _ => overrides.asr_precision,
    };
    let asr_attempts = build_attempts(
        asr_model,
        asr_device,
        asr_budget_mb,
        effective_forced,
        measured,
    );
    let fallback_precision = if asr_device.is_gpu() {
        Precision::Bf16
    } else {
        Precision::Fp32
    };
    let supported_list: Vec<String> = supported_precisions
        .iter()
        .map(|p| p.as_str().to_string())
        .collect();
    let asr_precision = match overrides.asr_precision {
        None => asr_attempts
            .first()
            .map(|a| a.precision)
            .unwrap_or(fallback_precision),
        Some(requested) => match unsupported_forced {
            None => requested,
            Some(_) if custom_precision_error => {
                errors.push(unsupported_precision_error(
                    asr_model,
                    asr_device,
                    requested,
                    &supported_precisions,
                ));
                requested
            }
            Some(_) => {
                let corrected = asr_attempts
                    .first()
                    .map(|a| a.precision)
                    .unwrap_or(fallback_precision);
                reasons.push(ProfileReason::new(
                    "asr_precision_adjusted",
                    format!(
                        "{} is not supported by {} on {}; using {} instead \
                         (supported: {}).",
                        requested.as_str(),
                        asr_model.label,
                        asr_device.as_str(),
                        corrected.as_str(),
                        supported_list.join(", "),
                    ),
                ));
                corrected
            }
        },
    };
    // "Nothing fits" is only meaningful when the ladder actually ran the
    // supported rungs; when the Custom error fired, the error is the verdict.
    if asr_attempts.is_empty() && !custom_precision_error {
        reasons.push(ProfileReason::new(
            "asr_no_viable_precision",
            format!(
                "No precision of {} fits the {:.0} MB budget on {}.",
                asr_model.id,
                asr_budget_mb,
                asr_device.as_str()
            ),
        ));
    }
    if let Some(requested) = overrides.asr_precision {
        if unsupported_forced.is_none() && asr_attempts.is_empty() {
            reasons.push(ProfileReason::new(
                "asr_precision_forced_over_budget",
                format!(
                    "{} was requested for {} but its estimated {:.0} MB peak exceeds the \
                     {:.0} MB budget. Honouring the choice; it may fail to load or spill.",
                    requested.as_str(),
                    asr_model.id,
                    if asr_device.is_gpu() {
                        asr_model.estimated_vram_mb(requested)
                    } else {
                        asr_model.estimated_cpu_ram_mb()
                    },
                    asr_budget_mb
                ),
            ));
        }
    }

    // --- Refinement -------------------------------------------------------
    let asr_peak = attempt_peak(&asr_attempts, asr_precision, asr_model, asr_device);
    let remaining_vram = if asr_device.is_gpu() {
        (free_vram - reserve - asr_peak).max(0.0)
    } else {
        (free_vram - reserve).max(0.0)
    };

    let (refinement_model, refinement_device, refinement_gpu_layers) = resolve_refinement(
        preset,
        overrides,
        caps,
        remaining_vram,
        measured,
        &mut reasons,
    );

    // --- Streaming --------------------------------------------------------
    let measured_rtf = measured(asr_model.id, asr_device, asr_precision).rtf;
    let streaming_enabled = match overrides.force_streaming {
        Some(forced) => {
            reasons.push(ProfileReason::new(
                "streaming_forced",
                format!(
                    "Chunk-on-silence {} by an explicit override.",
                    if forced { "enabled" } else { "disabled" }
                ),
            ));
            forced
        }
        None => match measured_rtf {
            Some(rtf) if rtf <= STREAMING_RTF_CEILING => {
                reasons.push(ProfileReason::new(
                    "streaming_enabled",
                    format!(
                        "Measured RTF {rtf:.2} is at or below {STREAMING_RTF_CEILING:.2}, \
                         so segments finish faster than they arrive."
                    ),
                ));
                true
            }
            Some(rtf) => {
                reasons.push(ProfileReason::new(
                    "streaming_disabled_slow",
                    format!(
                        "Measured RTF {rtf:.2} exceeds {STREAMING_RTF_CEILING:.2}; \
                         segments would queue behind the speaker, so \
                         chunk-on-silence is a net loss."
                    ),
                ));
                false
            }
            None => {
                reasons.push(ProfileReason::new(
                    "streaming_disabled_unmeasured",
                    "RTF has not been measured on this machine yet, so \
                     chunk-on-silence stays off until it has been."
                        .to_string(),
                ));
                false
            }
        },
    };

    // --- Language ---------------------------------------------------------
    let language = overrides
        .language
        .clone()
        .filter(|l| !l.trim().is_empty())
        .unwrap_or_else(|| "en".to_string());
    if !asr_model.supports_language(&language) {
        reasons.push(ProfileReason::new(
            "language_unvalidated",
            format!(
                "{} has not been validated for '{}'; routing or a different model is needed.",
                asr_model.id, language
            ),
        ));
    }

    ResolvedProfile {
        preset,
        asr_model: asr_model.id.to_string(),
        asr_device,
        asr_precision,
        asr_attempts,
        refinement_model,
        refinement_device,
        refinement_gpu_layers,
        language,
        streaming_enabled,
        inference_threads: caps.cpu.inference_threads(),
        reasons,
        errors,
    }
}

/// Peak for the chosen precision, preferring a measurement.
fn attempt_peak(
    attempts: &[LoadAttempt],
    precision: Precision,
    manifest: &ModelManifest,
    device: Device,
) -> f32 {
    attempts
        .iter()
        .find(|a| a.precision == precision)
        .map(|a| a.expected_peak_mb)
        .unwrap_or_else(|| {
            if device.is_gpu() {
                manifest.estimated_vram_mb(precision)
            } else {
                manifest.estimated_cpu_ram_mb()
            }
        })
}

/// Largest ASR model whose best precision fits the budget, biased by preset.
fn pick_asr_model(
    preset: Preset,
    device: Device,
    budget_mb: f32,
    caps: &Capabilities,
    measured: MeasuredLookup<'_>,
) -> &'static ModelManifest {
    let smallest = &ASR_MODELS[0];

    // Fast always takes the smallest model: it is the lowest-latency option and
    // the preset's whole point is latency.
    if matches!(preset, Preset::Fast | Preset::Auto) {
        return smallest;
    }

    // On CPU the binding constraint is throughput, not memory. A 16 GB laptop
    // has room for the 1.7B model at fp32 and would still transcribe several
    // times slower than real time, which is unusable for dictation and would
    // also disable chunk-on-silence. Memory availability is not permission.
    if !device.is_gpu() {
        return smallest;
    }

    // Accurate and Balanced prefer the largest model that fits. Walk from the
    // largest down so a machine with headroom gets the better model.
    let mut candidates: Vec<&'static ModelManifest> = ASR_MODELS.iter().collect();
    candidates.sort_by_key(|m| std::cmp::Reverse(m.params));
    for manifest in candidates {
        // Fitting the *current* budget is not sufficient to choose a model
        // automatically. Free VRAM fluctuates with whatever else is on the
        // desktop, and a model admitted at a quiet moment has to survive a busy
        // one, plus KV growth over a long dictation. So an automatic choice also
        // has to be a model the card could hold at full precision.
        //
        // Concretely: a 4 GiB card must not auto-select the 1.7B model, whose
        // BF16 footprint is ~4.7 GB. Its INT8 footprint does fit a momentarily
        // quiet 4 GiB card, and admitting it on that basis is how a machine ends
        // up spilling — or, once the allocation fails, falling back to the CPU
        // and taking 17546 ms per dictation instead of 2887 ms. `select_asr_load`
        // applies no such ceiling, because there the user has named a model
        // explicitly and is owed that choice when it genuinely fits.
        let card_could_hold_it =
            manifest.estimated_vram_mb(Precision::Bf16) <= caps.total_vram_mb();
        if !card_could_hold_it {
            continue;
        }
        let fits = build_attempts(manifest, device, budget_mb, None, measured)
            .iter()
            .any(|a| a.device == device);
        if fits {
            return manifest;
        }
    }
    smallest
}

/// What the loader should actually load, versus what was asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct AsrSelection {
    /// Manifest id to load (`"0.6b"` / `"1.7b"`).
    pub model_id: &'static str,
    pub device: Device,
    pub precision: Precision,
    /// Set when the selection differs from the request, so the UI can say why
    /// instead of the user discovering it as unexplained slowness.
    pub downgrade: Option<String>,
    /// Set when the request cannot be loaded at all — e.g. a `Custom`
    /// precision the model does not support.
    ///
    /// The load must not proceed on this selection: `model_id`, `device` and
    /// `precision` still echo the request so the UI can show exactly what was
    /// rejected. Before this field existed the ladder silently fell back to
    /// the CPU, which is precisely the bug it now prevents.
    pub error: Option<ProfileError>,
}

/// Choose the ASR load that will actually be fast on this machine right now.
///
/// Two inputs govern the ceiling, and they mean different things:
///
/// * `preset` is the performance intent — how much latency the user is willing
///   to trade for accuracy. `Fast` takes the smallest model outright, `Accurate`
///   reaches for the largest that fits, `Auto`/`Balanced` respect the model named
///   in Settings, and `Custom` means the user is driving and their choice is not
///   second-guessed beyond what the hardware can physically load.
/// * `requested_model` is the model named in Settings, treated as a ceiling
///   rather than a command.
///
/// The reason the Settings choice is only a ceiling is a measured cliff: on a
/// 4 GB card with ~1.9 GB held by the desktop, the 1.7B model does not fit, and
/// the only fallback the runtime had was GPU -> CPU. That took **17546 ms** to
/// transcribe 7.3s of audio, against **2887 ms** for the 0.6B model on the GPU —
/// a 6x latency penalty to gain an accuracy difference nowhere near 6x. Stepping
/// the model down keeps the work on the GPU, which is the decision that matters.
///
/// Only ever steps *down* from the ceiling, and only among models that are
/// actually installed. Every deviation is reported in `downgrade`.
#[allow(clippy::too_many_arguments)]
pub fn select_asr_load_for_runtime(
    runtime: &str,
    preset: Preset,
    requested_model: &str,
    requested_device: &str,
    requested_precision: &str,
    installed: &dyn Fn(&str) -> bool,
    caps: &Capabilities,
    measured: MeasuredLookup<'_>,
) -> AsrSelection {
    if runtime != "native" {
        return select_asr_load(
            preset,
            requested_model,
            requested_device,
            requested_precision,
            installed,
            caps,
            measured,
        );
    }
    let models = super::manifest::NATIVE_ASR_MODELS;
    let requested = super::manifest::native_asr_manifest(requested_model).unwrap_or(&models[0]);
    let device = if requested_device == "cpu" || caps.primary_gpu().is_none() {
        Device::Cpu
    } else {
        Device::Vulkan
    };

    // Same contract as the Python path below: a forced precision the model
    // cannot load is a structured error under Custom, an explained correction
    // under the presets that may adjust.
    let forced_precision = Precision::parse(requested_precision);
    let supported = requested.supported_precisions_on(device);
    let unsupported_forced = forced_precision.filter(|p| !supported.contains(p));
    if let Some(p) = unsupported_forced.filter(|_| preset == Preset::Custom) {
        return AsrSelection {
            model_id: requested.id,
            device,
            precision: p,
            downgrade: None,
            error: Some(unsupported_precision_error(
                requested, device, p, &supported,
            )),
        };
    }

    let ceiling = match preset {
        Preset::Fast | Preset::Auto => models[0].params,
        Preset::Accurate => models[1].params,
        _ => requested.params,
    };
    let budget = (caps.free_vram_mb() - vram_reserve_mb(caps) - TRANSIENT_MARGIN_MB).max(0.0);
    let chosen = models.iter().rev().find(|m| {
        m.params <= ceiling
            && installed(m.id)
            && (device == Device::Cpu || m.estimated_vram_mb(Precision::Int8) <= budget)
    });
    let (chosen, device) = match chosen {
        Some(model) => (model, device),
        None => (
            models.iter().find(|m| installed(m.id)).unwrap_or(requested),
            Device::Cpu,
        ),
    };
    let mut notes: Vec<String> = Vec::new();
    if chosen.id != requested.id {
        notes.push(format!(
            "Running {} instead of {} to fit the native ASR memory budget.",
            chosen.label, requested.label
        ));
    }
    if let Some(p) = unsupported_forced {
        notes.push(format!(
            "{} is not supported by {}; the native GGUF weights run at int8 \
             (supported: {}).",
            p.as_str(),
            requested.label,
            supported
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    AsrSelection {
        model_id: chosen.id,
        device,
        precision: Precision::Int8,
        downgrade: (!notes.is_empty()).then(|| notes.join(" ")),
        error: None,
    }
}

pub fn select_asr_load(
    preset: Preset,
    requested_model: &str,
    requested_device: &str,
    requested_precision: &str,
    installed: &dyn Fn(&str) -> bool,
    caps: &Capabilities,
    measured: MeasuredLookup<'_>,
) -> AsrSelection {
    let forced_precision = Precision::parse(requested_precision);
    let requested = asr_manifest(requested_model).unwrap_or(&ASR_MODELS[0]);
    let wants_cpu = requested_device.trim().eq_ignore_ascii_case("cpu");
    let cuda_usable = caps.can_use_cuda();

    // The device the request would actually land on, for support checks.
    let effective_device = if wants_cpu || !cuda_usable {
        Device::Cpu
    } else {
        Device::Cuda
    };
    // A forced precision the requested model cannot load used to produce an
    // empty ladder, which fell through to the CPU fallback below with a
    // misleading "not enough VRAM" note — the silent CPU drop this fix exists
    // to remove. Under Custom it is a structured error; under the presets
    // that may adjust, the precision is relaxed and the correction reported.
    let supported = requested.supported_precisions_on(effective_device);
    let unsupported_forced = forced_precision.filter(|p| !supported.contains(p));
    if let Some(p) = unsupported_forced.filter(|_| preset == Preset::Custom) {
        return AsrSelection {
            model_id: requested.id,
            device: effective_device,
            precision: p,
            downgrade: None,
            error: Some(unsupported_precision_error(
                requested,
                effective_device,
                p,
                &supported,
            )),
        };
    }

    let installed_by_size = |ascending: bool| -> Vec<&'static ModelManifest> {
        let mut models: Vec<&'static ModelManifest> =
            ASR_MODELS.iter().filter(|m| installed(m.id)).collect();
        models.sort_by_key(|m| m.params);
        if !ascending {
            models.reverse();
        }
        models
    };

    // Smallest installed model, the last resort on either device.
    let smallest_installed = || -> &'static ModelManifest {
        installed_by_size(true)
            .into_iter()
            .next()
            .unwrap_or(requested)
    };

    if wants_cpu || !cuda_usable {
        // CPU is bound by throughput, not memory, so the largest model that
        // *fits* is the wrong question — a big model on CPU is slow whether or
        // not the RAM is there. Take the smallest installed model.
        let chosen = if preset == Preset::Custom {
            requested
        } else {
            smallest_installed()
        };
        let mut notes: Vec<String> = Vec::new();
        if chosen.id != requested.id {
            notes.push(format!(
                "Running {} instead of {} because ASR is on the CPU, \
                 where a larger model is slower without being more usable.",
                chosen.label, requested.label
            ));
        }
        if let Some(p) = unsupported_forced {
            notes.push(format!(
                "The CPU path never quantizes, so {} is dropped and the \
                 model runs at fp32.",
                p.as_str()
            ));
        }
        return AsrSelection {
            model_id: chosen.id,
            device: Device::Cpu,
            precision: Precision::Fp32,
            downgrade: (!notes.is_empty()).then(|| notes.join(" ")),
            error: None,
        };
    }

    let budget_mb = (caps.free_vram_mb() - vram_reserve_mb(caps) - TRANSIENT_MARGIN_MB).max(0.0);

    // The preset sets the ceiling; Settings sets it for the presets that defer.
    let (ceiling, preset_reason): (&'static ModelManifest, Option<&'static str>) = match preset {
        Preset::Fast => (
            smallest_installed(),
            Some("the Fast profile prioritises latency over accuracy"),
        ),
        Preset::Auto => (
            smallest_installed(),
            Some("the Auto profile prioritises throughput over model size"),
        ),
        Preset::Accurate => (
            installed_by_size(false)
                .into_iter()
                .next()
                .unwrap_or(requested),
            Some("the Accurate profile prefers the most capable model that fits"),
        ),
        Preset::Balanced | Preset::Custom => (requested, None),
    };

    // Largest installed model at or below the ceiling that fits the budget.
    let mut candidates: Vec<&'static ModelManifest> = ASR_MODELS
        .iter()
        .filter(|m| m.params <= ceiling.params && installed(m.id))
        .collect();
    candidates.sort_by_key(|m| std::cmp::Reverse(m.params));

    for manifest in candidates {
        // A forced precision this candidate cannot load is relaxed to the
        // automatic ladder rather than producing an empty one — which is how
        // "int4 on the 0.6B model" used to read as a VRAM problem and drop
        // ASR to the CPU. The correction is reported below.
        let forced_here = forced_precision.filter(|p| manifest.supports(Device::Cuda, *p));
        let attempts = build_attempts(manifest, Device::Cuda, budget_mb, forced_here, measured);
        if let Some(attempt) = attempts.first() {
            let mut notes: Vec<String> = Vec::new();
            if manifest.id != requested.id {
                notes.push(
                    if let Some(reason) = preset_reason.filter(|_| manifest.id == ceiling.id) {
                        // The preset, not the hardware, moved us off the Settings model.
                        format!(
                            "Running {} instead of {} because {reason}.",
                            manifest.label, requested.label
                        )
                    } else {
                        format!(
                            "Running {} instead of {}: only {:.0} MB of VRAM is free, \
                             and {} needs about {:.0} MB. A smaller model on the GPU is \
                             far faster than a larger one on the CPU.",
                            manifest.label,
                            requested.label,
                            budget_mb,
                            requested.label,
                            requested
                                .estimated_vram_mb(forced_precision.unwrap_or(Precision::Int8)),
                        )
                    },
                );
            }
            if forced_here != forced_precision {
                // `forced_here` is None exactly when the requested precision
                // is unsupported on this model; say what replaced it.
                let p = forced_precision.expect("forced_here differs");
                notes.push(format!(
                    "{} is not supported by {} on {}; using {} instead.",
                    p.as_str(),
                    manifest.label,
                    Device::Cuda.as_str(),
                    attempt.precision.as_str(),
                ));
            }
            return AsrSelection {
                model_id: manifest.id,
                device: Device::Cuda,
                precision: attempt.precision,
                downgrade: (!notes.is_empty()).then(|| notes.join(" ")),
                error: None,
            };
        }
    }

    // Nothing fits on the GPU at all. Fall back to CPU with the smallest
    // installed model rather than the requested one, for the same
    // throughput reason as above.
    let chosen = smallest_installed();
    let mut fallback_note = format!(
        "Running {} on the CPU: only {:.0} MB of VRAM is free, which is not \
         enough for any installed model.",
        chosen.label, budget_mb
    );
    if let Some(p) = unsupported_forced {
        fallback_note.push_str(&format!(
            " {} is not supported by {}; the CPU path runs fp32.",
            p.as_str(),
            requested.label
        ));
    }
    AsrSelection {
        model_id: chosen.id,
        device: Device::Cpu,
        precision: Precision::Fp32,
        downgrade: Some(fallback_note),
        error: None,
    }
}

/// Ordered load ladder for one model on one device.
///
/// Generalises the Python `build_attempts`: rungs that cannot fit the measured
/// budget are omitted rather than attempted and failed, and a measured peak
/// always overrides the estimate.
pub fn build_attempts(
    manifest: &ModelManifest,
    device: Device,
    budget_mb: f32,
    forced_precision: Option<Precision>,
    measured: MeasuredLookup<'_>,
) -> Vec<LoadAttempt> {
    let mut attempts = Vec::new();

    if !device.is_gpu() {
        // The CPU path never quantizes: weight-only quantization on CPU costs
        // RAM without buying throughput.
        let peaks = measured(manifest.id, Device::Cpu, Precision::Fp32);
        let peak = peaks
            .ram_mb
            .unwrap_or_else(|| manifest.estimated_cpu_ram_mb());
        // The budget applies on CPU too. Reporting a rung that does not fit is
        // how the resolver ended up handing an 8 GB laptop the 1.7B model.
        if peak <= budget_mb {
            attempts.push(LoadAttempt {
                device: Device::Cpu,
                precision: Precision::Fp32,
                expected_peak_mb: peak,
                measured: peaks.ram_mb.is_some(),
            });
        }
        return attempts;
    }

    // GPU: order by expected quality at equal fit. BF16 first when it fits,
    // because weight-only INT8 at batch size 1 pays dequantization overhead
    // without a bandwidth win. INT8 exists to make a model fit at all.
    //
    // INT4 is deliberately absent from the automatic ladder. Its accuracy cost
    // has not been measured, so selecting it on the user's behalf would trade
    // transcription quality for memory without anyone agreeing to that. It
    // remains available as an explicit choice.
    const AUTO_ORDER: [Precision; 2] = [Precision::Bf16, Precision::Int8];
    let forced = forced_precision.map(|p| [p]);
    let order: &[Precision] = match &forced {
        Some(single) => single.as_slice(),
        None => AUTO_ORDER.as_slice(),
    };

    // Keep each rung's measured RTF alongside it, for ordering below.
    let mut rtfs: Vec<Option<f32>> = Vec::new();
    for &precision in order {
        if !manifest.supports(device, precision) {
            continue;
        }
        let peaks = measured(manifest.id, device, precision);
        let peak = peaks
            .vram_mb
            .unwrap_or_else(|| manifest.estimated_vram_mb(precision));
        if peak <= budget_mb {
            rtfs.push(peaks.rtf);
            attempts.push(LoadAttempt {
                device,
                precision,
                expected_peak_mb: peak,
                measured: peaks.vram_mb.is_some(),
            });
        }
    }
    // A measurement outranks an estimate — and the BF16-first order above is
    // itself an estimate ("INT8 pays dequantization overhead at batch size
    // 1"). When every viable rung has been timed on this hardware, the
    // measured fastest goes first; a partial table cannot rank the unmeasured
    // rungs, so it changes nothing.
    if attempts.len() > 1 && rtfs.iter().all(Option::is_some) {
        let mut ranked: Vec<(Option<f32>, LoadAttempt)> = rtfs.into_iter().zip(attempts).collect();
        ranked.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        attempts = ranked.into_iter().map(|(_, attempt)| attempt).collect();
    }
    attempts
}

fn resolve_refinement(
    preset: Preset,
    overrides: &ProfileOverrides,
    caps: &Capabilities,
    remaining_vram_mb: f32,
    measured: MeasuredLookup<'_>,
    reasons: &mut Vec<ProfileReason>,
) -> (Option<String>, Device, u32) {
    // Fast is defined as no refinement, and that is not a degradation.
    if preset == Preset::Fast && overrides.refinement_model.is_none() {
        reasons.push(ProfileReason::new(
            "refinement_off_fast_preset",
            "The Fast preset does not use the refinement model.".to_string(),
        ));
        return (None, Device::Cpu, 0);
    }

    let explicit_none = overrides
        .refinement_model
        .as_deref()
        .is_some_and(|id| id.eq_ignore_ascii_case("none"));
    if explicit_none {
        reasons.push(ProfileReason::new(
            "refinement_off_user_choice",
            "Refinement was turned off by the user.".to_string(),
        ));
        return (None, Device::Cpu, 0);
    }

    let manifest = match overrides
        .refinement_model
        .as_deref()
        .and_then(refinement_manifest)
    {
        Some(m) => m,
        None => match preset {
            Preset::Accurate => refinement_manifest("qwen3.5-2b").unwrap_or(&REFINEMENT_MODELS[0]),
            _ => &REFINEMENT_MODELS[0],
        },
    };

    // Device: an explicit choice wins; otherwise CPU unless the GPU has room
    // left after ASR *and* the offload has actually been measured as safe.
    if let Some(device) = overrides.refinement_device {
        let layers = overrides
            .refinement_gpu_layers
            .unwrap_or(if device.is_gpu() { 99 } else { 0 });
        reasons.push(ProfileReason::new(
            "refinement_device_user_choice",
            format!(
                "Refinement device set to {} with {} GPU layers by the user.",
                device.as_str(),
                layers
            ),
        ));
        return (Some(manifest.id.to_string()), device, layers);
    }

    if let Some(layers) = overrides.refinement_gpu_layers {
        let device = if layers == 0 {
            Device::Cpu
        } else {
            Device::Cuda
        };
        reasons.push(ProfileReason::new(
            "refinement_layers_user_choice",
            format!("Refinement pinned to {layers} GPU layers by the user."),
        ));
        return (Some(manifest.id.to_string()), device, layers);
    }

    if !caps.can_use_cuda() {
        reasons.push(ProfileReason::new(
            "refinement_cpu_no_cuda",
            "Refinement runs on the CPU because CUDA is unavailable.".to_string(),
        ));
        return (Some(manifest.id.to_string()), Device::Cpu, 0);
    }

    let measured_peak = measured(manifest.id, Device::Cuda, Precision::Int4).vram_mb;
    match measured_peak {
        Some(peak) if peak + TRANSIENT_MARGIN_MB <= remaining_vram_mb => {
            reasons.push(ProfileReason::new(
                "refinement_gpu_measured_fit",
                format!(
                    "Refinement fits the GPU: measured {peak:.0} MB against \
                     {remaining_vram_mb:.0} MB remaining after ASR."
                ),
            ));
            (Some(manifest.id.to_string()), Device::Cuda, 99)
        }
        Some(peak) => {
            reasons.push(ProfileReason::new(
                "refinement_cpu_measured_no_fit",
                format!(
                    "Refinement measured at {peak:.0} MB but only \
                     {remaining_vram_mb:.0} MB remains after ASR, so it runs on the CPU."
                ),
            ));
            (Some(manifest.id.to_string()), Device::Cpu, 0)
        }
        None => {
            // Never put the refiner on the GPU on the strength of an estimate.
            // Guessing here is how a configuration ends up spilled, and spill is
            // silent on WDDM. The partial-offload sweep measures it properly.
            reasons.push(ProfileReason::new(
                "refinement_cpu_unmeasured",
                format!(
                    "Refinement runs on the CPU until GPU offload has been measured; \
                     {remaining_vram_mb:.0} MB remains after ASR, but an unverified \
                     GPU load risks spilling into shared system memory."
                ),
            ));
            (Some(manifest.id.to_string()), Device::Cpu, 0)
        }
    }
}
