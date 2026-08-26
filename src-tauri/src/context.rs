use std::collections::HashSet;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use tokio::sync::mpsc;

use crate::asr::{ASREngine, Qwen3AsrSidecar};
use crate::audio::AudioCaptureEngine;
use crate::dory::{CaptureKind, DoryBus};
use crate::history::HistoryStore;
use crate::model::ModelManager;
use crate::pairing::PairingState;
use crate::platform::PlatformSys;
use crate::rewrite::FlowRuntime;
use crate::settings::SettingsStore;
use crate::state::{AppStateEnum, LatencyMetrics, LatencyTimer};

#[derive(Clone)]
pub struct ApiRuntime {
    pub bind: String,
    pub shutdown: tokio::sync::watch::Sender<bool>,
}

#[derive(Clone)]
pub struct AppContext {
    pub state_enum: Arc<RwLock<AppStateEnum>>,
    pub settings_store: Arc<SettingsStore>,
    pub history_store: Arc<HistoryStore>,
    pub model_manager: Arc<ModelManager>,
    pub audio_engine: Arc<RwLock<AudioCaptureEngine>>,
    pub asr_engine: Arc<RwLock<Box<dyn ASREngine>>>,
    pub latency_timer: Arc<RwLock<LatencyTimer>>,
    pub last_latency_metrics: Arc<RwLock<LatencyMetrics>>,
    pub recording_sample_sender: Arc<RwLock<Option<mpsc::UnboundedSender<Vec<f32>>>>>,
    pub recording_pcm: Arc<parking_lot::Mutex<Vec<f32>>>,
    pub dictation_target_hwnd: Arc<RwLock<isize>>,
    pub registered_hotkey: Arc<RwLock<String>>,
    pub hotkey_error: Arc<RwLock<Option<String>>>,
    pub bus: DoryBus,
    pub capture_kind: Arc<RwLock<CaptureKind>>,
    pub last_audio_level: Arc<RwLock<f32>>,
    pub pairing: Arc<PairingState>,
    pub api_runtime: Arc<RwLock<Option<ApiRuntime>>>,
    pub flow_runtime: Arc<FlowRuntime>,
    pub active_intelligence_downloads: Arc<Mutex<HashSet<String>>>,
    pub active_runtime_downloads: Arc<Mutex<HashSet<String>>>,
}

impl AppContext {
    pub fn bootstrap() -> Self {
        let db_path = PlatformSys::get_db_path();
        let config_path = PlatformSys::get_config_path();
        let models_dir = PlatformSys::get_models_dir();
        let _ = std::fs::create_dir_all(PlatformSys::get_logs_dir());

        let settings_store = Arc::new(SettingsStore::new(config_path));
        let initial_settings = settings_store.get();
        let history_store = Arc::new(
            HistoryStore::new(db_path).expect("Failed to initialize SQLite history database"),
        );
        let model_manager = Arc::new(ModelManager::new(models_dir));
        let pairing_path = PlatformSys::get_app_dir().join("config").join("api-devices.json");

        Self {
            state_enum: Arc::new(RwLock::new(AppStateEnum::Ready)),
            settings_store,
            history_store,
            model_manager,
            audio_engine: Arc::new(RwLock::new(AudioCaptureEngine::new())),
            asr_engine: Arc::new(RwLock::new(Box::new(Qwen3AsrSidecar::new()))),
            latency_timer: Arc::new(RwLock::new(LatencyTimer::default())),
            last_latency_metrics: Arc::new(RwLock::new(LatencyMetrics::default())),
            recording_sample_sender: Arc::new(RwLock::new(None)),
            recording_pcm: Arc::new(parking_lot::Mutex::new(Vec::new())),
            dictation_target_hwnd: Arc::new(RwLock::new(0)),
            registered_hotkey: Arc::new(RwLock::new(initial_settings.hotkey)),
            hotkey_error: Arc::new(RwLock::new(None)),
            bus: DoryBus::new(),
            capture_kind: Arc::new(RwLock::new(CaptureKind::None)),
            last_audio_level: Arc::new(RwLock::new(0.0)),
            pairing: Arc::new(PairingState::new(pairing_path)),
            api_runtime: Arc::new(RwLock::new(None)),
            flow_runtime: Arc::new(FlowRuntime::default()),
            active_intelligence_downloads: Arc::new(Mutex::new(HashSet::new())),
            active_runtime_downloads: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    #[cfg(test)]
    pub fn bootstrap_test(dir: std::path::PathBuf) -> Self {
        use crate::asr::MockASREngine;

        let _ = std::fs::create_dir_all(&dir);
        let settings_store = Arc::new(SettingsStore::new(dir.join("settings.json")));
        let history_store = Arc::new(HistoryStore::new(dir.join("history.db")).expect("test db"));
        let initial = settings_store.get();
        Self {
            state_enum: Arc::new(RwLock::new(AppStateEnum::Ready)),
            settings_store,
            history_store,
            model_manager: Arc::new(ModelManager::new(dir.join("models"))),
            audio_engine: Arc::new(RwLock::new(AudioCaptureEngine::new())),
            asr_engine: Arc::new(RwLock::new(Box::new(MockASREngine::new()))),
            latency_timer: Arc::new(RwLock::new(LatencyTimer::default())),
            last_latency_metrics: Arc::new(RwLock::new(LatencyMetrics::default())),
            recording_sample_sender: Arc::new(RwLock::new(None)),
            recording_pcm: Arc::new(parking_lot::Mutex::new(Vec::new())),
            dictation_target_hwnd: Arc::new(RwLock::new(0)),
            registered_hotkey: Arc::new(RwLock::new(initial.hotkey)),
            hotkey_error: Arc::new(RwLock::new(None)),
            bus: DoryBus::new(),
            capture_kind: Arc::new(RwLock::new(CaptureKind::None)),
            last_audio_level: Arc::new(RwLock::new(0.0)),
            pairing: Arc::new(PairingState::new(dir.join("api-devices.json"))),
            api_runtime: Arc::new(RwLock::new(None)),
            flow_runtime: Arc::new(FlowRuntime::default()),
            active_intelligence_downloads: Arc::new(Mutex::new(HashSet::new())),
            active_runtime_downloads: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}
