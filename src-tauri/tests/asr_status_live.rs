//! Live verification that ASR model status converges through a real sidecar
//! load.
//!
//! This is the regression test for the bug that made Reflow look completely
//! broken. The chain was:
//!
//! 1. the Python sidecar acks `load_model` and then blocks its main stdin loop
//!    inside `_warm_imports()` for 9-52s (45.4s on the machine this was
//!    diagnosed on). `ping` and `status` are served from that same thread, so
//!    the sidecar answers nothing at all for the duration;
//! 2. Rust gave `status` a 20s budget and recorded three consecutive probe
//!    timeouts as `EngineStatus::error`;
//! 3. `status` also reported `loaded=false, is_loading=false` for the window
//!    between the ack and the loader thread starting, i.e. "idle and not
//!    loaded" — indistinguishable from a failed load;
//! 4. `session::start_microphone_at` refuses to record unless the engine
//!    reports `loaded`, so the hotkey went dead, no transcript was produced,
//!    and nothing was ever pasted.
//!
//! The assertions below pin all three observable halves: status must never
//! claim idle-and-unloaded while a load is pending, must never surface an
//! error for a load that is merely slow, and must end up `loaded`.
//!
//! Skipped when the weights are not installed, so CI stays green without the
//! multi-gigabyte checkpoint.

use std::time::{Duration, Instant};

use reflow_lib::asr::AsrHandle;
use reflow_lib::model::manager::weights_present;
use reflow_lib::model::ModelManager;
use reflow_lib::platform::PlatformSys;

/// The 1.7B checkpoint is what the diagnosed configuration uses.
const MODEL_ID: &str = "1.7b";
const PRECISION: &str = "int8";

/// Generous: a cold filesystem has been measured at 45s of warm import before
/// the load even starts, plus ~15s to load and warm the model.
const CONVERGE_DEADLINE: Duration = Duration::from_secs(240);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn model_status_converges_to_loaded_without_a_false_failure() {
    let manager = ModelManager::new(PlatformSys::get_models_dir());
    let model_dir = manager.get_model_dir(MODEL_ID);
    if !weights_present(&model_dir) {
        eprintln!("skipping: no Qwen3-ASR weights at {}", model_dir.display());
        return;
    }

    let handle = AsrHandle::new_sidecar();
    handle
        .initialize()
        .await
        .expect("the sidecar should start and answer ping");

    handle
        .load_model_with_precision(&model_dir.to_string_lossy(), "auto", PRECISION)
        .await
        .expect("load_model should be acked");

    let started = Instant::now();
    let mut polls = 0u32;
    let mut saw_loading = false;
    let mut loaded = false;

    while started.elapsed() < CONVERGE_DEADLINE {
        // Exactly what `spawn_model_status_watch` does: poll, tolerate a probe
        // that could not be answered, never give up.
        let status = handle
            .refresh_status()
            .await
            .unwrap_or_else(|_| handle.engine_status());
        polls += 1;

        assert!(
            status.error.is_none(),
            "poll {polls} at {:?} surfaced an error for a load that is merely slow: {:?}\n\
             This is the bug that made the recording gate report \
             'Model failed to load' and silently disabled the hotkey.",
            started.elapsed(),
            status.error
        );

        if status.loaded {
            loaded = true;
            assert!(
                !status.is_loading,
                "a loaded model must not still report is_loading"
            );
            assert!(
                !status.device.is_empty() && status.device != "none",
                "a loaded model must name its device, got {:?}",
                status.device
            );
            break;
        }

        // Not loaded yet, so the load must still be reported as in progress.
        // The window where this was false is precisely what made a healthy
        // sidecar look like a failed one.
        assert!(
            status.is_loading,
            "poll {polls} at {:?} reported neither loaded nor loading \
             (backend={:?}). A pending load must always read as in progress.",
            started.elapsed(),
            status.backend
        );
        saw_loading = true;

        tokio::time::sleep(Duration::from_millis(700)).await;
    }

    assert!(
        saw_loading,
        "expected to observe the loading state at least once"
    );
    assert!(
        loaded,
        "model never reported loaded within {CONVERGE_DEADLINE:?} ({polls} polls)"
    );

    let final_status = handle.engine_status();
    eprintln!(
        "converged after {:?} and {polls} polls: device={} backend={} precision={} load={}s",
        started.elapsed(),
        final_status.device,
        final_status.backend,
        final_status.precision,
        final_status.load_seconds
    );

    // The exact condition `session::start_microphone_at` gates recording on.
    assert!(
        final_status.loaded && !final_status.is_loading && final_status.error.is_none(),
        "the recording gate must now admit dictation, got {final_status:?}"
    );

    let _ = handle.unload_model().await;
}
