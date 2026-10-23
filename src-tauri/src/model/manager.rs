use crate::asr::engine::EngineStatus;
use crate::state::ModelStatus;
use std::fs;
use std::path::{Path, PathBuf};

pub struct ModelSpec {
    pub id: &'static str,       // "0.6b" | "1.7b"
    pub dir_name: &'static str, // qwen3-asr-0.6b
    pub repo: &'static str,     // Qwen/Qwen3-ASR-0.6B-hf
    pub label: &'static str,    // "0.6B · Realtime"
    pub approx_bytes: u64,
}

pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: "0.6b",
        dir_name: "qwen3-asr-0.6b",
        repo: "Qwen/Qwen3-ASR-0.6B-hf",
        label: "0.6B",
        approx_bytes: 1_565_000_000,
    },
    ModelSpec {
        id: "1.7b",
        dir_name: "qwen3-asr-1.7b",
        repo: "Qwen/Qwen3-ASR-1.7B-hf",
        label: "1.7B",
        approx_bytes: 4_076_000_000,
    },
    ModelSpec {
        id: "native-0.6b",
        dir_name: "qwen3-asr-0.6b-native",
        repo: "ggml-org/Qwen3-ASR-0.6B-GGUF",
        label: "0.6B (native)",
        approx_bytes: 1_019_141_728,
    },
    ModelSpec {
        id: "native-1.7b",
        dir_name: "qwen3-asr-1.7b-native",
        repo: "ggml-org/Qwen3-ASR-1.7B-GGUF",
        label: "1.7B (native)",
        approx_bytes: 2_520_744_288,
    },
    ModelSpec {
        id: "phonon-2",
        dir_name: "phonon-2",
        repo: "FermionResearch/Phonon-2",
        label: "Phonon-2 · Fast English",
        approx_bytes: 163_515_201,
    },
    ModelSpec {
        id: "zipformer-20m",
        dir_name: "zipformer-20m",
        repo: "csukuangfj/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17",
        label: "Zipformer 20M INT8",
        approx_bytes: 43_649_301,
    },
];

pub fn runtime_model_id(model: &str, runtime: &str) -> String {
    // Already a concrete dir id (native variant or bundled-runtime model):
    // re-mapping would produce a nonexistent "native-native-*" id.
    if matches!(model, "phonon-2" | "zipformer-20m") || model.starts_with("native-") {
        return model.to_string();
    }
    if runtime == "native" {
        format!("native-{model}")
    } else {
        model.to_string()
    }
}

pub fn spec_for(id: &str) -> &'static ModelSpec {
    MODELS.iter().find(|m| m.id == id).unwrap_or(&MODELS[0])
}

pub struct ModelManager {
    models_dir: PathBuf,
}

fn nonempty_file(path: &std::path::Path) -> bool {
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
}

impl ModelManager {
    pub fn new(models_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&models_dir);
        Self { models_dir }
    }

    pub fn get_model_dir(&self, id: &str) -> PathBuf {
        self.models_dir.join(spec_for(id).dir_name)
    }

    pub fn repo_for(&self, id: &str) -> &'static str {
        spec_for(id).repo
    }

    /// Weights are considered installed when a real HF model directory is there.
    pub fn is_installed(&self, id: &str) -> bool {
        let dir = self.get_model_dir(id);
        if matches!(id, "phonon-2" | "zipformer-20m") {
            // Manifest-pinned files only: every listed file must be present.
            // Zipformer ships three ONNX files plus tokens.txt — checking only
            // the encoder would pass on an interrupted multi-file download.
            if let Some(manifest) =
                crate::profile::manifest::asr_manifest(id).filter(|m| m.id == id)
            {
                return nonempty_file(&dir.join(manifest.filename))
                    && manifest
                        .auxiliary_files
                        .iter()
                        .all(|f| nonempty_file(&dir.join(f.filename)));
            }
            return false;
        }
        if let Some(manifest) =
            crate::profile::manifest::native_asr_manifest(id).filter(|m| m.id == id)
        {
            nonempty_file(&dir.join(manifest.filename))
                && manifest
                    .auxiliary_files
                    .iter()
                    .all(|f| nonempty_file(&dir.join(f.filename)))
        } else if let Some(manifest) =
            crate::profile::manifest::asr_manifest(id).filter(|m| m.id == id)
        {
            if dir.join("reflow-install.json").exists() {
                let valid = std::fs::read(dir.join("reflow-install.json"))
                    .ok()
                    .filter(|bytes| bytes.len() <= 16 * 1024)
                    .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                    .is_some_and(|receipt| {
                        receipt.get("model_id").and_then(|value| value.as_str()) == Some(id)
                    });
                if !valid {
                    return false;
                }
            }
            // A receipt cannot substitute for the weights and processor files.
            // Hash verification runs on load/repair; discovery checks structure
            // without hashing gigabytes every time the settings page polls.
            let all_files = manifest
                .auxiliary_files
                .iter()
                .map(|f| f.filename)
                .chain(std::iter::once(manifest.filename));
            all_files.clone().all(|f| {
                dir.join(f)
                    .metadata()
                    .map(|meta| meta.len() > 0)
                    .unwrap_or(false)
            }) && [
                "config.json",
                "tokenizer_config.json",
                "preprocessor_config.json",
            ]
            .iter()
            .all(|file| nonempty_file(&dir.join(file)))
                && (nonempty_file(&dir.join("tokenizer.json"))
                    || nonempty_file(&dir.join("vocab.json")))
                && Self::get_dir_size(&dir).unwrap_or(0) >= manifest.download_bytes
        } else {
            weights_present(&dir)
        }
    }

    pub fn prepare_repair(&self, id: &str) -> Result<(), String> {
        let manifest = crate::profile::asr_manifest(id)
            .or_else(|| crate::profile::manifest::native_asr_manifest(id))
            .ok_or("Unknown speech model")?;
        let directory = self.get_model_dir(id);
        for (name, hash) in std::iter::once((manifest.filename, manifest.sha256)).chain(
            manifest
                .auxiliary_files
                .iter()
                .map(|file| (file.filename, file.sha256)),
        ) {
            let file = directory.join(name);
            if file.exists() && crate::rewrite::runtime_install::verify_sha256(&file, hash).is_err()
            {
                let quarantine = file.with_extension(format!("corrupt-{}", uuid::Uuid::new_v4()));
                std::fs::rename(&file, quarantine).map_err(|e| {
                    format!("Could not preserve damaged model file before repair: {e}")
                })?;
            }
        }
        Ok(())
    }

    /// `resolved_id` is what the resolver would actually load for the
    /// current settings (`ctx.last_asr_load` at call time): selection is a
    /// ceiling — a legitimate downgrade (1.7B picked, 0.6B resolved) must not
    /// read as stale, but an engine holding anything *else* is.
    pub fn get_status(
        &self,
        engine: &EngineStatus,
        active_id: &str,
        resolved_id: Option<&str>,
    ) -> ModelStatus {
        let spec = spec_for(active_id);
        let model_dir = self.get_model_dir(spec.id);
        let installed = self.is_installed(spec.id);
        let size_bytes = if installed {
            Self::get_dir_size(&model_dir).unwrap_or(0)
        } else {
            0
        };
        let downloading = engine.is_downloading;
        let backend_loading =
            engine.is_loading || engine.backend.to_ascii_lowercase().contains("loading");
        // The engine still holds the previously selected model: reporting it
        // as `loaded` under the new selection's name is exactly how a stale
        // engine got labelled ready. Unknown id (older sidecar) is treated as
        // matching rather than assumed stale.
        let stale_engine = engine.loaded
            && engine
                .model_id
                .as_deref()
                .is_some_and(|loaded_id| loaded_id != active_id && resolved_id != Some(loaded_id));
        let (gpu_name, _) = crate::platform::PlatformSys::detect_gpu();
        let gpu_available = !gpu_name.is_empty() && gpu_name != "CPU";
        // Only surface the `pip install` hint when there's actually a GPU to
        // enable — otherwise the banner is just noise.
        let asr_gpu_hint = if gpu_available && !engine.cuda_available {
            engine.gpu_hint.clone()
        } else {
            None
        };

        ModelStatus {
            selected_model: active_id.into(),
            installed,
            loaded: engine.loaded && !backend_loading && !stale_engine,
            version: spec.label.into(),
            name: spec.repo.to_string(),
            size_bytes,
            download_progress_pct: if downloading {
                engine.download_progress_pct
            } else if installed {
                100
            } else {
                0
            },
            download_speed_mbps: 0.0,
            backend: if engine.backend.is_empty() {
                gpu_name.clone()
            } else {
                engine.backend.clone()
            },
            is_downloading: downloading,
            is_loading: backend_loading && !downloading,
            error: engine.error.clone(),
            gpu_available,
            gpu_name: gpu_name.clone(),
            cuda_available: engine.cuda_available,
            torch_cuda_version: engine.torch_cuda_version.clone(),
            asr_gpu_hint,
            asr_selection_notice: None,
            failure_kind: engine.failure_kind.clone(),
        }
    }

    /// Bytes on disk for a model dir, complete or partial. 0 when absent.
    pub fn size_on_disk(&self, id: &str) -> u64 {
        Self::get_dir_size(&self.get_model_dir(id)).unwrap_or(0)
    }

    pub fn remove_model(&self, id: &str) -> Result<(), String> {
        if !MODELS.iter().any(|model| model.id == id) {
            return Err(format!("Unknown ASR model: {id}"));
        }
        let model_dir = self.get_model_dir(id);
        if model_dir.exists() {
            fs::remove_dir_all(&model_dir)
                .map_err(|e| format!("Failed to remove ASR model '{id}': {e}"))?;
        }
        Ok(())
    }

    fn get_dir_size(path: &Path) -> std::io::Result<u64> {
        let mut total = 0;
        if path.is_dir() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let meta = entry.metadata()?;
                if meta.is_dir() {
                    total += Self::get_dir_size(&entry.path())?;
                } else {
                    total += meta.len();
                }
            }
        }
        Ok(total)
    }
}

pub fn weights_present(model_dir: &Path) -> bool {
    if !model_dir.is_dir() {
        return false;
    }
    let has_config = model_dir.join("config.json").is_file();
    let has_weights = fs::read_dir(model_dir)
        .map(|entries| {
            entries.filter_map(|e| e.ok()).any(|e| {
                let name = e.file_name().to_string_lossy().to_lowercase();
                name.ends_with(".safetensors") || name.ends_with(".bin")
            })
        })
        .unwrap_or(false);
    has_config && has_weights
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asr::engine::EngineStatus;

    #[test]
    fn receipt_only_and_empty_zipformer_are_not_installed() {
        let root = std::env::temp_dir().join(format!(
            "reflow-install-regression-{}",
            uuid::Uuid::new_v4()
        ));
        let manager = ModelManager::new(root.clone());
        let qwen = manager.get_model_dir("0.6b");
        fs::create_dir_all(&qwen).unwrap();
        fs::write(qwen.join("reflow-install.json"), r#"{"model_id":"0.6b"}"#).unwrap();
        assert!(!manager.is_installed("0.6b"));
        let zip = manager.get_model_dir("zipformer-20m");
        fs::create_dir_all(&zip).unwrap();
        let manifest = crate::profile::asr_manifest("zipformer-20m").unwrap();
        for file in std::iter::once(manifest.filename)
            .chain(manifest.auxiliary_files.iter().map(|f| f.filename))
        {
            fs::write(zip.join(file), []).unwrap();
        }
        assert!(!manager.is_installed("zipformer-20m"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn loading_backend_is_not_reported_ready() {
        let dir = std::env::temp_dir().join(format!("reflow_model_{}", uuid::Uuid::new_v4()));
        let mgr = ModelManager::new(dir.clone());
        let engine = EngineStatus {
            loaded: true,
            backend: "loading…".into(),
            is_loading: false,
            ..Default::default()
        };
        let status = mgr.get_status(&engine, "1.7b", None);
        assert!(!status.loaded);
        assert!(status.is_loading);
        assert_eq!(status.name, "Qwen/Qwen3-ASR-1.7B-hf");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn runtime_qualified_ids_pass_through_unchanged() {
        // Callers that pass a concrete dir id (e.g. delete-by-id from the
        // downloaded-models list) must not get a double "native-" prefix.
        assert_eq!(runtime_model_id("native-1.7b", "native"), "native-1.7b");
        assert_eq!(runtime_model_id("native-1.7b", "python"), "native-1.7b");
        assert_eq!(runtime_model_id("0.6b", "native"), "native-0.6b");
        assert_eq!(runtime_model_id("0.6b", "python"), "0.6b");
        assert_eq!(runtime_model_id("phonon-2", "native"), "phonon-2");
    }

    #[test]
    fn removing_unknown_model_preserves_default_model() {
        let dir =
            std::env::temp_dir().join(format!("reflow_model_remove_{}", uuid::Uuid::new_v4()));
        let manager = ModelManager::new(dir.clone());
        let model_dir = manager.get_model_dir("0.6b");
        fs::create_dir_all(&model_dir).unwrap();
        fs::write(model_dir.join("config.json"), "{}").unwrap();
        assert!(manager.remove_model("unknown").is_err());
        assert!(model_dir.join("config.json").is_file());
        manager.remove_model("0.6b").unwrap();
        assert!(!model_dir.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn qwen_install_requires_manifest_completeness() {
        let dir =
            std::env::temp_dir().join(format!("reflow_model_complete_{}", uuid::Uuid::new_v4()));
        let manager = ModelManager::new(dir.clone());
        let model_dir = manager.get_model_dir("0.6b");
        fs::create_dir_all(&model_dir).unwrap();
        fs::write(model_dir.join("config.json"), "{}").unwrap();

        // config.json alone, and config + truncated weights, are partial.
        assert!(!manager.is_installed("0.6b"));
        let weights = model_dir.join("model.safetensors");
        fs::write(&weights, b"partial").unwrap();
        assert!(!manager.is_installed("0.6b"));

        for file in [
            "tokenizer_config.json",
            "preprocessor_config.json",
            "tokenizer.json",
        ] {
            fs::write(model_dir.join(file), "{}").unwrap();
        }
        // A sparse file at the manifest's expected bytes completes the structural install check.
        let expected = crate::profile::asr_manifest("0.6b").unwrap().download_bytes;
        fs::File::create(&weights)
            .unwrap()
            .set_len(expected)
            .unwrap();
        assert!(manager.is_installed("0.6b"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_loaded_engine_under_another_label_is_not_ready() {
        let dir = std::env::temp_dir().join(format!("reflow_model_stale_{}", uuid::Uuid::new_v4()));
        let manager = ModelManager::new(dir.clone());
        let engine = EngineStatus {
            loaded: true,
            backend: "Qwen3-ASR 0.6B · CPU FP32".into(),
            model_id: Some("0.6b".into()),
            ..Default::default()
        };
        let status = manager.get_status(&engine, "1.7b", None);
        assert_eq!(status.selected_model, "1.7b");
        assert!(
            !status.loaded,
            "the resident 0.6B must not read as a ready 1.7B"
        );
        assert!(!status.is_loading, "no reload is in flight");
        assert_eq!(status.version, "1.7B");

        // Engines that do not report an id (older sidecars) still count as
        // matching — unknown is not stale.
        let engine = EngineStatus {
            loaded: true,
            backend: "Qwen3-ASR 1.7B · CPU FP32".into(),
            ..Default::default()
        };
        assert!(manager.get_status(&engine, "1.7b", None).loaded);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_resolver_downgrade_counts_as_loaded() {
        let dir = std::env::temp_dir().join(format!("reflow_model_down_{}", uuid::Uuid::new_v4()));
        let manager = ModelManager::new(dir.clone());
        // Selected 1.7B, resolver downgraded to 0.6B, engine loaded 0.6B —
        // that is the resolved target, not staleness.
        let engine = EngineStatus {
            loaded: true,
            backend: "Qwen3-ASR 0.6B · CPU FP32".into(),
            model_id: Some("0.6b".into()),
            ..Default::default()
        };
        let status = manager.get_status(&engine, "1.7b", Some("0.6b"));
        assert!(status.loaded, "the resolved model must count as ready");
        // A resolved id is only evidence while it was resolved for the
        // *current* selection: the callers pass `None` when last_asr_load
        // belongs to a different (older or newer) selection, and then the
        // engine's model must not be credited to the active one.
        let previous_selection_resolved = manager.get_status(&engine, "1.7b", None);
        assert!(
            !previous_selection_resolved.loaded,
            "a resolution made for another selection must not bless this engine"
        );
        // An engine holding a third, unrelated model is still stale.
        let engine = EngineStatus {
            loaded: true,
            backend: "Phonon-2".into(),
            model_id: Some("phonon-2".into()),
            ..Default::default()
        };
        assert!(!manager.get_status(&engine, "1.7b", Some("0.6b")).loaded);
        let _ = std::fs::remove_dir_all(dir);
    }
}
