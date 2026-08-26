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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub hotkey: String,
    pub push_to_talk: bool,
    pub auto_stop_silence_ms: u64,
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
    #[serde(default = "default_flow_model")]
    pub flow_model: String,
    #[serde(default = "default_style")]
    pub style: String,
    #[serde(default = "default_auto_style")]
    pub auto_style_from_app: bool,
    pub dictation_mode: String,
    pub compute_backend: String,
    #[serde(default = "default_asr_model")]
    pub asr_model: String,
    #[serde(default = "default_asr_precision")]
    pub asr_precision: String,
    /// Number of transformer layers to offload to the GPU for the Stage 2 LLM
    /// (`llama-server`). `-1` = auto (use the binary 0/99 choice driven by
    /// `compute_backend`); `0` = force CPU; any positive value = offload that
    /// many layers (partial offload). The runtime is relaunched on change.
    #[serde(
        default = "default_flow_n_gpu_layers",
        deserialize_with = "deserialize_flow_n_gpu_layers"
    )]
    pub flow_n_gpu_layers: i32,
    pub keep_model_loaded: bool,
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

fn deserialize_flow_n_gpu_layers<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<i32>::deserialize(deserializer)?.unwrap_or_else(default_flow_n_gpu_layers))
}

fn default_api_port() -> u16 {
    7840
}

fn default_cleanup_level() -> String {
    "light".into()
}

fn default_intelligence_tier() -> String {
    "smart_flow".into()
}

fn default_flow_model() -> String {
    "lfm2.5-1.2b".into()
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
            hotkey: crate::platform::default_hotkey().into(),
            push_to_talk: true,
            auto_stop_silence_ms: 1500,
            max_duration_sec: 60,
            language: "auto".into(),
            auto_detect_language: true,
            microphone_device_id: None,
            input_gain: 1.0,
            vad_sensitivity: 0.5,
            processing_mode: "smart".into(),
            cleanup_level: default_cleanup_level(),
            intelligence_tier: default_intelligence_tier(),
            flow_model: default_flow_model(),
            style: default_style(),
            auto_style_from_app: default_auto_style(),
            dictation_mode: "normal".into(),
            compute_backend: "auto".into(),
            asr_model: default_asr_model(),
            asr_precision: default_asr_precision(),
            flow_n_gpu_layers: default_flow_n_gpu_layers(),
            keep_model_loaded: true,
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
}

fn tier_for_cleanup(level: &str) -> &'static str {
    match level.trim().to_ascii_lowercase().as_str() {
        "raw" | "light" => "raw_verbatim",
        "high" => "deep_context",
        "medium" => "smart_flow",
        _ => "smart_flow",
    }
}

fn flow_model_for_tier(tier: &str) -> &'static str {
    match tier.trim().to_ascii_lowercase().as_str() {
        "deep_context" => "qwen3.5-2b",
        "raw_verbatim" => "none",
        _ => "lfm2.5-1.2b",
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
        if !self.cleanup_level.is_empty() {
            return self.cleanup_level.clone();
        }
        cleanup_for_processing_mode(&self.processing_mode).to_string()
    }

    /// Compute the effective dictation intent at read time. The priority
    /// order is: `intelligence_tier` (if explicitly set) > `cleanup_level`
    /// (if explicitly set) > `processing_mode` fallback. The three
    /// underlying fields are never written back from this function.
    pub fn resolve_intent(&self) -> ResolvedIntent {
        let tier_raw = self.intelligence_tier.trim().to_ascii_lowercase();
        let tier: String = if !tier_raw.is_empty() {
            match tier_raw.as_str() {
                "raw_verbatim" | "smart_flow" | "deep_context" => tier_raw,
                _ => "smart_flow".into(),
            }
        } else if !self.cleanup_level.trim().is_empty() {
            tier_for_cleanup(&self.cleanup_level).into()
        } else {
            tier_for_cleanup(cleanup_for_processing_mode(&self.processing_mode)).into()
        };

        let flow_model: String = flow_model_for_tier(&tier).into();

        let cleanup_level: String = if !self.cleanup_level.trim().is_empty() {
            self.cleanup_level.trim().to_ascii_lowercase()
        } else {
            cleanup_for_tier(&tier).into()
        };

        let processing_mode: String = processing_mode_for_cleanup(&cleanup_level).into();

        ResolvedIntent {
            tier,
            flow_model,
            cleanup_level,
            processing_mode,
        }
    }
}

pub struct SettingsStore {
    file_path: PathBuf,
    settings: Arc<RwLock<AppSettings>>,
}

impl SettingsStore {
    pub fn new(file_path: PathBuf) -> Self {
        let loaded = if file_path.exists() {
            match fs::read_to_string(&file_path) {
                Ok(content) => serde_json::from_str::<AppSettings>(&content)
                    .unwrap_or_else(|_| AppSettings::default()),
                Err(_) => AppSettings::default(),
            }
        } else {
            AppSettings::default()
        };

        Self {
            file_path,
            settings: Arc::new(RwLock::new(loaded)),
        }
    }

    pub fn get(&self) -> AppSettings {
        self.settings.read().clone()
    }

    pub fn update(&self, new_settings: AppSettings) -> Result<AppSettings, String> {
        if let Some(parent) = self.file_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let json = serde_json::to_string_pretty(&new_settings)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;

        fs::write(&self.file_path, json)
            .map_err(|e| format!("Failed to write settings file: {}", e))?;

        *self.settings.write() = new_settings.clone();
        Ok(new_settings)
    }

    pub fn merge_update(&self, patch: serde_json::Value) -> Result<AppSettings, String> {
        let mut lock = self.settings.write();
        let mut merged_value = serde_json::to_value(&*lock)
            .map_err(|e| format!("Failed to serialize current settings: {e}"))?;

        match (merged_value.as_object_mut(), patch.as_object()) {
            (Some(dst), Some(src)) => {
                for (key, value) in src {
                    dst.insert(key.clone(), value.clone());
                }
            }
            _ => return Err("Settings patch must be a JSON object".into()),
        }

        let mut merged: AppSettings = serde_json::from_value(merged_value)
            .map_err(|e| format!("Failed to apply settings patch: {e}"))?;
        if let Some(src) = patch.as_object() {
            if src.contains_key("intelligence_tier") {
                // Only the tier itself and its deterministic flow_model are
                // touched. The cleanup_level and processing_mode fields stay
                // as the user last set them; the effective intent is computed
                // at read time via `resolve_intent()`.
                let normalized = merged.intelligence_tier.trim().to_ascii_lowercase();
                merged.intelligence_tier = match normalized.as_str() {
                    "deep_context" | "raw_verbatim" | "smart_flow" => normalized,
                    _ => "smart_flow".into(),
                };
                merged.flow_model = flow_model_for_tier(&merged.intelligence_tier).to_string();
            } else if src.contains_key("cleanup_level") {
                merged.processing_mode =
                    processing_mode_for_cleanup(&merged.cleanup_level).to_string();
            } else if src.contains_key("processing_mode") {
                // Legacy field update: keep the existing semantics where
                // the user-set processing_mode maps to a cleanup level, but
                // do NOT clobber intelligence_tier or flow_model. The
                // effective intent resolves at read time.
                merged.cleanup_level =
                    cleanup_for_processing_mode(&merged.processing_mode).to_string();
            }
        }

        if let Some(parent) = self.file_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let json = serde_json::to_string_pretty(&merged)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;

        fs::write(&self.file_path, json)
            .map_err(|e| format!("Failed to write settings file: {}", e))?;

        *lock = merged.clone();
        Ok(merged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn developer_mode_and_cleanup_defaults() {
        let settings = AppSettings::default();
        assert!(!settings.developer_mode);
        assert_eq!(settings.cleanup_level, "light");
        assert_eq!(settings.intelligence_tier, "smart_flow");
        assert_eq!(settings.flow_model, "lfm2.5-1.2b");
        assert_eq!(settings.style, "neutral");
        assert!(settings.auto_style_from_app);
        assert_eq!(settings.processing_mode, "smart");
        assert_eq!(settings.asr_precision, "auto");
    }

    #[test]
    fn resolved_cleanup_level_falls_back_to_processing_mode() {
        let mut settings = AppSettings::default();
        settings.cleanup_level.clear();
        settings.processing_mode = "raw".into();
        assert_eq!(settings.resolved_cleanup_level(), "raw");
        settings.processing_mode = "flow".into();
        assert_eq!(settings.resolved_cleanup_level(), "medium");
        settings.processing_mode = "smart".into();
        assert_eq!(settings.resolved_cleanup_level(), "light");
        settings.cleanup_level = "high".into();
        assert_eq!(settings.resolved_cleanup_level(), "high");
    }

    #[test]
    fn merge_update_syncs_cleanup_level_and_processing_mode() {
        let dir = std::env::temp_dir().join(format!("reflow_settings_{}", uuid::Uuid::new_v4()));
        let store = SettingsStore::new(dir.join("settings.json"));

        let updated = store
            .merge_update(serde_json::json!({ "processing_mode": "flow" }))
            .unwrap();
        assert_eq!(updated.processing_mode, "flow");
        assert_eq!(updated.cleanup_level, "medium");

        let updated = store
            .merge_update(serde_json::json!({ "cleanup_level": "high" }))
            .unwrap();
        assert_eq!(updated.cleanup_level, "high");
        assert_eq!(updated.processing_mode, "flow");

        let updated = store
            .merge_update(serde_json::json!({ "cleanup_level": "raw" }))
            .unwrap();
        assert_eq!(updated.processing_mode, "raw");

        let updated = store
            .merge_update(serde_json::json!({ "cleanup_level": "light" }))
            .unwrap();
        assert_eq!(updated.processing_mode, "smart");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn merge_update_does_not_clobber_tier_with_cleanup_level() {
        // Regression: prior to the fix, setting cleanup_level=high after the
        // user picked the raw_verbatim tier would silently rewrite the
        // tier to deep_context. The three fields must stay independent.
        let dir = std::env::temp_dir().join(format!(
            "reflow_settings_decoupled_{}",
            uuid::Uuid::new_v4()
        ));
        let store = SettingsStore::new(dir.join("settings.json"));

        let updated = store
            .merge_update(serde_json::json!({ "intelligence_tier": "raw_verbatim" }))
            .unwrap();
        assert_eq!(updated.intelligence_tier, "raw_verbatim");
        assert_eq!(updated.flow_model, "none");
        // The default cleanup level remains independent of the tier update.
        assert_eq!(updated.cleanup_level, "light");

        let updated = store
            .merge_update(serde_json::json!({ "cleanup_level": "high" }))
            .unwrap();
        assert_eq!(updated.cleanup_level, "high");
        assert_eq!(
            updated.intelligence_tier, "raw_verbatim",
            "cleanup_level must not overwrite the user's tier choice"
        );
        assert_eq!(
            updated.flow_model, "none",
            "cleanup_level must not overwrite the flow_model derived from the tier"
        );
        assert_eq!(updated.processing_mode, "flow");

        // The resolved intent honors the explicit tier, not the cleanup_level.
        let intent = updated.resolve_intent();
        assert_eq!(intent.tier, "raw_verbatim");
        assert_eq!(intent.flow_model, "none");
        assert_eq!(intent.cleanup_level, "high");
        assert_eq!(intent.processing_mode, "flow");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resolve_intent_priority_order() {
        // 1) explicit tier always wins.
        let mut s = AppSettings::default();
        s.intelligence_tier = "deep_context".into();
        s.cleanup_level = "raw".into();
        s.processing_mode = "raw".into();
        let intent = s.resolve_intent();
        assert_eq!(intent.tier, "deep_context");
        assert_eq!(intent.flow_model, "qwen3.5-2b");
        assert_eq!(intent.cleanup_level, "raw");
        assert_eq!(intent.processing_mode, "raw");

        // 2) with no tier but an explicit cleanup_level, derive the tier.
        let mut s = AppSettings::default();
        s.intelligence_tier.clear();
        s.cleanup_level = "high".into();
        s.processing_mode = "smart".into();
        let intent = s.resolve_intent();
        assert_eq!(intent.tier, "deep_context");
        assert_eq!(intent.flow_model, "qwen3.5-2b");

        // 3) no tier + no cleanup_level: fall back to processing_mode.
        let mut s = AppSettings::default();
        s.intelligence_tier.clear();
        s.cleanup_level.clear();
        s.processing_mode = "raw".into();
        let intent = s.resolve_intent();
        assert_eq!(intent.tier, "raw_verbatim");
        assert_eq!(intent.flow_model, "none");
    }

    #[test]
    fn null_gpu_layer_override_deserializes_as_auto() {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value["flow_n_gpu_layers"] = serde_json::Value::Null;

        let loaded: AppSettings = serde_json::from_value(value).unwrap();
        assert_eq!(loaded.flow_n_gpu_layers, -1);
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
        obj.remove("asr_precision");
        obj.remove("flow_n_gpu_layers");
        let loaded: AppSettings = serde_json::from_value(value).unwrap();
        assert_eq!(loaded.cleanup_level, "light");
        assert_eq!(loaded.intelligence_tier, "smart_flow");
        assert_eq!(loaded.flow_model, "lfm2.5-1.2b");
        assert_eq!(loaded.style, "neutral");
        assert!(loaded.auto_style_from_app);
        assert!(!loaded.developer_mode);
        assert_eq!(loaded.app_theme, "system");
        assert_eq!(loaded.accent_color, "sky");
        assert_eq!(loaded.hud_scale, "standard");
        assert_eq!(loaded.waveform_style, "bars");
        assert!(!loaded.reduce_motion);
        assert_eq!(loaded.ui_font_scale, "normal");
        assert_eq!(loaded.asr_precision, "auto");
        assert_eq!(loaded.flow_n_gpu_layers, -1);
    }

    #[test]
    fn asr_precision_round_trips_through_settings_store() {
        let dir = std::env::temp_dir().join(format!(
            "reflow_settings_precision_{}",
            uuid::Uuid::new_v4()
        ));
        let store = SettingsStore::new(dir.join("settings.json"));

        // Default is "auto"
        assert_eq!(store.get().asr_precision, "auto");

        // Each of the four valid values must survive a merge_update +
        // disk write + reload cycle. This is the path the UI uses when the
        // user clicks a precision card.
        for value in ["int4", "int8", "bf16", "auto"] {
            let updated = store
                .merge_update(serde_json::json!({ "asr_precision": value }))
                .unwrap();
            assert_eq!(updated.asr_precision, value);

            // Open a fresh store pointing at the same on-disk file and
            // confirm the value is what we just wrote. This is what the
            // next app launch sees.
            let store2 = SettingsStore::new(dir.join("settings.json"));
            assert_eq!(
                store2.get().asr_precision,
                value,
                "asr_precision {value:?} did not survive a settings reload"
            );
        }

        // Other settings must be untouched by an asr_precision update.
        let current = store.get();
        assert_eq!(current.asr_model, "1.7b");
        assert_eq!(current.compute_backend, "auto");

        let _ = std::fs::remove_dir_all(dir);
    }
}
