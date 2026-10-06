use super::promote_with_asr_restore;
use crate::asr::actor::ModelLoadRequest;
use crate::asr::{ASREngine, AsrHandle};
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq)]
enum Call {
    Load(ModelLoadRequest),
    Unload,
}

#[derive(Default)]
struct State {
    loaded: bool,
    fail_load: bool,
    calls: Vec<Call>,
}

struct TrackingEngine(Arc<Mutex<State>>);

impl ASREngine for TrackingEngine {
    fn initialize(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn load_model_with_precision(
        &mut self,
        model_dir: &str,
        backend: &str,
        precision: &str,
    ) -> Result<(), String> {
        // This is also the real native server's launch gate: restoration must
        // run after maintenance releases both its pending flag and write lock.
        let _launch = super::super::runtime_inventory::launch_guard()?;
        let mut state = self.0.lock();
        state.calls.push(Call::Load(ModelLoadRequest {
            model_dir: model_dir.into(),
            backend: backend.into(),
            precision: precision.into(),
        }));
        if state.fail_load {
            return Err("fixture restore failed".into());
        }
        state.loaded = true;
        Ok(())
    }

    fn unload_model(&mut self) -> Result<(), String> {
        let mut state = self.0.lock();
        state.calls.push(Call::Unload);
        state.loaded = false;
        Ok(())
    }

    fn install_model_dir(&mut self, _model_dir: &str, _repo: &str) -> Result<(), String> {
        self.0.lock().loaded = false;
        Ok(())
    }

    fn is_model_loaded(&self) -> bool {
        self.0.lock().loaded
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
        "fixture".into()
    }
}

fn fixture() -> (AsrHandle, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State::default()));
    let asr = AsrHandle::spawn(Box::new(TrackingEngine(Arc::clone(&state))), 8);
    (asr, state)
}

#[test]
fn runtime_activation_restores_native_asr_and_reports_both_failures() {
    let request = ModelLoadRequest {
        model_dir: "fixture/qwen3-asr-1.7b-native".into(),
        backend: "cpu".into(),
        precision: "int8".into(),
    };
    for promotion_fails in [false, true] {
        for restoration_fails in [false, true] {
            let (asr, state) = fixture();
            asr.load_model_with_precision_blocking(
                &request.model_dir,
                &request.backend,
                &request.precision,
            )
            .unwrap();
            state.lock().calls.clear();
            let mut flow_stopped = false;
            let result = promote_with_asr_restore(
                &asr,
                true,
                || flow_stopped = true,
                || {
                    let mut current = state.lock();
                    assert!(!current.loaded, "promotion must stop the incumbent");
                    assert_eq!(current.calls, [Call::Unload]);
                    current.fail_load = restoration_fails;
                    if promotion_fails {
                        Err("fixture promotion failed".into())
                    } else {
                        Ok(())
                    }
                },
            );
            assert!(flow_stopped);
            let current = state.lock();
            assert_eq!(current.calls, [Call::Unload, Call::Load(request.clone())]);
            assert_eq!(current.loaded, !restoration_fails);
            assert_eq!(asr.is_model_loaded(), !restoration_fails);
            match (promotion_fails, restoration_fails) {
                (false, false) => result.unwrap(),
                (true, false) => assert_eq!(result.unwrap_err(), "fixture promotion failed"),
                (_, true) => {
                    let error = result.unwrap_err();
                    assert!(error.contains("fixture restore failed"));
                    assert_eq!(error.contains("fixture promotion failed"), promotion_fails);
                }
            }
        }
    }

    // An asynchronously installed native model gets its restore request from
    // installation, even though the actor never received an explicit load.
    let (asr, state) = fixture();
    asr.install_model_dir_blocking("fixture/native-installed", "fixture/repo")
        .unwrap();
    state.lock().loaded = true;
    promote_with_asr_restore(&asr, true, || {}, || Ok(())).unwrap();
    assert_eq!(
        state.lock().calls,
        [
            Call::Unload,
            Call::Load(ModelLoadRequest {
                model_dir: "fixture/native-installed".into(),
                backend: "auto".into(),
                precision: "auto".into(),
            })
        ]
    );

    // Native models already unloaded stay unloaded. Python is untouched.
    for native in [false, true] {
        let (asr, state) = fixture();
        if !native {
            state.lock().loaded = true;
        }
        promote_with_asr_restore(&asr, native, || {}, || Ok(())).unwrap();
        assert_eq!(state.lock().loaded, !native);
        assert_eq!(
            state.lock().calls,
            if native { vec![Call::Unload] } else { vec![] }
        );
    }

    // Refuse before stopping a loaded engine whose identity cannot be restored.
    let (asr, state) = fixture();
    state.lock().loaded = true;
    let result = promote_with_asr_restore(
        &asr,
        true,
        || panic!("must preserve the incumbent"),
        || panic!("must not activate without a restore request"),
    );
    assert!(result.unwrap_err().contains("no restorable load request"));
    assert!(state.lock().loaded);
    assert!(state.lock().calls.is_empty());
}
