use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use crate::formatting::replacements::{CustomReplacements, ReplacementRule};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictionaryTerm {
    pub id: String,
    pub term: String,
    pub preferred_spelling: String,
    pub category: String,
}

// ---------------------------------------------------------------------------
// Task 9: ASR and refinement are independent placement decisions.
//
// One `compute_backend` field used to govern both, which made "run ASR on the
// GPU and refinement on the CPU" — the correct configuration on a 4 GB card —
// impossible to express. `flow_n_gpu_layers` was likewise the only refinement
// knob, so context size, keep-warm and the deadline had nowhere to live.
// ---------------------------------------------------------------------------

/// Where and how the speech model runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AsrSettings {
    /// Manifest id, e.g. `"0.6b"`.
    #[serde(default = "default_asr_model")]
    pub model: String,
    /// `"auto"` | `"cpu"` | `"cuda"`.
    #[serde(default = "default_device_auto")]
    pub device: String,
    /// `"auto"` | `"bf16"` | `"int8"` | `"int4"`.
    #[serde(default = "default_asr_precision")]
    pub precision: String,
    /// Keep weights resident between dictations.
    #[serde(default = "default_true")]
    pub keep_loaded: bool,
}

impl Default for AsrSettings {
    fn default() -> Self {
        Self {
            model: default_asr_model(),
            device: default_device_auto(),
            precision: default_asr_precision(),
            keep_loaded: true,
        }
    }
}

/// Where and how the Stage 2 refinement model runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefinementSettings {
    /// Manifest id, or `"none"` for ASR-only.
    #[serde(default = "default_flow_model")]
    pub model: String,
    /// `"auto"` | `"cpu"` | `"cuda"` | `"vulkan"`.
    #[serde(default = "default_device_auto")]
    pub device: String,
    /// `--n-gpu-layers`. `-1` = decide from `device`; `0` = force CPU; a
    /// positive value = offload exactly that many layers.
    #[serde(
        default = "default_flow_n_gpu_layers",
        deserialize_with = "de_i32_or_null"
    )]
    pub gpu_layers: i32,
    /// `--ctx-size`. Input longer than this is chunked or refused rather than
    /// silently truncated.
    #[serde(default = "default_context_size")]
    pub context_size: u32,
    /// Keep `llama-server` running between dictations. Turning this off trades
    /// a few hundred MB for a multi-second cold start on the critical path.
    #[serde(default = "default_true")]
    pub keep_warm: bool,
    /// Give up on refinement after this long and insert the deterministic text.
    ///
    /// A deadline that fires often is worse than no refinement, because output
    /// stops being reproducible between dictations.
    #[serde(default = "default_deadline_ms")]
    pub deadline_ms: u64,
}

impl Default for RefinementSettings {
    fn default() -> Self {
        Self {
            model: default_flow_model(),
            device: default_device_auto(),
            gpu_layers: default_flow_n_gpu_layers(),
            context_size: default_context_size(),
            keep_warm: true,
            deadline_ms: default_deadline_ms(),
        }
    }
}

/// Chunk-on-silence behaviour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamingSettings {
    /// `"auto"` (enable only when the measured RTF allows it), `"off"`, or
    /// `"chunk_on_silence"` to force it on.
    #[serde(default = "default_streaming_mode")]
    pub mode: String,
    /// A silence must last at least this long to be a segment boundary. The cut
    /// is placed at the midpoint, so no word spans it.
    #[serde(default = "default_silence_boundary_ms")]
    pub silence_boundary_ms: u64,
    /// Force a boundary after this much continuous speech, so a monologue with
    /// no pauses still produces progress.
    #[serde(default = "default_max_segment_seconds")]
    pub max_segment_seconds: u64,
}

impl Default for StreamingSettings {
    fn default() -> Self {
        Self {
            mode: default_streaming_mode(),
            silence_boundary_ms: default_silence_boundary_ms(),
            max_segment_seconds: default_max_segment_seconds(),
        }
    }
}

/// Memory admission policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryPolicySettings {
    /// VRAM to keep free for the display and other apps. `0` = derive it from
    /// the detected hardware, which is almost always the right answer.
    #[serde(default)]
    pub vram_reserve_mb: u32,
    /// Unload idle runtimes after this many seconds. `0` = never.
    #[serde(default)]
    pub unload_idle_after_seconds: u64,
    /// Allow refinement on the GPU at all. Off by default: an unmeasured GPU
    /// refinement load can spill into shared system memory, which is silent on
    /// Windows and roughly ten times slower.
    #[serde(default)]
    pub allow_gpu_refinement: bool,
}

impl Default for MemoryPolicySettings {
    /// `0` means "derive it", and GPU refinement is off until measured.
    fn default() -> Self {
        Self {
            vram_reserve_mb: 0,
            unload_idle_after_seconds: 0,
            allow_gpu_refinement: false,
        }
    }
}

fn default_device_auto() -> String {
    "auto".into()
}

fn default_true() -> bool {
    true
}

fn default_context_size() -> u32 {
    1024
}

fn default_deadline_ms() -> u64 {
    12_000
}

fn default_streaming_mode() -> String {
    "auto".into()
}

fn default_silence_boundary_ms() -> u64 {
    400
}

fn default_max_segment_seconds() -> u64 {
    12
}

fn default_preset() -> String {
    "auto".into()
}

fn de_i32_or_null<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<i32>::deserialize(deserializer)?.unwrap_or_else(default_flow_n_gpu_layers))
}

/// Rewrite a persisted document to the current schema, in place.
///
/// Runs on the raw JSON rather than on a deserialized `AppSettings`, because the
/// schema-2 keys it needs to read (`compute_backend`, `flow_n_gpu_layers`, …) no
/// longer exist on the struct. Deserializing first would discard exactly the
/// information the migration depends on.
///
/// Returns the version the document was found at.
pub fn migrate_document(value: &mut serde_json::Value) -> u32 {
    let Some(object) = value.as_object_mut() else {
        return CURRENT_SETTINGS_VERSION;
    };

    let found_version = object
        .get("settings_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32;

    if found_version < 3 {
        migrate_v2_to_v3(object);
    }
    if found_version < 4 {
        migrate_v3_to_v4(object);
    }

    object.insert(
        "settings_version".into(),
        serde_json::Value::from(CURRENT_SETTINGS_VERSION),
    );
    found_version
}

/// Flow-model ids that no longer exist, and what replaced them.
///
/// A retired id left in the document does not fail loudly: `flow_model_spec`
/// falls through to the `none` entry, so refinement is skipped with no error
/// anywhere. That is the same silent-degradation failure mode that had this
/// project shipping `rewriter_used = 0` for every dictation, so retiring a
/// model has to come with a migration.
const RETIRED_FLOW_MODELS: &[(&str, &str)] = &[
    // LFM2.5-1.2B was replaced by a smaller, faster model: 508 MB vs 697 MB on
    // disk, 2.0-2.5s to serve /health vs 5.1s.
    ("lfm2.5-1.2b", "qwen3.5-0.8b"),
];

fn replacement_for_retired_flow_model(id: &str) -> Option<&'static str> {
    let id = id.trim().to_ascii_lowercase();
    RETIRED_FLOW_MODELS
        .iter()
        .find(|(old, _)| *old == id)
        .map(|(_, new)| *new)
}

/// Schema 3 -> 4: repoint documents that still name a retired flow model.
fn migrate_v3_to_v4(object: &mut serde_json::Map<String, serde_json::Value>) {
    if let Some(replacement) = object
        .get("flow_model")
        .and_then(|v| v.as_str())
        .and_then(replacement_for_retired_flow_model)
    {
        object.insert("flow_model".into(), serde_json::Value::from(replacement));
    }

    let retired_refinement = object
        .get("refinement")
        .and_then(|v| v.get("model"))
        .and_then(|v| v.as_str())
        .and_then(replacement_for_retired_flow_model);
    if let Some(replacement) = retired_refinement {
        set_nested(
            object,
            "refinement",
            "model",
            serde_json::Value::from(replacement),
        );
    }
}

/// Sections that merge field-by-field instead of being replaced wholesale.
fn is_nested_section(key: &str) -> bool {
    matches!(key, "asr" | "refinement" | "streaming" | "memory_policy")
}

fn set_nested(
    dst: &mut serde_json::Map<String, serde_json::Value>,
    section: &str,
    field: &str,
    value: serde_json::Value,
) {
    let entry = dst
        .entry(section.to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if !entry.is_object() {
        *entry = serde_json::Value::Object(serde_json::Map::new());
    }
    if let Some(object) = entry.as_object_mut() {
        object.insert(field.to_string(), value);
    }
}

/// Map legacy flat compute keys named by a patch onto the nested sections.
///
/// Deliberately narrower than [`migrate_v2_to_v3`]: only the fields the patch
/// actually names are touched, so a caller updating `compute_backend` does not
/// reset the precision or the keep-loaded flag as a side effect.
fn apply_legacy_compute_patch(
    dst: &mut serde_json::Map<String, serde_json::Value>,
    patch: &serde_json::Map<String, serde_json::Value>,
) {
    if let Some(backend) = patch.get("compute_backend").and_then(|v| v.as_str()) {
        let device = match backend.trim().to_ascii_lowercase().as_str() {
            "cpu" => "cpu",
            "gpu" | "cuda" => "cuda",
            _ => "auto",
        };
        set_nested(dst, "asr", "device", device.into());
    }
    if let Some(value) = patch.get("asr_model") {
        set_nested(dst, "asr", "model", value.clone());
    }
    if let Some(value) = patch.get("asr_precision") {
        set_nested(dst, "asr", "precision", value.clone());
    }
    if let Some(value) = patch.get("keep_model_loaded") {
        set_nested(dst, "asr", "keep_loaded", value.clone());
    }
    if let Some(layers) = patch.get("flow_n_gpu_layers").and_then(|v| v.as_i64()) {
        let (device, layers) = if layers > 0 {
            ("cuda", layers as i32)
        } else {
            ("cpu", 0)
        };
        set_nested(dst, "refinement", "device", device.into());
        set_nested(dst, "refinement", "gpu_layers", layers.into());
    }

    for key in [
        "compute_backend",
        "asr_model",
        "asr_precision",
        "flow_n_gpu_layers",
        "keep_model_loaded",
    ] {
        dst.remove(key);
    }
}

/// Split the flat compute fields into the nested sections.
///
/// The mapping is deliberately conservative: a legacy `compute_backend` applied
/// to both stages, so it seeds both devices. The one exception is that a legacy
/// `flow_n_gpu_layers` of `-1` (auto) becomes CPU for refinement, because
/// putting the refiner on the GPU without a measurement is what risks a silent
/// spill — and the old "auto" meant the binary 0/99 choice, which on a 4 GB card
/// meant 99 layers and near-certain spill.
fn migrate_v2_to_v3(object: &mut serde_json::Map<String, serde_json::Value>) {
    let take_string = |object: &serde_json::Map<String, serde_json::Value>, key: &str| {
        object
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
    };

    let legacy_backend = take_string(object, "compute_backend");
    let legacy_asr_model = take_string(object, "asr_model");
    let legacy_precision = take_string(object, "asr_precision");
    let legacy_flow_model = take_string(object, "flow_model");
    let legacy_keep_loaded = object
        .get("keep_model_loaded")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let legacy_layers = object
        .get("flow_n_gpu_layers")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);

    // A legacy backend of "gpu"/"cuda"/"auto" maps onto the modern device
    // vocabulary; anything unrecognised becomes "auto".
    let asr_device = match legacy_backend.as_deref() {
        Some("cpu") => "cpu",
        Some("gpu") | Some("cuda") => "cuda",
        _ => "auto",
    };

    if !object.contains_key("asr") {
        let mut asr = serde_json::Map::new();
        asr.insert(
            "model".into(),
            legacy_asr_model.unwrap_or_else(default_asr_model).into(),
        );
        asr.insert("device".into(), asr_device.into());
        asr.insert(
            "precision".into(),
            legacy_precision
                .unwrap_or_else(default_asr_precision)
                .into(),
        );
        asr.insert("keep_loaded".into(), legacy_keep_loaded.into());
        object.insert("asr".into(), serde_json::Value::Object(asr));
    }

    if !object.contains_key("refinement") {
        // Only carry over an explicit positive layer count. `-1` was "auto",
        // which the old code turned into 99 layers on any machine with a GPU.
        let (device, layers) = match legacy_layers {
            Some(0) => ("cpu", 0),
            Some(n) if n > 0 => ("cuda", n),
            _ => ("cpu", 0),
        };
        let mut refinement = serde_json::Map::new();
        refinement.insert(
            "model".into(),
            legacy_flow_model.unwrap_or_else(default_flow_model).into(),
        );
        refinement.insert("device".into(), device.into());
        refinement.insert("gpu_layers".into(), layers.into());
        refinement.insert("context_size".into(), default_context_size().into());
        refinement.insert("keep_warm".into(), true.into());
        refinement.insert("deadline_ms".into(), default_deadline_ms().into());
        object.insert("refinement".into(), serde_json::Value::Object(refinement));
    }

    // Drop the flat keys so there is exactly one place each value lives.
    for key in [
        "compute_backend",
        "asr_model",
        "asr_precision",
        "flow_n_gpu_layers",
        "keep_model_loaded",
    ] {
        object.remove(key);
    }
}

/// Current settings schema version.
///
/// * 1 (implicit, field absent) — the flat schema, where `intelligence_tier`
///   and `cleanup_level` were stored independently and could contradict each
///   other. A default install shipped `smart_flow` + `light`, which made the
///   Stage 2 gate short-circuit so refinement never ran.
/// * 2 — `resolve_intent()` is the single authority and persisted values are
///   normalized through it on load.
/// * 3 — ASR and refinement compute settings are separate nested sections.
///   A single `compute_backend` could not express "ASR on the GPU, refinement
///   on the CPU", which is the correct configuration on a 4 GB card.
pub const CURRENT_SETTINGS_VERSION: u32 = 4;

fn default_settings_version() -> u32 {
    // Absent means a pre-versioning config, which is schema 1.
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// Schema version of the persisted document. See
    /// [`CURRENT_SETTINGS_VERSION`].
    #[serde(default = "default_settings_version")]
    pub settings_version: u32,
    pub hotkey: String,
    pub push_to_talk: bool,
    pub auto_stop_silence_ms: u64,
    /// Safety cap on a single recording, in seconds. `0` disables it.
    ///
    /// A stuck-key guard, not a feature limit: hold-to-talk stops on key-up, and
    /// a missed key-up would otherwise record until the disk filled. It does not
    /// apply to hands-free (double-tap) sessions, which are ended by an explicit
    /// second tap and are meant to run for as long as the user keeps talking.
    pub max_duration_sec: u64,
    pub language: String,
    pub auto_detect_language: bool,
    pub microphone_device_id: Option<String>,
    pub input_gain: f32,
    pub vad_sensitivity: f32,
    pub processing_mode: String,
    #[serde(default = "default_cleanup_level")]
    pub cleanup_level: String,
    #[serde(default = "default_intelligence_tier")]
    pub intelligence_tier: String,
    /// The tier to restore when polishing is switched back on.
    ///
    /// Turning polish off sets `intelligence_tier` to `raw_verbatim`, which
    /// erases which polishing tier the user was using — and `normalized()`
    /// rewrites `flow_model` to `none` along with it. Without somewhere to keep
    /// it, every Fast -> Polished toggle would land on the default tier instead
    /// of the one the user chose, which is the opposite of remembering their
    /// preference across restarts.
    #[serde(default = "default_intelligence_tier")]
    pub last_polish_tier: String,
    #[serde(default = "default_flow_model")]
    pub flow_model: String,
    #[serde(default = "default_style")]
    pub style: String,
    #[serde(default = "default_auto_style")]
    pub auto_style_from_app: bool,
    pub dictation_mode: String,
    /// Performance intent: `"auto"` | `"fast"` | `"balanced"` | `"accurate"` |
    /// `"custom"`. Anything but `"custom"` lets the resolver choose; `"custom"`
    /// is never silently overwritten.
    #[serde(default = "default_preset")]
    pub preset: String,
    /// Speech-model placement. Independent of [`Self::refinement`].
    #[serde(default)]
    pub asr: AsrSettings,
    /// Refinement-model placement. Independent of [`Self::asr`].
    #[serde(default)]
    pub refinement: RefinementSettings,
    #[serde(default)]
    pub streaming: StreamingSettings,
    #[serde(default)]
    pub memory_policy: MemoryPolicySettings,
    pub history_retention: String,
    pub overlay_position: String,
    pub overlay_theme: String,
    #[serde(default = "default_app_theme")]
    pub app_theme: String,
    #[serde(default = "default_accent_color")]
    pub accent_color: String,
    #[serde(default = "default_hud_scale")]
    pub hud_scale: String,
    #[serde(default = "default_waveform_style")]
    pub waveform_style: String,
    #[serde(default = "default_reduce_motion")]
    pub reduce_motion: bool,
    #[serde(default = "default_ui_font_scale")]
    pub ui_font_scale: String,
    #[serde(default = "default_developer_mode")]
    pub developer_mode: bool,
    pub active_profile: String,
    pub launch_at_startup: bool,
    pub start_minimized: bool,
    pub offline_mode: bool,
    pub spoken_punctuation_enabled: bool,
    pub filler_removal_enabled: bool,
    pub clipboard_restore_enabled: bool,
    pub custom_replacements: Vec<ReplacementRule>,
    pub dictionary_terms: Vec<DictionaryTerm>,
    #[serde(default)]
    pub api_enabled: bool,
    #[serde(default = "default_api_bind")]
    pub api_bind: String,
    #[serde(default = "default_api_port")]
    pub api_port: u16,
    #[serde(default)]
    pub api_mdns: bool,
    #[serde(default)]
    pub api_inject_default: bool,
}

fn default_app_theme() -> String {
    "system".into()
}

fn default_accent_color() -> String {
    "sky".into()
}

fn default_hud_scale() -> String {
    "standard".into()
}

fn default_waveform_style() -> String {
    "bars".into()
}

fn default_reduce_motion() -> bool {
    false
}

fn default_ui_font_scale() -> String {
    "normal".into()
}

fn default_api_bind() -> String {
    "lan".into()
}

fn default_asr_model() -> String {
    "1.7b".into()
}

fn default_asr_precision() -> String {
    "auto".into()
}

fn default_flow_n_gpu_layers() -> i32 {
    -1
}

fn default_api_port() -> u16 {
    7840
}

/// Deterministic Stage 1 aggressiveness for a fresh install.
///
/// `"light"` alongside the `smart_flow` tier used to mean Stage 2 never ran,
/// but that was because `cleanup_level` was doubling as the Stage 2 gate. With
/// `ResolvedIntent::run_llm` owning that decision, light Stage 1 plus LLM
/// refinement is both correct and the better default: Stage 1 already removes
/// fillers and self-repairs, so a heavier pass mostly duplicates work and
/// nudges the model toward over-editing already-clean text.
fn default_cleanup_level() -> String {
    "light".into()
}

fn default_intelligence_tier() -> String {
    "smart_flow".into()
}

fn default_flow_model() -> String {
    "qwen3.5-0.8b".into()
}

fn default_style() -> String {
    "neutral".into()
}

fn default_auto_style() -> bool {
    true
}

fn default_developer_mode() -> bool {
    false
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            settings_version: CURRENT_SETTINGS_VERSION,
            hotkey: crate::platform::default_hotkey().into(),
            push_to_talk: true,
            auto_stop_silence_ms: 1500,
            max_duration_sec: 600,
            language: "auto".into(),
            auto_detect_language: true,
            microphone_device_id: None,
            input_gain: 1.0,
            vad_sensitivity: 0.5,
            processing_mode: "smart".into(),
            cleanup_level: default_cleanup_level(),
            intelligence_tier: default_intelligence_tier(),
            last_polish_tier: default_intelligence_tier(),
            flow_model: default_flow_model(),
            style: default_style(),
            auto_style_from_app: default_auto_style(),
            dictation_mode: "normal".into(),
            preset: default_preset(),
            asr: AsrSettings::default(),
            refinement: RefinementSettings::default(),
            streaming: StreamingSettings::default(),
            memory_policy: MemoryPolicySettings::default(),
            history_retention: "30_days".into(),
            overlay_position: "bottom_center".into(),
            overlay_theme: "dark".into(),
            app_theme: default_app_theme(),
            accent_color: default_accent_color(),
            hud_scale: default_hud_scale(),
            waveform_style: default_waveform_style(),
            reduce_motion: default_reduce_motion(),
            ui_font_scale: default_ui_font_scale(),
            developer_mode: false,
            active_profile: "Default".into(),
            launch_at_startup: false,
            start_minimized: false,
            offline_mode: true,
            spoken_punctuation_enabled: true,
            filler_removal_enabled: true,
            clipboard_restore_enabled: true,
            api_enabled: false,
            api_bind: default_api_bind(),
            api_port: default_api_port(),
            api_mdns: true,
            api_inject_default: false,
            custom_replacements: CustomReplacements::default_rules(),
            dictionary_terms: vec![
                DictionaryTerm {
                    id: "1".into(),
                    term: "Qwen".into(),
                    preferred_spelling: "Qwen".into(),
                    category: "Model".into(),
                },
                DictionaryTerm {
                    id: "2".into(),
                    term: "Tauri".into(),
                    preferred_spelling: "Tauri".into(),
                    category: "Framework".into(),
                },
                DictionaryTerm {
                    id: "3".into(),
                    term: "Supabase".into(),
                    preferred_spelling: "Supabase".into(),
                    category: "Database".into(),
                },
            ],
        }
    }
}

/// The resolved dictation intent. Derived at read time from the three
/// user-touched fields (`intelligence_tier`, `cleanup_level`, and the legacy
/// `processing_mode` field). The three fields are stored independently and
/// this struct picks the right one based on what the user last touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedIntent {
    pub tier: String,
    pub flow_model: String,
    pub cleanup_level: String,
    pub processing_mode: String,
    /// The single authoritative answer to "should Stage 2 run?".
    ///
    /// Previously every caller re-derived this from `cleanup_level`, which is
    /// how a default install ended up advertising the `smart_flow` tier while
    /// the refinement stage was short-circuited by `cleanup_level: light`.
    pub run_llm: bool,
}

/// Constrain a cleanup level to what the tier can coherently mean.
///
/// The two settings govern different stages and are deliberately independent:
/// `intelligence_tier` decides whether the Stage 2 LLM runs and which model it
/// uses, `cleanup_level` decides how aggressive the deterministic Stage 1
/// pipeline is. Any combination of those is meaningful *except* an aggressive
/// Stage 1 under the verbatim tier, where the user has explicitly asked for
/// their words back unchanged.
///
/// Note what this function deliberately does **not** do: it does not force an
/// LLM tier up to a particular cleanup level. The original bug was not that
/// `smart_flow` + `light` is incoherent — it is a perfectly reasonable
/// configuration, and arguably the better one now that Stage 1 already removes
/// fillers. The bug was that `cleanup_level` was being used as the Stage 2
/// gate. That is fixed by `ResolvedIntent::run_llm`, not by clamping.
fn clamp_cleanup_for_tier(tier: &str, cleanup: &str) -> &'static str {
    let level = cleanup.trim().to_ascii_lowercase();
    let normalized = match level.as_str() {
        "raw" => "raw",
        "light" => "light",
        "high" => "high",
        "medium" => "medium",
        // Unrecognised: fall back to the tier's natural level.
        _ => cleanup_for_tier(tier),
    };
    if tier == "raw_verbatim" && matches!(normalized, "medium" | "high") {
        // Verbatim must not perform an aggressive deterministic rewrite.
        return "light";
    }
    normalized
}

/// Canonical tier ↔ flow-model mapping. Single source of truth: every other
/// copy (`commands`, `rewrite::server`, the TS `flowModelForTier`) must
/// delegate here or match it exactly.
pub fn normalize_tier(tier: &str) -> &'static str {
    match tier.trim().to_ascii_lowercase().as_str() {
        "raw_verbatim" => "raw_verbatim",
        "deep_context" => "deep_context",
        "smart_flow" => "smart_flow",
        _ => "smart_flow",
    }
}

/// Tier implied by a cleanup level, for legacy documents that have no tier.
///
/// Only `raw` implies the verbatim tier. `light` used to map here too, which
/// meant a legacy config asking for light cleanup silently lost refinement —
/// the same conflation this milestone removes.
fn tier_for_cleanup(level: &str) -> &'static str {
    match level.trim().to_ascii_lowercase().as_str() {
        "raw" => "raw_verbatim",
        "high" => "deep_context",
        _ => "smart_flow",
    }
}

pub fn flow_model_for_tier(tier: &str) -> &'static str {
    match tier.trim().to_ascii_lowercase().as_str() {
        "deep_context" => "qwen3.5-2b",
        "raw_verbatim" => "none",
        _ => "qwen3.5-0.8b",
    }
}

fn cleanup_for_tier(tier: &str) -> &'static str {
    match tier.trim().to_ascii_lowercase().as_str() {
        "raw_verbatim" => "raw",
        "deep_context" => "high",
        _ => "medium",
    }
}

fn cleanup_for_processing_mode(mode: &str) -> &'static str {
    match mode.trim().to_ascii_lowercase().as_str() {
        "raw" => "raw",
        "flow" => "medium",
        _ => "light",
    }
}

fn processing_mode_for_cleanup(level: &str) -> &'static str {
    match level.trim().to_ascii_lowercase().as_str() {
        "raw" => "raw",
        "medium" | "high" => "flow",
        _ => "smart",
    }
}

impl AppSettings {
    /// Backwards-compatible effective cleanup level. Honors an explicit
    /// `cleanup_level`; otherwise falls back to the legacy `processing_mode`
    /// ("raw" / "flow" / anything else -> "light").
    pub fn resolved_cleanup_level(&self) -> String {
        self.resolve_intent().cleanup_level
    }

    /// Compute the effective dictation intent at read time. The priority
    /// order is: `intelligence_tier` (if explicitly set) > `cleanup_level`
    /// (if explicitly set) > `processing_mode` fallback. The three
    /// underlying fields are never written back from this function.
    pub fn resolve_intent(&self) -> ResolvedIntent {
        let tier_raw = self.intelligence_tier.trim();
        let tier: String = if !tier_raw.is_empty() {
            normalize_tier(tier_raw).into()
        } else if !self.cleanup_level.trim().is_empty() {
            tier_for_cleanup(&self.cleanup_level).into()
        } else {
            tier_for_cleanup(cleanup_for_processing_mode(&self.processing_mode)).into()
        };

        let flow_model: String = flow_model_for_tier(&tier).into();

        // Fall back through the same chain the tier used, so a legacy document
        // that only has `processing_mode` keeps the level that field implies
        // instead of jumping to the tier's natural level.
        let requested_cleanup = if !self.cleanup_level.trim().is_empty() {
            self.cleanup_level.trim().to_ascii_lowercase()
        } else if !self.processing_mode.trim().is_empty() {
            cleanup_for_processing_mode(&self.processing_mode).to_string()
        } else {
            cleanup_for_tier(&tier).to_string()
        };
        // Clamp so the resolved intent can never contradict itself.
        let cleanup_level: String = clamp_cleanup_for_tier(&tier, &requested_cleanup).into();
        let processing_mode: String = processing_mode_for_cleanup(&cleanup_level).into();

        // The authoritative Stage 2 gate.
        //
        // The tier owns the decision, with one exception: `cleanup_level: raw`
        // is a request for the words back verbatim, and running a polishing LLM
        // over them would contradict it outright. `light`, `medium` and `high`
        // are all degrees of cleanup that refinement complements — the original
        // bug was that *light* blocked Stage 2, which was simply wrong.
        let run_llm = tier != "raw_verbatim" && flow_model != "none" && cleanup_level != "raw";

        ResolvedIntent {
            tier,
            flow_model,
            cleanup_level,
            processing_mode,
            run_llm,
        }
    }

    /// Rewrite the stored fields so the document on disk matches what
    /// [`Self::resolve_intent`] would compute, and stamp the current schema
    /// version. Idempotent.
    pub fn normalized(mut self) -> Self {
        let intent = self.resolve_intent();
        self.intelligence_tier = intent.tier;
        self.cleanup_level = intent.cleanup_level;
        self.processing_mode = intent.processing_mode;

        // `intelligence_tier` and `refinement.model` are the same choice
        // expressed twice, so one has to be the authority. The tier is, because
        // it is what the tier UI writes; `refinement.model` follows. Without
        // this the two drift and the app loads one model while claiming another.
        self.flow_model = intent.flow_model.clone();
        self.refinement.model = intent.flow_model;

        // A CPU refinement device cannot have GPU layers, and zero GPU layers
        // is a CPU request regardless of which accelerated backend alias was
        // persisted by an older build. Negative values are the internal
        // "choose the fastest safe full or partial offload" sentinel.
        if self.refinement.device.eq_ignore_ascii_case("cpu") {
            self.refinement.gpu_layers = 0;
        } else if self.refinement.gpu_layers == 0 {
            self.refinement.device = "cpu".into();
        } else if self.refinement.gpu_layers < -1 {
            self.refinement.gpu_layers = -1;
        }
        // Refinement on the GPU requires the memory policy to permit it.
        if !self.memory_policy.allow_gpu_refinement
            && !self.refinement.device.eq_ignore_ascii_case("cpu")
        {
            self.refinement.device = "cpu".into();
            self.refinement.gpu_layers = 0;
        }

        self.settings_version = CURRENT_SETTINGS_VERSION;
        self
    }

    /// `true` when the persisted document predates the current schema.
    pub fn needs_migration(&self) -> bool {
        self.settings_version < CURRENT_SETTINGS_VERSION
    }
}

pub struct SettingsStore {
    file_path: PathBuf,
    settings: Arc<RwLock<AppSettings>>,
}

impl SettingsStore {
    pub fn new(file_path: PathBuf) -> Self {
        // Migrate the raw document before deserializing: the schema-2 keys the
        // migration reads no longer exist on the struct, so parsing first would
        // throw away exactly what it needs.
        let mut migrating = false;
        let loaded = if file_path.exists() {
            match fs::read_to_string(&file_path) {
                Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(mut value) => {
                        let found = migrate_document(&mut value);
                        migrating = found < CURRENT_SETTINGS_VERSION;
                        serde_json::from_value::<AppSettings>(value).unwrap_or_else(|err| {
                            log::warn!(
                                "Settings file could not be applied ({err}); using defaults"
                            );
                            AppSettings::default()
                        })
                    }
                    Err(err) => {
                        log::warn!("Settings file is not valid JSON ({err}); using defaults");
                        AppSettings::default()
                    }
                },
                Err(_) => AppSettings::default(),
            }
        } else {
            AppSettings::default()
        };

        let normalized = loaded.normalized();
        if migrating {
            log::info!(
                "Migrated settings to schema v{CURRENT_SETTINGS_VERSION}: tier={} cleanup={} run_llm={}",
                normalized.intelligence_tier,
                normalized.cleanup_level,
                normalized.resolve_intent().run_llm
            );
        }

        let store = Self {
            file_path,
            settings: Arc::new(RwLock::new(normalized.clone())),
        };
        if migrating {
            // Persist the migration so the next launch is a no-op.
            if let Err(err) = store.write_to_disk(&normalized) {
                log::warn!("Could not persist migrated settings: {err}");
            }
        }
        store
    }

    fn write_to_disk(&self, settings: &AppSettings) -> Result<(), String> {
        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create settings directory: {e}"))?;
        }
        let json = serde_json::to_string_pretty(settings)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;
        use std::io::Write;
        let temporary = self
            .file_path
            .with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> std::io::Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &self.file_path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(|e| format!("Failed to write settings file: {e}"))
    }

    pub fn get(&self) -> AppSettings {
        self.settings.read().clone()
    }

    pub fn update(&self, new_settings: AppSettings) -> Result<AppSettings, String> {
        let mut settings = self.settings.write();
        let normalized = new_settings.normalized();
        self.write_to_disk(&normalized)?;
        *settings = normalized.clone();
        Ok(normalized)
    }

    pub fn merge_update(&self, patch: serde_json::Value) -> Result<AppSettings, String> {
        let mut lock = self.settings.write();
        let mut merged_value = serde_json::to_value(&*lock)
            .map_err(|e| format!("Failed to serialize current settings: {e}"))?;

        match (merged_value.as_object_mut(), patch.as_object()) {
            (Some(dst), Some(src)) => {
                for (key, value) in src {
                    // Nested sections merge field-by-field. A shallow insert
                    // would let a patch that names one field of `asr` silently
                    // reset the other three to their defaults.
                    match (dst.get_mut(key), value) {
                        (
                            Some(serde_json::Value::Object(existing)),
                            serde_json::Value::Object(incoming),
                        ) if is_nested_section(key) => {
                            for (field, field_value) in incoming {
                                existing.insert(field.clone(), field_value.clone());
                            }
                        }
                        _ => {
                            dst.insert(key.clone(), value.clone());
                        }
                    }
                }
            }
            _ => return Err("Settings patch must be a JSON object".into()),
        }

        // A patch may still be written in the legacy flat vocabulary — the LAN
        // API and any older frontend both do — so translate those keys onto the
        // nested sections rather than dropping them silently.
        if let (Some(dst), Some(src)) = (merged_value.as_object_mut(), patch.as_object()) {
            apply_legacy_compute_patch(dst, src);
        }

        let mut merged: AppSettings = serde_json::from_value(merged_value)
            .map_err(|e| format!("Failed to apply settings patch: {e}"))?;

        // `intelligence_tier` and `cleanup_level` are independent controls over
        // different stages, so a patch to one must not silently rewrite the
        // other. In particular a cleanup change must never flip the tier:
        // picking "light" would then turn refinement off without the user
        // asking, which is the same class of surprise as the original bug.
        // `normalized()` afterwards enforces the one genuine constraint
        // (verbatim cannot run an aggressive Stage 1).
        if let Some(src) = patch.as_object() {
            if src.contains_key("intelligence_tier") {
                merged.intelligence_tier = normalize_tier(&merged.intelligence_tier).to_string();
            } else if src.contains_key("processing_mode") && !src.contains_key("cleanup_level") {
                // Legacy field: map it onto the modern cleanup level only.
                merged.cleanup_level =
                    cleanup_for_processing_mode(&merged.processing_mode).to_string();
            }
        }

        let merged = merged.normalized();
        self.write_to_disk(&merged)?;
        *lock = merged.clone();
        Ok(merged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_settings_writes_leave_disk_and_memory_consistent() {
        let dir =
            std::env::temp_dir().join(format!("reflow_settings_atomic_{}", uuid::Uuid::new_v4()));
        let path = dir.join("settings.json");
        let store = Arc::new(SettingsStore::new(path.clone()));
        let writers: Vec<_> = (0..8)
            .map(|index| {
                let store = Arc::clone(&store);
                std::thread::spawn(move || {
                    for _ in 0..10 {
                        let mut settings = store.get();
                        settings.language = format!("language-{index}");
                        store.update(settings).unwrap();
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let persisted: AppSettings =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(persisted.language, store.get().language);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_persistence_does_not_change_active_settings() {
        let dir =
            std::env::temp_dir().join(format!("reflow_settings_fail_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let blocked_parent = dir.join("file");
        fs::write(&blocked_parent, "existing").unwrap();
        let store = SettingsStore::new(blocked_parent.join("settings.json"));
        let before = store.get().language;
        assert!(store
            .merge_update(serde_json::json!({"language":"changed"}))
            .is_err());
        assert_eq!(store.get().language, before);
        assert_eq!(fs::read_to_string(&blocked_parent).unwrap(), "existing");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn developer_mode_and_cleanup_defaults() {
        let settings = AppSettings::default();
        assert!(!settings.developer_mode);
        assert_eq!(settings.cleanup_level, "light");
        assert_eq!(settings.intelligence_tier, "smart_flow");
        assert_eq!(settings.flow_model, "qwen3.5-0.8b");
        assert_eq!(settings.style, "neutral");
        assert!(settings.auto_style_from_app);
        assert_eq!(settings.processing_mode, "smart");
        assert_eq!(settings.asr.precision, "auto");
    }

    /// `cleanup_level: raw` is a request for the words back verbatim, so it is
    /// the one Stage 1 level that also suppresses Stage 2.
    #[test]
    fn raw_cleanup_suppresses_the_llm_but_light_does_not() {
        let raw = AppSettings {
            cleanup_level: "raw".into(),
            ..AppSettings::default()
        };
        let intent = raw.resolve_intent();
        assert_eq!(intent.tier, "smart_flow", "the tier is untouched");
        assert_eq!(intent.cleanup_level, "raw");
        assert!(
            !intent.run_llm,
            "polishing verbatim text contradicts the request"
        );

        // Light is a degree of cleanup, not a verbatim request, and must not
        // block refinement. This was the original bug.
        let light = AppSettings {
            cleanup_level: "light".into(),
            ..AppSettings::default()
        };
        assert!(light.resolve_intent().run_llm);
    }

    #[test]
    fn resolved_cleanup_level_falls_back_through_the_resolver() {
        // With no tier and no cleanup level, the legacy processing_mode
        // decides — and the answer is still a coherent tier/level pair.
        let mut settings = AppSettings::default();
        settings.intelligence_tier.clear();
        settings.cleanup_level.clear();

        settings.processing_mode = "raw".into();
        assert_eq!(settings.resolved_cleanup_level(), "raw");
        assert!(!settings.resolve_intent().run_llm);

        settings.processing_mode = "flow".into();
        assert_eq!(settings.resolved_cleanup_level(), "medium");
        assert!(settings.resolve_intent().run_llm);

        settings.processing_mode = "smart".into();
        assert_eq!(settings.resolved_cleanup_level(), "light");
        // Light does not suppress refinement; only raw does.
        assert!(settings.resolve_intent().run_llm);

        // An explicit cleanup level wins over processing_mode.
        settings.cleanup_level = "high".into();
        assert_eq!(settings.resolved_cleanup_level(), "high");
    }

    #[test]
    fn resolve_intent_priority_order() {
        // 1) An explicit tier decides the model, and the explicit cleanup level
        //    is preserved — the two are independent controls. Here the verbatim
        //    Stage 1 request also suppresses Stage 2.
        let s = AppSettings {
            intelligence_tier: "deep_context".into(),
            cleanup_level: "raw".into(),
            processing_mode: "raw".into(),
            ..AppSettings::default()
        };
        let intent = s.resolve_intent();
        assert_eq!(intent.tier, "deep_context");
        assert_eq!(intent.flow_model, "qwen3.5-2b");
        assert_eq!(
            intent.cleanup_level, "raw",
            "an explicit Stage 1 choice is preserved"
        );
        assert_eq!(intent.processing_mode, "raw");
        assert!(
            !intent.run_llm,
            "raw is a verbatim request, so Stage 2 is suppressed"
        );

        // 2) With no tier but an explicit cleanup_level, derive the tier.
        let mut s = AppSettings::default();
        s.intelligence_tier.clear();
        s.cleanup_level = "high".into();
        s.processing_mode = "smart".into();
        let intent = s.resolve_intent();
        assert_eq!(intent.tier, "deep_context");
        assert_eq!(intent.flow_model, "qwen3.5-2b");

        // 3) No tier and no cleanup_level: fall back to processing_mode.
        let mut s = AppSettings::default();
        s.intelligence_tier.clear();
        s.cleanup_level.clear();
        s.processing_mode = "raw".into();
        let intent = s.resolve_intent();
        assert_eq!(intent.tier, "raw_verbatim");
        assert_eq!(intent.flow_model, "none");
        assert!(!intent.run_llm);
    }

    #[test]
    fn null_gpu_layer_override_deserializes_as_auto() {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value["refinement"]["gpu_layers"] = serde_json::Value::Null;

        let loaded: AppSettings = serde_json::from_value(value).unwrap();
        assert_eq!(loaded.refinement.gpu_layers, -1);
    }

    #[test]
    fn legacy_json_without_new_fields_deserializes() {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        let obj = value.as_object_mut().unwrap();
        obj.remove("cleanup_level");
        obj.remove("intelligence_tier");
        obj.remove("flow_model");
        obj.remove("style");
        obj.remove("auto_style_from_app");
        obj.remove("developer_mode");
        obj.remove("app_theme");
        obj.remove("accent_color");
        obj.remove("hud_scale");
        obj.remove("waveform_style");
        obj.remove("reduce_motion");
        obj.remove("ui_font_scale");
        obj.remove("asr");
        obj.remove("refinement");
        obj.remove("streaming");
        obj.remove("memory_policy");
        obj.remove("preset");
        obj.remove("settings_version");
        let loaded: AppSettings = serde_json::from_value(value).unwrap();
        assert_eq!(loaded.settings_version, 1, "absent version means schema 1");
        assert_eq!(loaded.cleanup_level, "light");
        assert_eq!(loaded.intelligence_tier, "smart_flow");
        assert_eq!(loaded.flow_model, "qwen3.5-0.8b");
        assert_eq!(loaded.style, "neutral");
        assert!(loaded.auto_style_from_app);
        assert!(!loaded.developer_mode);
        assert_eq!(loaded.app_theme, "system");
        assert_eq!(loaded.accent_color, "sky");
        assert_eq!(loaded.hud_scale, "standard");
        assert_eq!(loaded.waveform_style, "bars");
        assert!(!loaded.reduce_motion);
        assert_eq!(loaded.ui_font_scale, "normal");
        // Every nested section falls back to its own defaults.
        assert_eq!(loaded.asr, AsrSettings::default());
        assert_eq!(loaded.refinement, RefinementSettings::default());
        assert_eq!(loaded.streaming, StreamingSettings::default());
        assert_eq!(loaded.memory_policy, MemoryPolicySettings::default());
        assert_eq!(loaded.preset, "auto");
    }

    #[test]
    fn asr_precision_round_trips_through_settings_store() {
        let dir = std::env::temp_dir().join(format!(
            "reflow_settings_precision_{}",
            uuid::Uuid::new_v4()
        ));
        let store = SettingsStore::new(dir.join("settings.json"));

        assert_eq!(store.get().asr.precision, "auto");

        // Each valid value must survive a merge_update + disk write + reload
        // cycle. This is the path the UI uses when the user picks a precision.
        for value in ["int4", "int8", "bf16", "auto"] {
            let updated = store
                .merge_update(serde_json::json!({ "asr": { "precision": value } }))
                .unwrap();
            assert_eq!(updated.asr.precision, value);

            let store2 = SettingsStore::new(dir.join("settings.json"));
            assert_eq!(
                store2.get().asr.precision,
                value,
                "precision {value:?} did not survive a settings reload"
            );
        }

        // A patch naming one field of a nested section must not reset the rest
        // of that section to its defaults.
        let current = store.get();
        assert_eq!(current.asr.model, "1.7b");
        assert_eq!(current.asr.device, "auto");
        assert!(current.asr.keep_loaded);

        let _ = std::fs::remove_dir_all(dir);
    }
}
