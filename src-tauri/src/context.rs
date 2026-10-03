use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use tokio::sync::mpsc;

use crate::asr::AsrHandle;
use crate::audio::{AudioCaptureEngine, AudioRingBuffer};
use crate::dory::{CaptureKind, DoryBus};
use crate::history::HistoryStore;
use crate::model::ModelManager;
use crate::pairing::PairingState;
use crate::platform::PlatformSys;
use crate::rewrite::FlowRuntime;
use crate::settings::SettingsStore;
use crate::state::{AppStateEnum, LatencyHistory, LatencyMetrics, LatencyTimer};

#[derive(Clone)]
pub struct ApiRuntime {
    pub bind: String,
    pub shutdown: tokio::sync::watch::Sender<bool>,
    pub finished: tokio::sync::watch::Receiver<bool>,
    pub generation: u64,
}

/// Why the refinement runtime needs recovering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRecoveryRequest {
    /// The user's compute-backend preference, which decides whether a CPU or
    /// CUDA build is fetched.
    pub compute_backend: String,
    /// The error `FlowRuntime::ensure` returned.
    pub reason: String,
}

/// Repairs a missing or broken refinement runtime.
///
/// Boxed so the session layer never names a GUI type.
pub type RuntimeRecoveryHook = Arc<dyn Fn(RuntimeRecoveryRequest) + Send + Sync + 'static>;

#[derive(Clone)]
pub struct AppContext {
    pub state_enum: Arc<RwLock<AppStateEnum>>,
    pub settings_store: Arc<SettingsStore>,
    /// Keep persisted preferences and their storage side effects in order.
    pub settings_operation: Arc<Mutex<()>>,
    pub history_store: Arc<HistoryStore>,
    pub model_manager: Arc<ModelManager>,
    pub audio_engine: Arc<RwLock<AudioCaptureEngine>>,
    pub asr_handle: AsrHandle,
    pub asr_runtime: Arc<parking_lot::Mutex<String>>,
    pub file_jobs: Arc<RwLock<std::collections::HashMap<String, crate::file_jobs::FileJob>>>,
    pub session_operation: Arc<tokio::sync::Mutex<()>>,
    pub session_cancel: Arc<RwLock<Option<tokio::sync::watch::Sender<bool>>>>,
    pub session_phone_token: Arc<RwLock<Option<String>>>,
    pub current_session_id: Arc<RwLock<Option<u64>>>,
    pub session_intent: Arc<RwLock<crate::dory::SessionIntent>>,
    pub session_mode: Arc<RwLock<Option<crate::settings::Mode>>>,
    pub session_context: Arc<RwLock<crate::session::SessionContext>>,
    pub latency_timer: Arc<RwLock<LatencyTimer>>,
    pub last_latency_metrics: Arc<RwLock<LatencyMetrics>>,
    /// Rolling window of completed dictations, so the diagnostics view can
    /// report p50/p95 instead of a single unrepresentative sample.
    pub latency_history: Arc<RwLock<LatencyHistory>>,
    pub latency_report_path: std::path::PathBuf,
    pub recording_sample_sender: Arc<RwLock<Option<mpsc::Sender<Vec<f32>>>>>,
    pub recording_pcm: Arc<parking_lot::Mutex<AudioRingBuffer>>,
    /// Resolved by the sample loop once every captured chunk has been handed
    /// to the ASR engine. The stop path awaits this instead of sleeping, so
    /// the final fraction of a second of speech cannot be dropped by racing
    /// `stop_stream()`.
    pub audio_drain_done: Arc<Mutex<Option<tokio::sync::oneshot::Receiver<()>>>>,
    /// Invoked when the refinement runtime turns out to be missing or broken.
    ///
    /// A callback rather than a `tauri::AppHandle` parameter threaded through
    /// `session::stop`: the session layer must not depend on the GUI runtime.
    /// That coupling also made every session test link the whole Tauri surface.
    /// `lib.rs` installs the real implementation at startup; tests leave it
    /// `None` and let the fallback formatting path take over.
    pub runtime_recovery: Arc<RwLock<Option<RuntimeRecoveryHook>>>,
    pub dictation_target_hwnd: Arc<RwLock<isize>>,
    pub registered_hotkey: Arc<RwLock<String>>,
    pub hotkey_error: Arc<RwLock<Option<String>>>,
    pub bus: DoryBus,
    pub capture_kind: Arc<RwLock<CaptureKind>>,
    pub last_audio_level: Arc<RwLock<f32>>,
    pub pairing: Arc<PairingState>,
    pub api_runtime: Arc<RwLock<Option<ApiRuntime>>>,
    pub api_operation: Arc<tokio::sync::Mutex<()>>,
    pub api_jobs: Arc<tokio::sync::Semaphore>,
    pub api_identity: Arc<std::sync::OnceLock<Result<crate::api::tls::LanIdentity, String>>>,
    pub api_identity_path: Arc<std::path::PathBuf>,
    pub flow_runtime: Arc<FlowRuntime>,
    pub active_intelligence_downloads: Arc<Mutex<HashSet<String>>>,
    pub active_runtime_downloads: Arc<Mutex<HashSet<String>>>,
    /// Guards the single long-lived model-status poller.
    ///
    /// The poller is the only thing that refreshes `asr_handle`'s status
    /// cache, and `get_model_status` serves that cache directly. It therefore
    /// has to keep running for the life of the app: when it used to exit after
    /// a bounded number of ticks, nothing ever polled the sidecar again and the
    /// UI showed whatever the last tick saw — permanently. This flag lets
    /// `install_model` / `reload_model` ask for the poller without stacking a
    /// new one on every call.
    pub model_status_watch_active: Arc<AtomicBool>,
    /// Why the loaded ASR model differs from the one chosen in Settings, if it
    /// does.
    ///
    /// A silent substitution is worse than a slow one: the user needs to know
    /// that a smaller model is running because the bigger one did not fit, or
    /// they will read the accuracy drop as a bug.
    pub asr_selection_notice: Arc<RwLock<Option<String>>>,
    /// The (model, device, precision) the resolver last chose for a load —
    /// the *resolved* triple, not the Settings one. The status poller records
    /// measured peaks against it once the sidecar reports them.
    pub last_asr_load: Arc<RwLock<Option<(String, String, String)>>>,
}

impl AppContext {
    pub fn bootstrap() -> Self {
        let db_path = PlatformSys::get_db_path();
        let config_path = PlatformSys::get_config_path();
        let models_dir = PlatformSys::get_models_dir();
        let _ = std::fs::create_dir_all(PlatformSys::get_logs_dir());

        let settings_store = Arc::new(SettingsStore::new(config_path));
        let initial_settings = settings_store.get();
        if let Err(error) = crate::network_policy::configure(
            initial_settings.offline_mode,
            &PlatformSys::get_app_dir(),
        ) {
            log::error!("Network policy could not be saved: {error}");
        }
        let history_store = Arc::new(HistoryStore::open_recovering(db_path));
        if let Err(error) = history_store.set_audio_retention(&initial_settings.audio_retention) {
            log::error!("Could not apply audio retention: {error}");
        }
        history_store.require_encryption(initial_settings.history_encryption);
        if let Err(error) = history_store.set_encryption(initial_settings.history_encryption) {
            log::error!("History encryption could not be applied: {error}");
        }
        let model_manager = Arc::new(ModelManager::new(models_dir));
        let pairing_path = PlatformSys::get_app_dir()
            .join("config")
            .join("api-devices.json");

        Self {
            state_enum: Arc::new(RwLock::new(AppStateEnum::Ready)),
            settings_store,
            settings_operation: Arc::new(Mutex::new(())),
            history_store,
            model_manager,
            audio_engine: Arc::new(RwLock::new(AudioCaptureEngine::new())),
            asr_handle: AsrHandle::new_runtime(&initial_settings.asr.runtime),
            asr_runtime: Arc::new(parking_lot::Mutex::new(
                initial_settings.asr.runtime.clone(),
            )),
            file_jobs: Arc::new(RwLock::new(Default::default())),
            session_operation: Arc::new(tokio::sync::Mutex::new(())),
            session_cancel: Arc::new(RwLock::new(None)),
            session_phone_token: Arc::new(RwLock::new(None)),
            current_session_id: Arc::new(RwLock::new(None)),
            session_intent: Arc::new(RwLock::new(crate::dory::SessionIntent::Dictate)),
            session_mode: Arc::new(RwLock::new(None)),
            session_context: Arc::new(RwLock::new(crate::session::SessionContext::default())),
            latency_timer: Arc::new(RwLock::new(LatencyTimer::default())),
            last_latency_metrics: Arc::new(RwLock::new(LatencyMetrics::default())),
            latency_history: Arc::new(RwLock::new(LatencyHistory::default())),
            latency_report_path: crate::state::latency_report_path(),
            recording_sample_sender: Arc::new(RwLock::new(None)),
            recording_pcm: Arc::new(parking_lot::Mutex::new(AudioRingBuffer::default())),
            audio_drain_done: Arc::new(Mutex::new(None)),
            runtime_recovery: Arc::new(RwLock::new(None)),
            dictation_target_hwnd: Arc::new(RwLock::new(0)),
            registered_hotkey: Arc::new(RwLock::new(initial_settings.hotkey)),
            hotkey_error: Arc::new(RwLock::new(None)),
            bus: DoryBus::new(),
            capture_kind: Arc::new(RwLock::new(CaptureKind::None)),
            last_audio_level: Arc::new(RwLock::new(0.0)),
            pairing: Arc::new(PairingState::new(pairing_path)),
            api_runtime: Arc::new(RwLock::new(None)),
            api_operation: Arc::new(tokio::sync::Mutex::new(())),
            api_jobs: Arc::new(tokio::sync::Semaphore::new(4)),
            api_identity: Arc::new(std::sync::OnceLock::new()),
            api_identity_path: Arc::new(
                PlatformSys::get_app_dir()
                    .join("config")
                    .join("lan-identity.json"),
            ),
            flow_runtime: Arc::new(FlowRuntime::default()),
            active_intelligence_downloads: Arc::new(Mutex::new(HashSet::new())),
            active_runtime_downloads: Arc::new(Mutex::new(HashSet::new())),
            model_status_watch_active: Arc::new(AtomicBool::new(false)),
            asr_selection_notice: Arc::new(RwLock::new(None)),
            last_asr_load: Arc::new(RwLock::new(None)),
        }
    }

    /// Build a context rooted at `dir` with the mock ASR engine, for tests.
    ///
    /// Deliberately not `#[cfg(test)]`: integration tests in `tests/` link the
    /// library as an external crate and cannot see `cfg(test)` items, and the
    /// session/telemetry paths are only meaningfully testable end to end.
    pub fn bootstrap_test(dir: std::path::PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        let settings_store = Arc::new(SettingsStore::new(dir.join("settings.json")));
        let history_store = Arc::new(HistoryStore::new(dir.join("history.db")).expect("test db"));
        let initial = settings_store.get();
        Self {
            state_enum: Arc::new(RwLock::new(AppStateEnum::Ready)),
            settings_store,
            settings_operation: Arc::new(Mutex::new(())),
            history_store,
            model_manager: Arc::new(ModelManager::new(dir.join("models"))),
            audio_engine: Arc::new(RwLock::new(AudioCaptureEngine::new())),
            asr_handle: AsrHandle::new_mock(),
            asr_runtime: Arc::new(parking_lot::Mutex::new("python".into())),
            file_jobs: Arc::new(RwLock::new(Default::default())),
            session_operation: Arc::new(tokio::sync::Mutex::new(())),
            session_cancel: Arc::new(RwLock::new(None)),
            session_phone_token: Arc::new(RwLock::new(None)),
            current_session_id: Arc::new(RwLock::new(None)),
            session_intent: Arc::new(RwLock::new(crate::dory::SessionIntent::Dictate)),
            session_mode: Arc::new(RwLock::new(None)),
            session_context: Arc::new(RwLock::new(crate::session::SessionContext::default())),
            latency_timer: Arc::new(RwLock::new(LatencyTimer::default())),
            last_latency_metrics: Arc::new(RwLock::new(LatencyMetrics::default())),
            latency_history: Arc::new(RwLock::new(LatencyHistory::default())),
            latency_report_path: dir.join("latency-report.json"),
            recording_sample_sender: Arc::new(RwLock::new(None)),
            recording_pcm: Arc::new(parking_lot::Mutex::new(AudioRingBuffer::default())),
            audio_drain_done: Arc::new(Mutex::new(None)),
            runtime_recovery: Arc::new(RwLock::new(None)),
            dictation_target_hwnd: Arc::new(RwLock::new(0)),
            registered_hotkey: Arc::new(RwLock::new(initial.hotkey)),
            hotkey_error: Arc::new(RwLock::new(None)),
            bus: DoryBus::new(),
            capture_kind: Arc::new(RwLock::new(CaptureKind::None)),
            last_audio_level: Arc::new(RwLock::new(0.0)),
            pairing: Arc::new(PairingState::new(dir.join("api-devices.json"))),
            api_runtime: Arc::new(RwLock::new(None)),
            api_operation: Arc::new(tokio::sync::Mutex::new(())),
            api_jobs: Arc::new(tokio::sync::Semaphore::new(4)),
            api_identity: Arc::new(std::sync::OnceLock::new()),
            api_identity_path: Arc::new(dir.join("lan-identity.json")),
            flow_runtime: Arc::new(FlowRuntime::default()),
            active_intelligence_downloads: Arc::new(Mutex::new(HashSet::new())),
            active_runtime_downloads: Arc::new(Mutex::new(HashSet::new())),
            model_status_watch_active: Arc::new(AtomicBool::new(false)),
            asr_selection_notice: Arc::new(RwLock::new(None)),
            last_asr_load: Arc::new(RwLock::new(None)),
        }
    }
}
