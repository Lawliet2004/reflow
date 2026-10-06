//! Exercise the protocol and load clock without models or long sleeps.
use super::*;

struct Fixture {
    root: PathBuf,
    engine: Qwen3AsrSidecar,
}

impl Fixture {
    fn new() -> Option<Self> {
        Qwen3AsrSidecar::find_python()?;
        let root = std::env::temp_dir().join(format!(
            "reflow_sidecar_load_deadline_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("mode"), "idle").unwrap();
        std::fs::write(
            root.join("qwen3_asr_runtime.py"),
            r#"
import json, pathlib, sys
mode_path = pathlib.Path(__file__).with_name('mode')
for line in sys.stdin:
    request = json.loads(line)
    cmd = request['cmd']
    mode = mode_path.read_text()
    response = {'id': request['id'], 'status': 'ok', 'pong': True}
    if cmd == 'load_model':
        response['status'] = 'loading'
    elif cmd == 'probe_cuda':
        response['status'] = 'probing'
    elif cmd == 'status':
        response.update(loaded=mode == 'ready', is_loading=mode in ('loading', 'both'),
                        is_downloading=mode in ('downloading', 'both'),
                        backend='loading' if mode in ('loading', 'both') else 'fixture',
                        cuda_probe_pending=False)
        if mode == 'failed':
            response.update(error='fixture load failed', failure_kind='weights_missing')
    print(json.dumps(response), flush=True)
"#,
        )
        .unwrap();
        let mut engine = Qwen3AsrSidecar::new();
        engine.set_resource_dir(root.clone());
        engine.initialize().unwrap();
        Some(Self { root, engine })
    }

    fn mode(&self, mode: &str) {
        std::fs::write(self.root.join("mode"), mode).unwrap();
    }

    fn poll_at(&mut self, now: Instant) -> Result<(), String> {
        let response = self.engine.send_command(json!({"cmd": "status"}))?;
        self.engine.apply_status_response(response, now)
    }

    fn load(&mut self) -> Instant {
        self.mode("loading");
        self.engine
            .load_model_with_precision("fixture", "cpu", "fp32")
            .unwrap();
        self.engine.model_load_started_at.unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.engine.unload_model().unwrap();
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn model_load_deadline_retires_responsive_and_mute_loaders_and_allows_reload() {
    let Some(mut fixture) = Fixture::new() else {
        return;
    };
    let started = fixture.load();
    let writer = fixture.engine.writer().unwrap();
    fixture
        .poll_at(started + TIMEOUT_MODEL_LOAD - Duration::from_nanos(1))
        .unwrap();
    assert!(fixture.engine.child.is_some());
    // Another accepted request must not reset a loader's existing budget.
    fixture.engine.load_model("fixture", "cpu").unwrap();
    assert_eq!(fixture.engine.model_load_started_at, Some(started));
    let error = fixture.poll_at(started + TIMEOUT_MODEL_LOAD).unwrap_err();
    assert!(is_engine_timeout(&error));
    assert!(error.contains("Reload the model"));
    assert!(fixture.engine.child.is_none());
    assert!(fixture.engine.stdin.is_none());
    assert!(fixture.engine.responses.is_none());
    assert!(writer.closed.load(Ordering::Acquire));
    assert!(fixture.engine.model_load_started_at.is_none());
    let status = fixture.engine.engine_status();
    assert!(!status.loaded && !status.is_loading && !status.is_downloading);
    assert_eq!(status.error.as_deref(), Some(error.as_str()));
    assert_eq!(status.failure_kind.as_deref(), Some("timeout"));
    assert!(fixture.engine.crash_timestamps.is_empty());

    fixture.mode("ready");
    fixture.engine.load_model("fixture", "cpu").unwrap();
    assert!(fixture.engine.engine_status().loaded);
    assert!(fixture.engine.status_cache.error.is_none());
    assert!(fixture.engine.model_load_started_at.is_none());
    assert!(!Arc::ptr_eq(&writer, &fixture.engine.writer().unwrap()));

    let started = fixture.load();
    let error = fixture.engine.on_command_timeout_at(
        "status",
        TIMEOUT_STATUS,
        started + TIMEOUT_MODEL_LOAD,
    );
    assert!(is_engine_timeout(&error));
    assert!(fixture.engine.child.is_none());
    assert_eq!(
        fixture.engine.status_cache.error.as_deref(),
        Some(error.as_str())
    );
}

#[test]
fn model_load_deadline_excludes_downloads_and_starts_after_they_finish() {
    let Some(mut fixture) = Fixture::new() else {
        return;
    };
    let started = fixture.load();
    let pid = fixture.engine.child.as_ref().unwrap().id();
    // Downloads take precedence, including when Python reports both flags.
    for mode in ["downloading", "both"] {
        fixture.mode(mode);
        fixture.poll_at(started + TIMEOUT_MODEL_LOAD * 4).unwrap();
        assert!(fixture.engine.status_cache.is_downloading);
        assert!(fixture.engine.model_load_started_at.is_none());
        assert_eq!(fixture.engine.child.as_ref().unwrap().id(), pid);
        fixture.engine.load_model("fixture", "cpu").unwrap();
        assert!(fixture.engine.model_load_started_at.is_none());
        fixture.engine.on_command_timeout_at(
            "status",
            TIMEOUT_STATUS,
            started + TIMEOUT_MODEL_LOAD * 8,
        );
        assert_eq!(fixture.engine.child.as_ref().unwrap().id(), pid);
    }

    let after_download = started + TIMEOUT_MODEL_LOAD * 10;
    fixture.mode("loading");
    fixture.poll_at(after_download).unwrap();
    assert_eq!(fixture.engine.model_load_started_at, Some(after_download));
    fixture
        .poll_at(after_download + TIMEOUT_MODEL_LOAD - Duration::from_nanos(1))
        .unwrap();
    assert_eq!(fixture.engine.child.as_ref().unwrap().id(), pid);
    assert!(fixture
        .poll_at(after_download + TIMEOUT_MODEL_LOAD)
        .is_err());
    assert!(fixture.engine.child.is_none());
}

#[test]
fn model_load_deadline_preserves_resolved_models_and_cuda_only_probes() {
    let Some(mut fixture) = Fixture::new() else {
        return;
    };
    for mode in ["ready", "failed"] {
        let started = fixture.load();
        let pid = fixture.engine.child.as_ref().unwrap().id();
        fixture.mode(mode);
        // Read the live resolution before considering an expired cached clock.
        fixture.poll_at(started + TIMEOUT_MODEL_LOAD * 2).unwrap();
        assert_eq!(fixture.engine.child.as_ref().unwrap().id(), pid);
        assert!(fixture.engine.model_load_started_at.is_none());
        assert!(!fixture.engine.load_in_flight);
        assert!(!fixture.engine.status_cache.is_loading);
        assert_eq!(fixture.engine.status_cache.loaded, mode == "ready");
        if mode == "failed" {
            assert_eq!(
                fixture.engine.status_cache.error.as_deref(),
                Some("fixture load failed")
            );
        }
    }
    fixture.engine.unload_model().unwrap();
    assert!(fixture.engine.model_load_started_at.is_none());
    fixture.mode("idle");
    fixture.engine.probe_cuda().unwrap();
    assert!(fixture.engine.load_in_flight);
    assert!(fixture.engine.model_load_started_at.is_none());
    let pid = fixture.engine.child.as_ref().unwrap().id();
    fixture.engine.on_command_timeout_at(
        "status",
        TIMEOUT_STATUS,
        Instant::now() + TIMEOUT_MODEL_LOAD * 2,
    );
    assert_eq!(fixture.engine.child.as_ref().unwrap().id(), pid);
    assert!(fixture.engine.status_cache.error.is_none());
}
