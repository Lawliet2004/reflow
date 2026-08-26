use std::fs;
use std::path::{Path, PathBuf};
use crate::asr::engine::EngineStatus;
use crate::state::ModelStatus;

pub struct ModelSpec {
    pub id: &'static str,        // "0.6b" | "1.7b"
    pub dir_name: &'static str,  // qwen3-asr-0.6b
    pub repo: &'static str,      // Qwen/Qwen3-ASR-0.6B-hf
    pub label: &'static str,     // "0.6B · Realtime"
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
];

pub fn spec_for(id: &str) -> &'static ModelSpec {
    MODELS
        .iter()
        .find(|m| m.id == id)
        .unwrap_or(&MODELS[0])
}

pub struct ModelManager {
    models_dir: PathBuf,
}

impl ModelManager {
    pub fn new(models_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&models_dir);
        Self { models_dir }
    }

    pub fn get_model_dir(&self, id: &str) -> PathBuf {
        self.models_dir.join(spec_for(id).dir_name)
    }

    /// Legacy helper: directory of the active model.
    pub fn get_qwen3_dir(&self) -> PathBuf {
        self.get_qwen3_dir_for("0.6b")
    }

    pub fn get_qwen3_dir_for(&self, id: &str) -> PathBuf {
        self.get_model_dir(id)
    }

    pub fn repo_for(&self, id: &str) -> &'static str {
        spec_for(id).repo
    }

    /// Weights are considered installed when a real HF model directory is there.
    pub fn is_installed(&self, id: &str) -> bool {
        weights_present(&self.get_model_dir(id))
    }

    pub fn get_status(&self, engine: &EngineStatus, active_id: &str) -> ModelStatus {
        let spec = spec_for(active_id);
        let model_dir = self.get_model_dir(spec.id);
        let installed = self.is_installed(spec.id);
        let size_bytes = if installed {
            Self::get_dir_size(&model_dir).unwrap_or(0)
        } else {
            0
        };
        let downloading = engine.is_downloading;
        let backend_loading = engine.is_loading
            || engine.backend.to_ascii_lowercase().contains("loading");
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
            installed,
            loaded: engine.loaded && !backend_loading,
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
        }
    }

    pub fn remove_model(&self, id: &str) -> Result<(), String> {
        let model_dir = self.get_model_dir(id);
        if model_dir.exists() {
            let _ = fs::remove_dir_all(&model_dir);
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
            entries
                .filter_map(|e| e.ok())
                .any(|e| {
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
    fn loading_backend_is_not_reported_ready() {
        let dir = std::env::temp_dir().join(format!("reflow_model_{}", uuid::Uuid::new_v4()));
        let mgr = ModelManager::new(dir.clone());
        let engine = EngineStatus {
            loaded: true,
            backend: "loading…".into(),
            is_loading: false,
            ..Default::default()
        };
        let status = mgr.get_status(&engine, "1.7b");
        assert!(!status.loaded);
        assert!(status.is_loading);
        assert_eq!(status.name, "Qwen/Qwen3-ASR-1.7B-hf");
        let _ = std::fs::remove_dir_all(dir);
    }
}
