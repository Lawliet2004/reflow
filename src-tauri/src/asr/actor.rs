use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::{mpsc, oneshot};

use super::engine::{ASREngine, EngineStatus};
use super::mock::MockASREngine;
use super::sidecar::Qwen3AsrSidecar;

type InferenceCancel = Arc<RwLock<Option<(u64, super::engine::InferenceCancellation)>>>;

pub const DEFAULT_ASR_CHANNEL_CAPACITY: usize = 64;

/// Dedicated actor thread that owns the ASR engine instance.
pub struct AsrActor;

/// Polymorphic reply channel supporting async oneshot and sync std::sync::mpsc channels.
pub enum Reply<T: Send + 'static> {
    Async(oneshot::Sender<T>),
    Sync(std::sync::mpsc::Sender<T>),
    None,
}

impl<T: Send + 'static> Reply<T> {
    pub fn send(self, val: T) {
        match self {
            Reply::Async(tx) => {
                let _ = tx.send(val);
            }
            Reply::Sync(tx) => {
                let _ = tx.send(val);
            }
            Reply::None => {}
        }
    }
}

/// Messages sent to the dedicated ASR actor thread.
pub enum AsrCommand {
    Initialize {
        reply: Reply<Result<(), String>>,
    },
    /// Ask the engine to compute its CUDA capability snapshot. See
    /// [`ASREngine::probe_cuda`].
    ProbeCuda {
        reply: Reply<Result<(), String>>,
    },
    LoadModel {
        model_dir: String,
        backend: String,
        precision: String,
        reply: Reply<Result<(), String>>,
    },
    UnloadModel {
        reply: Reply<Result<(), String>>,
    },
    StartStream {
        session_id: u64,
        language: String,
        vocabulary: Vec<String>,
        reply: Reply<Result<(), String>>,
    },
    PushAudio {
        session_id: u64,
        sequence_id: u32,
        samples: Vec<f32>,
        reply: Reply<Result<Option<String>, String>>,
    },
    GetPartialTranscript {
        session_id: u64,
        reply: Reply<Result<String, String>>,
    },
    StopStream {
        session_id: u64,
        reply: Reply<Result<String, String>>,
    },
    CancelStream {
        session_id: u64,
        reply: Reply<Result<(), String>>,
    },
    InstallModelDir {
        model_dir: String,
        repo: String,
        backend: String,
        precision: String,
        reply: Reply<Result<(), String>>,
    },
    SetResourceDir {
        dir: PathBuf,
        reply: Reply<()>,
    },
    RefreshStatus {
        reply: Reply<EngineStatus>,
    },
    SwapEngine {
        engine: Box<dyn ASREngine>,
        reply: Reply<()>,
    },
}

/// Cloneable handle to the ASR actor.
///
/// Owns a bounded `mpsc::Sender<AsrCommand>` and maintains atomic/cached snapshots
/// of engine status and detected language so status queries never block on in-flight
/// inference commands.
#[derive(Clone)]
pub struct AsrHandle {
    sender: mpsc::Sender<AsrCommand>,
    status_cache: Arc<RwLock<EngineStatus>>,
    detected_language: Arc<RwLock<String>>,
    backend_name: Arc<RwLock<String>>,
    last_warning: Arc<RwLock<Option<String>>>,
    session_counter: Arc<AtomicU64>,
    inference_cancel: InferenceCancel,
    progress: super::engine::ProgressTracker,
}

impl AsrHandle {
    /// Spawn an actor managing the given `engine` with the specified bounded channel `capacity`.
    pub fn spawn(mut engine: Box<dyn ASREngine>, capacity: usize) -> Self {
        let (sender, mut receiver) = mpsc::channel::<AsrCommand>(capacity);
        let status_cache = Arc::new(RwLock::new(engine.engine_status()));
        let detected_language = Arc::new(RwLock::new(engine.get_detected_language()));
        let backend_name = Arc::new(RwLock::new(engine.get_backend_name()));
        let last_warning = Arc::new(RwLock::new(engine.take_last_warning()));
        let session_counter = Arc::new(AtomicU64::new(1));
        let inference_cancel: InferenceCancel = Arc::new(RwLock::new(None));
        let cancel_c = Arc::clone(&inference_cancel);
        let progress = Arc::new(RwLock::new(super::engine::InferenceProgress::default()));
        engine.set_progress_tracker(progress.clone());
        let progress_c = progress.clone();

        let status_c = Arc::clone(&status_cache);
        let lang_c = Arc::clone(&detected_language);
        let backend_c = Arc::clone(&backend_name);
        let warning_c = Arc::clone(&last_warning);

        std::thread::Builder::new()
            .name("asr-actor".into())
            .spawn(move || {
                let mut active_session_id: Option<u64> = None;
                let mut cancelled_sessions: std::collections::HashSet<u64> =
                    std::collections::HashSet::new();
                while let Some(cmd) = receiver.blocking_recv() {
                    match cmd {
                        AsrCommand::Initialize { reply } => {
                            let res = engine.initialize();
                            Self::sync_cache(
                                &mut *engine,
                                &status_c,
                                &lang_c,
                                &backend_c,
                                &warning_c,
                            );
                            reply.send(res);
                        }
                        AsrCommand::ProbeCuda { reply } => {
                            let res = engine.probe_cuda();
                            reply.send(res);
                        }
                        AsrCommand::LoadModel {
                            model_dir,
                            backend,
                            precision,
                            reply,
                        } => {
                            let res =
                                engine.load_model_with_precision(&model_dir, &backend, &precision);
                            Self::sync_cache(
                                &mut *engine,
                                &status_c,
                                &lang_c,
                                &backend_c,
                                &warning_c,
                            );
                            reply.send(res);
                        }
                        AsrCommand::UnloadModel { reply } => {
                            let res = engine.unload_model();
                            Self::sync_cache(
                                &mut *engine,
                                &status_c,
                                &lang_c,
                                &backend_c,
                                &warning_c,
                            );
                            reply.send(res);
                        }
                        AsrCommand::StartStream {
                            session_id,
                            language,
                            vocabulary,
                            reply,
                        } => {
                            cancelled_sessions.remove(&session_id);
                            *progress_c.write() = super::engine::InferenceProgress {
                                session_id: Some(session_id),
                                completed: 0,
                                total: 0,
                            };
                            active_session_id = Some(session_id);
                            let res = engine.start_stream(&language, &vocabulary);
                            *cancel_c.write() = if res.is_ok() {
                                engine
                                    .cancellation_signal()
                                    .map(|signal| (session_id, signal))
                            } else {
                                None
                            };
                            Self::sync_cache(
                                &mut *engine,
                                &status_c,
                                &lang_c,
                                &backend_c,
                                &warning_c,
                            );
                            reply.send(res);
                        }
                        AsrCommand::PushAudio {
                            session_id,
                            sequence_id: _,
                            samples,
                            reply,
                        } => {
                            // Drop superseded or cancelled session's commands immediately without dispatching
                            if active_session_id != Some(session_id)
                                || cancelled_sessions.contains(&session_id)
                            {
                                reply.send(Ok(None));
                                continue;
                            }
                            let res = engine.push_audio(&samples);
                            reply.send(res);
                        }
                        AsrCommand::GetPartialTranscript { session_id, reply } => {
                            if active_session_id != Some(session_id)
                                || cancelled_sessions.contains(&session_id)
                            {
                                reply.send(Ok(String::new()));
                                continue;
                            }
                            let res = engine.get_partial_transcript();
                            reply.send(res);
                        }
                        AsrCommand::StopStream { session_id, reply } => {
                            if active_session_id != Some(session_id)
                                || cancelled_sessions.contains(&session_id)
                            {
                                reply.send(Ok(String::new()));
                                continue;
                            }
                            active_session_id = None;
                            let res = engine.stop_stream();
                            *cancel_c.write() = None;
                            Self::sync_cache(
                                &mut *engine,
                                &status_c,
                                &lang_c,
                                &backend_c,
                                &warning_c,
                            );
                            reply.send(res);
                        }
                        AsrCommand::CancelStream { session_id, reply } => {
                            if active_session_id != Some(session_id) {
                                reply.send(Ok(()));
                                continue;
                            }
                            cancelled_sessions.clear();
                            cancelled_sessions.insert(session_id);
                            active_session_id = None;
                            let res = engine.cancel_stream();
                            *cancel_c.write() = None;
                            Self::sync_cache(
                                &mut *engine,
                                &status_c,
                                &lang_c,
                                &backend_c,
                                &warning_c,
                            );
                            reply.send(res);
                        }
                        AsrCommand::InstallModelDir {
                            model_dir,
                            repo,
                            backend,
                            precision,
                            reply,
                        } => {
                            let res = engine.install_model_dir_with_options(
                                &model_dir, &repo, &backend, &precision,
                            );
                            Self::sync_cache(
                                &mut *engine,
                                &status_c,
                                &lang_c,
                                &backend_c,
                                &warning_c,
                            );
                            reply.send(res);
                        }
                        AsrCommand::SetResourceDir { dir, reply } => {
                            engine.set_resource_dir(dir);
                            reply.send(());
                        }
                        AsrCommand::RefreshStatus { reply } => {
                            let status = engine.engine_status();
                            *status_c.write() = status.clone();
                            reply.send(status);
                        }
                        AsrCommand::SwapEngine {
                            engine: new_engine,
                            reply,
                        } => {
                            *cancel_c.write() = None;
                            engine = new_engine;
                            engine.set_progress_tracker(progress_c.clone());
                            *progress_c.write() = super::engine::InferenceProgress::default();
                            active_session_id = None;
                            Self::sync_cache(
                                &mut *engine,
                                &status_c,
                                &lang_c,
                                &backend_c,
                                &warning_c,
                            );
                            reply.send(());
                        }
                    }
                }
            })
            .expect("failed to spawn asr-actor thread");

        Self {
            sender,
            status_cache,
            detected_language,
            backend_name,
            last_warning,
            session_counter,
            inference_cancel,
            progress,
        }
    }

    fn sync_cache(
        engine: &mut dyn ASREngine,
        status_cache: &RwLock<EngineStatus>,
        detected_language: &RwLock<String>,
        backend_name: &RwLock<String>,
        last_warning: &RwLock<Option<String>>,
    ) {
        *status_cache.write() = engine.engine_status();
        *detected_language.write() = engine.get_detected_language();
        *backend_name.write() = engine.get_backend_name();
        if let Some(w) = engine.take_last_warning() {
            *last_warning.write() = Some(w);
        }
    }

    pub fn inference_progress(&self) -> super::engine::InferenceProgress {
        self.progress.read().clone()
    }

    pub fn new_runtime(runtime: &str) -> Self {
        if runtime == "native" {
            Self::spawn(
                Box::new(super::native::NativeAsrEngine::default()),
                DEFAULT_ASR_CHANNEL_CAPACITY,
            )
        } else {
            Self::new_sidecar()
        }
    }

    pub fn new_sidecar() -> Self {
        Self::spawn(
            Box::new(Qwen3AsrSidecar::new()),
            DEFAULT_ASR_CHANNEL_CAPACITY,
        )
    }

    pub fn new_mock() -> Self {
        Self::spawn(Box::new(MockASREngine::new()), DEFAULT_ASR_CHANNEL_CAPACITY)
    }

    pub fn next_session_id(&self) -> u64 {
        self.session_counter.fetch_add(1, Ordering::SeqCst)
    }

    pub fn engine_status(&self) -> EngineStatus {
        self.status_cache.read().clone()
    }

    pub fn is_model_loaded(&self) -> bool {
        self.status_cache.read().loaded
    }

    pub fn get_detected_language(&self) -> String {
        self.detected_language.read().clone()
    }

    pub fn get_backend_name(&self) -> String {
        self.backend_name.read().clone()
    }

    pub fn take_last_warning(&self) -> Option<String> {
        self.last_warning.write().take()
    }

    pub async fn initialize(&self) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(AsrCommand::Initialize {
                reply: Reply::Async(reply),
            })
            .await
            .map_err(|e| format!("ASR actor disconnected: {e}"))?;
        rx.await
            .map_err(|e| format!("ASR actor dropped reply: {e}"))?
    }

    /// Fire the engine's CUDA probe; completion is observed via
    /// [`AsrHandle::refresh_status`].
    pub async fn probe_cuda(&self) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(AsrCommand::ProbeCuda {
                reply: Reply::Async(reply),
            })
            .await
            .map_err(|e| format!("ASR actor disconnected: {e}"))?;
        rx.await
            .map_err(|e| format!("ASR actor dropped reply: {e}"))?
    }

    pub fn initialize_blocking(&self) -> Result<(), String> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.sender
            .try_send(AsrCommand::Initialize {
                reply: Reply::Sync(tx),
            })
            .map_err(|e| format!("ASR actor channel error: {e}"))?;
        rx.recv()
            .map_err(|e| format!("ASR actor dropped reply: {e}"))?
    }

    pub async fn load_model_with_precision(
        &self,
        model_dir: &str,
        backend: &str,
        precision: &str,
    ) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(AsrCommand::LoadModel {
                model_dir: model_dir.to_string(),
                backend: backend.to_string(),
                precision: precision.to_string(),
                reply: Reply::Async(reply),
            })
            .await
            .map_err(|e| format!("ASR actor disconnected: {e}"))?;
        rx.await
            .map_err(|e| format!("ASR actor dropped reply: {e}"))?
    }

    pub fn load_model_with_precision_blocking(
        &self,
        model_dir: &str,
        backend: &str,
        precision: &str,
    ) -> Result<(), String> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.sender
            .try_send(AsrCommand::LoadModel {
                model_dir: model_dir.to_string(),
                backend: backend.to_string(),
                precision: precision.to_string(),
                reply: Reply::Sync(tx),
            })
            .map_err(|e| format!("ASR actor channel error: {e}"))?;
        rx.recv()
            .map_err(|e| format!("ASR actor dropped reply: {e}"))?
    }

    pub async fn unload_model(&self) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(AsrCommand::UnloadModel {
                reply: Reply::Async(reply),
            })
            .await
            .map_err(|e| format!("ASR actor disconnected: {e}"))?;
        rx.await
            .map_err(|e| format!("ASR actor dropped reply: {e}"))?
    }

    pub fn unload_model_blocking(&self) -> Result<(), String> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.sender
            .try_send(AsrCommand::UnloadModel {
                reply: Reply::Sync(tx),
            })
            .map_err(|e| format!("ASR actor channel error: {e}"))?;
        rx.recv()
            .map_err(|e| format!("ASR actor dropped reply: {e}"))?
    }

    pub async fn start_stream(
        &self,
        session_id: u64,
        language: &str,
        vocabulary: &[String],
    ) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(AsrCommand::StartStream {
                session_id,
                language: language.to_string(),
                vocabulary: vocabulary.to_vec(),
                reply: Reply::Async(reply),
            })
            .await
            .map_err(|e| format!("ASR actor disconnected: {e}"))?;
        rx.await
            .map_err(|e| format!("ASR actor dropped reply: {e}"))?
    }

    pub fn start_stream_blocking(
        &self,
        session_id: u64,
        language: &str,
        vocabulary: &[String],
    ) -> Result<(), String> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.sender
            .try_send(AsrCommand::StartStream {
                session_id,
                language: language.to_string(),
                vocabulary: vocabulary.to_vec(),
                reply: Reply::Sync(tx),
            })
            .map_err(|e| format!("ASR actor channel error: {e}"))?;
        rx.recv()
            .map_err(|e| format!("ASR actor dropped reply: {e}"))?
    }

    pub async fn push_audio(
        &self,
        session_id: u64,
        sequence_id: u32,
        samples: Vec<f32>,
    ) -> Result<Option<String>, String> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(AsrCommand::PushAudio {
                session_id,
                sequence_id,
                samples,
                reply: Reply::Async(reply),
            })
            .await
            .map_err(|e| format!("ASR actor disconnected: {e}"))?;
        rx.await
            .map_err(|e| format!("ASR actor dropped reply: {e}"))?
    }

    pub fn push_audio_blocking(
        &self,
        session_id: u64,
        sequence_id: u32,
        samples: &[f32],
    ) -> Result<Option<String>, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.sender
            .try_send(AsrCommand::PushAudio {
                session_id,
                sequence_id,
                samples: samples.to_vec(),
                reply: Reply::Sync(tx),
            })
            .map_err(|e| format!("ASR actor channel error: {e}"))?;
        rx.recv()
            .map_err(|e| format!("ASR actor dropped reply: {e}"))?
    }

    pub async fn get_partial_transcript(&self, session_id: u64) -> Result<String, String> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(AsrCommand::GetPartialTranscript {
                session_id,
                reply: Reply::Async(reply),
            })
            .await
            .map_err(|e| format!("ASR actor disconnected: {e}"))?;
        rx.await
            .map_err(|e| format!("ASR actor dropped reply: {e}"))?
    }

    pub async fn stop_stream(&self, session_id: u64) -> Result<String, String> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(AsrCommand::StopStream {
                session_id,
                reply: Reply::Async(reply),
            })
            .await
            .map_err(|e| format!("ASR actor disconnected: {e}"))?;
        rx.await
            .map_err(|e| format!("ASR actor dropped reply: {e}"))?
    }

    pub fn stop_stream_blocking(&self, session_id: u64) -> Result<String, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.sender
            .try_send(AsrCommand::StopStream {
                session_id,
                reply: Reply::Sync(tx),
            })
            .map_err(|e| format!("ASR actor channel error: {e}"))?;
        rx.recv()
            .map_err(|e| format!("ASR actor dropped reply: {e}"))?
    }

    fn signal_cancel(&self, session_id: u64) {
        if let Some((active, signal)) = &*self.inference_cancel.read() {
            if *active == session_id {
                signal();
            }
        }
    }

    pub async fn cancel_stream(&self, session_id: u64) -> Result<(), String> {
        self.signal_cancel(session_id);
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(AsrCommand::CancelStream {
                session_id,
                reply: Reply::Async(reply),
            })
            .await
            .map_err(|e| format!("ASR actor disconnected: {e}"))?;
        rx.await
            .map_err(|e| format!("ASR actor dropped reply: {e}"))?
    }

    pub fn cancel_stream_blocking(&self, session_id: u64) -> Result<(), String> {
        self.signal_cancel(session_id);
        let (tx, rx) = std::sync::mpsc::channel();
        self.sender
            .try_send(AsrCommand::CancelStream {
                session_id,
                reply: Reply::Sync(tx),
            })
            .map_err(|e| format!("ASR actor channel error: {e}"))?;
        rx.recv()
            .map_err(|e| format!("ASR actor dropped reply: {e}"))?
    }

    pub async fn install_model_dir(&self, model_dir: &str, repo: &str) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(AsrCommand::InstallModelDir {
                model_dir: model_dir.to_string(),
                repo: repo.to_string(),
                backend: "auto".into(),
                precision: "auto".into(),
                reply: Reply::Async(reply),
            })
            .await
            .map_err(|e| format!("ASR actor disconnected: {e}"))?;
        rx.await
            .map_err(|e| format!("ASR actor dropped reply: {e}"))?
    }

    pub fn install_model_dir_blocking(&self, model_dir: &str, repo: &str) -> Result<(), String> {
        self.install_model_dir_with_options_blocking(model_dir, repo, "auto", "auto")
    }

    pub fn install_model_dir_with_options_blocking(
        &self,
        model_dir: &str,
        repo: &str,
        backend: &str,
        precision: &str,
    ) -> Result<(), String> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.sender
            .try_send(AsrCommand::InstallModelDir {
                model_dir: model_dir.to_string(),
                repo: repo.to_string(),
                backend: backend.to_string(),
                precision: precision.to_string(),
                reply: Reply::Sync(tx),
            })
            .map_err(|e| format!("ASR actor channel error: {e}"))?;
        rx.recv()
            .map_err(|e| format!("ASR actor dropped reply: {e}"))?
    }

    pub async fn set_resource_dir(&self, dir: PathBuf) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(AsrCommand::SetResourceDir {
                dir,
                reply: Reply::Async(reply),
            })
            .await
            .map_err(|e| format!("ASR actor disconnected: {e}"))?;
        rx.await
            .map_err(|e| format!("ASR actor dropped reply: {e}"))
    }

    pub fn set_resource_dir_blocking(&self, dir: PathBuf) -> Result<(), String> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.sender
            .try_send(AsrCommand::SetResourceDir {
                dir,
                reply: Reply::Sync(tx),
            })
            .map_err(|e| format!("ASR actor channel error: {e}"))?;
        rx.recv()
            .map_err(|e| format!("ASR actor dropped reply: {e}"))
    }

    pub async fn swap_engine(&self, engine: Box<dyn ASREngine>) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(AsrCommand::SwapEngine {
                engine,
                reply: Reply::Async(reply),
            })
            .await
            .map_err(|e| format!("ASR actor disconnected: {e}"))?;
        rx.await
            .map_err(|e| format!("ASR actor dropped reply: {e}"))
    }

    pub fn swap_engine_blocking(&self, engine: Box<dyn ASREngine>) -> Result<(), String> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.sender
            .try_send(AsrCommand::SwapEngine {
                engine,
                reply: Reply::Sync(tx),
            })
            .map_err(|e| format!("ASR actor channel error: {e}"))?;
        rx.recv()
            .map_err(|e| format!("ASR actor dropped reply: {e}"))
    }

    pub async fn refresh_status(&self) -> Result<EngineStatus, String> {
        let (reply, rx) = oneshot::channel();
        self.sender
            .send(AsrCommand::RefreshStatus {
                reply: Reply::Async(reply),
            })
            .await
            .map_err(|e| format!("ASR actor disconnected: {e}"))?;
        rx.await
            .map_err(|e| format!("ASR actor dropped reply: {e}"))
    }

    pub fn try_send_command(
        &self,
        cmd: AsrCommand,
    ) -> Result<(), mpsc::error::TrySendError<AsrCommand>> {
        self.sender.try_send(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::time::{Duration, Instant};

    struct AsyncReadyEngine {
        ready: Arc<AtomicBool>,
    }

    impl ASREngine for AsyncReadyEngine {
        fn initialize(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn load_model_with_precision(
            &mut self,
            _model_dir: &str,
            _backend: &str,
            _precision: &str,
        ) -> Result<(), String> {
            Ok(())
        }
        fn unload_model(&mut self) -> Result<(), String> {
            self.ready.store(false, Ordering::SeqCst);
            Ok(())
        }
        fn is_model_loaded(&self) -> bool {
            self.ready.load(Ordering::SeqCst)
        }
        fn start_stream(&mut self, _language: &str, _vocabulary: &[String]) -> Result<(), String> {
            Ok(())
        }
        fn push_audio(&mut self, _samples: &[f32]) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn get_partial_transcript(&mut self) -> Result<String, String> {
            Ok(String::new())
        }
        fn stop_stream(&mut self) -> Result<String, String> {
            Ok(String::new())
        }
        fn cancel_stream(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn get_detected_language(&self) -> String {
            "en".into()
        }
        fn get_backend_name(&self) -> String {
            "async-ready".into()
        }
    }

    #[tokio::test]
    async fn refresh_status_promotes_async_engine_readiness_into_cache() {
        let ready = Arc::new(AtomicBool::new(false));
        let handle = AsrHandle::spawn(
            Box::new(AsyncReadyEngine {
                ready: Arc::clone(&ready),
            }),
            8,
        );

        assert!(!handle.engine_status().loaded);
        ready.store(true, Ordering::SeqCst);
        assert!(
            !handle.engine_status().loaded,
            "a background engine transition is stale until it is polled"
        );

        let refreshed = handle.refresh_status().await.unwrap();
        assert!(refreshed.loaded);
        assert!(handle.engine_status().loaded);
    }

    struct SlowStopEngine {
        pushed_count: Arc<AtomicUsize>,
        stop_duration: Duration,
        cancelled: bool,
    }

    impl ASREngine for SlowStopEngine {
        fn initialize(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn load_model_with_precision(
            &mut self,
            _m: &str,
            _b: &str,
            _p: &str,
        ) -> Result<(), String> {
            Ok(())
        }
        fn unload_model(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn is_model_loaded(&self) -> bool {
            true
        }
        fn start_stream(&mut self, _l: &str, _v: &[String]) -> Result<(), String> {
            Ok(())
        }
        fn push_audio(&mut self, samples: &[f32]) -> Result<Option<String>, String> {
            if self.cancelled {
                return Err("Stream was cancelled".into());
            }
            self.pushed_count.fetch_add(samples.len(), Ordering::SeqCst);
            Ok(None)
        }
        fn get_partial_transcript(&mut self) -> Result<String, String> {
            Ok(String::new())
        }
        fn stop_stream(&mut self) -> Result<String, String> {
            std::thread::sleep(self.stop_duration);
            Ok("slow completed".into())
        }
        fn cancel_stream(&mut self) -> Result<(), String> {
            self.cancelled = true;
            Ok(())
        }
        fn get_detected_language(&self) -> String {
            "en".into()
        }
        fn get_backend_name(&self) -> String {
            "SlowEngine".into()
        }
    }

    #[tokio::test]
    async fn long_stop_stream_does_not_delay_status_query() {
        let pushed = Arc::new(AtomicUsize::new(0));
        let engine = Box::new(SlowStopEngine {
            pushed_count: Arc::clone(&pushed),
            cancelled: false,
            stop_duration: Duration::from_millis(300),
        });
        let handle = AsrHandle::spawn(engine, 16);

        handle.start_stream(1, "en", &[]).await.unwrap();

        let handle_clone = handle.clone();
        let stop_task = tokio::spawn(async move { handle_clone.stop_stream(1).await });

        // Let stop_stream begin executing inside the actor
        tokio::time::sleep(Duration::from_millis(30)).await;

        let query_start = Instant::now();
        let status = handle.engine_status();
        let is_loaded = handle.is_model_loaded();
        let query_elapsed = query_start.elapsed();

        assert!(is_loaded);
        assert_eq!(status.backend, "SlowEngine");
        assert!(
            query_elapsed < Duration::from_millis(50),
            "Status query took {:?}, expected < 50ms (non-blocking status cache)",
            query_elapsed
        );

        let res = stop_task.await.unwrap().unwrap();
        assert_eq!(res, "slow completed");
    }

    #[tokio::test]
    async fn superseded_commands_are_dropped() {
        let pushed = Arc::new(AtomicUsize::new(0));
        let engine = Box::new(SlowStopEngine {
            pushed_count: Arc::clone(&pushed),
            cancelled: false,
            stop_duration: Duration::from_millis(10),
        });
        let handle = AsrHandle::spawn(engine, 16);

        // Start session 1
        handle.start_stream(1, "en", &[]).await.unwrap();

        // Start session 2 (supersedes session 1)
        handle.start_stream(2, "en", &[]).await.unwrap();

        // Push audio for session 1 -> dropped
        let push_res = handle.push_audio(1, 0, vec![0.5; 1000]).await.unwrap();
        assert_eq!(push_res, None);
        assert_eq!(
            pushed.load(Ordering::SeqCst),
            0,
            "superseded audio must not reach the engine"
        );

        // Stop stream for session 1 -> dropped
        let stop_res = handle.stop_stream(1).await.unwrap();
        assert_eq!(
            stop_res, "",
            "superseded stop_stream must return empty text"
        );

        // Session 2 commands work normally
        handle.push_audio(2, 0, vec![0.5; 1000]).await.unwrap();
        assert_eq!(pushed.load(Ordering::SeqCst), 1000);

        let stop2_res = handle.stop_stream(2).await.unwrap();
        assert_eq!(stop2_res, "slow completed");
    }

    #[tokio::test]
    async fn stale_cancel_does_not_cancel_current_engine_stream() {
        let handle = AsrHandle::spawn(
            Box::new(SlowStopEngine {
                pushed_count: Arc::new(AtomicUsize::new(0)),
                stop_duration: Duration::ZERO,
                cancelled: false,
            }),
            8,
        );
        handle.start_stream(1, "en", &[]).await.unwrap();
        handle.start_stream(2, "en", &[]).await.unwrap();
        handle.cancel_stream(1).await.unwrap();
        handle.push_audio(2, 0, vec![0.5; 16000]).await.unwrap();
        assert!(!handle.stop_stream(2).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn bounded_queue_applies_backpressure() {
        let pushed = Arc::new(AtomicUsize::new(0));
        let engine = Box::new(SlowStopEngine {
            pushed_count: Arc::clone(&pushed),
            cancelled: false,
            stop_duration: Duration::from_millis(500),
        });
        // Capacity 2
        let handle = AsrHandle::spawn(engine, 2);

        handle.start_stream(1, "en", &[]).await.unwrap();

        // Start a slow stop_stream that occupies the actor for 500ms
        let h_clone = handle.clone();
        tokio::spawn(async move { h_clone.stop_stream(1).await });

        tokio::time::sleep(Duration::from_millis(20)).await;

        // Channel capacity is 2. Let's fill the channel.
        let s1 = handle.try_send_command(AsrCommand::Initialize { reply: Reply::None });
        let s2 = handle.try_send_command(AsrCommand::Initialize { reply: Reply::None });
        let s3 = handle.try_send_command(AsrCommand::Initialize { reply: Reply::None });

        assert!(s1.is_ok(), "first queued item should succeed");
        assert!(s2.is_ok(), "second queued item should succeed");
        assert!(
            s3.is_err(),
            "third queued item on capacity-2 channel must return TrySendError::Full"
        );
    }

    #[tokio::test]
    async fn cancel_aborts_in_flight_work_and_discards_transcript() {
        let pushed = Arc::new(AtomicUsize::new(0));
        let engine = Box::new(SlowStopEngine {
            pushed_count: Arc::clone(&pushed),
            cancelled: false,
            stop_duration: Duration::from_millis(200),
        });
        let handle = AsrHandle::spawn(engine, 16);

        handle.start_stream(1, "en", &[]).await.unwrap();

        // Spawn stop_stream in background
        let h_clone = handle.clone();
        let stop_fut = tokio::spawn(async move { h_clone.stop_stream(1).await });

        // Let stop_stream start
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Cancel stream for session 1
        handle.cancel_stream(1).await.unwrap();

        // Subsequent pushes and stops for session 1 are dropped immediately
        let push_res = handle.push_audio(1, 0, vec![0.1; 100]).await.unwrap();
        assert_eq!(push_res, None);

        let stop_res = handle.stop_stream(1).await.unwrap();
        assert_eq!(stop_res, "");

        let _ = stop_fut.await;
    }
}
