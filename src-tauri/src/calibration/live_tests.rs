//! Opt-in installed-weight evidence. Never records, inserts, downloads or applies settings.
use super::*;

struct SmokeScope;
impl SmokeScope {
    fn enter() -> Self {
        SMOKE.with(|enabled| enabled.set(true));
        Self
    }
}
impl Drop for SmokeScope {
    fn drop(&mut self) {
        SMOKE.with(|enabled| enabled.set(false));
        controller().running.store(false, Ordering::Release);
        *controller().active.lock() = None;
        *controller().pending.lock() = None;
    }
}

#[test]
#[ignore = "Requires installed CUDA ASR, llama-server, GGUF and labelled local WAV; run alone"]
fn installed_models_co_resident_quality_gate() {
    let _scope = SmokeScope::enter();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();
    let corpus_path = root.join(".codex-runtime-evidence/benchmark-corpus.json");
    let corpus: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&corpus_path).expect("labelled corpus fixture"))
            .expect("corpus JSON");
    let sample = corpus
        .as_array()
        .expect("corpus array")
        .iter()
        .find(|sample| sample["language"] == "en")
        .expect("English fixture");
    let reference = sample["reference"].as_str().expect("nonempty reference");
    assert!(policy::error_rates(reference, reference).is_some());
    let audio = corpus_path
        .parent()
        .expect("fixture directory")
        .join(sample["audio"].as_str().expect("audio path"));
    let mut reader = hound::WavReader::open(audio).expect("local WAV fixture");
    let spec = reader.spec();
    assert_eq!(
        (spec.sample_rate, spec.channels, spec.bits_per_sample),
        (16000, 1, 16)
    );
    assert_eq!(spec.sample_format, hound::SampleFormat::Int);
    let samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|value| f32::from(value.expect("PCM sample")) / 32768.0)
        .collect();
    assert!(!samples.is_empty());

    let temporary =
        std::env::temp_dir().join(format!("reflow-calibration-live-{}", uuid::Uuid::new_v4()));
    let mut ctx = AppContext::bootstrap_test(temporary.clone());
    ctx.model_manager = std::sync::Arc::new(crate::model::manager::ModelManager::new(
        crate::platform::PlatformSys::get_models_dir(),
    ));
    assert!(
        ctx.model_manager.is_installed("0.6b"),
        "No speech downloads allowed"
    );
    assert!(
        crate::rewrite::flow_gguf_path("qwen3.5-0.8b").is_file(),
        "No writing downloads allowed"
    );
    assert!(
        crate::rewrite::llama_server_bin().is_file(),
        "No runtime downloads allowed"
    );
    let mut settings = ctx.settings_store.get();
    settings.language = "en".into();
    settings.auto_detect_language = false;
    settings.intelligence_tier = "smart_flow".into();
    settings.cleanup_level = "light".into();
    settings.style = "faithful".into();
    settings.auto_style_from_app = false;
    ctx.settings_store
        .update(settings)
        .expect("temporary settings");
    assert!(ctx.settings_store.get().resolve_intent().run_llm);

    // Obtain actual torch CUDA availability; a GPU name alone is not evidence.
    let probe = AsrHandle::new_runtime("python");
    probe
        .set_resource_dir_blocking(root.clone())
        .expect("local sidecar resource");
    probe
        .initialize_blocking()
        .expect("initialize local Python runtime");
    tauri::async_runtime::block_on(probe.probe_cuda()).expect("start real CUDA probe");
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let status = tauri::async_runtime::block_on(probe.refresh_status());
        if let Ok(status) = status {
            if !status.cuda_probe_pending {
                assert!(
                    status.cuda_available,
                    "Installed torch cannot use CUDA: {:?}",
                    status.error
                );
                break;
            }
        }
        assert!(Instant::now() < deadline, "Real CUDA probe did not settle");
        std::thread::sleep(Duration::from_millis(250));
    }
    probe
        .unload_model_blocking()
        .expect("release probe runtime");
    drop(probe);
    assert!(crate::capability::capabilities_uncached().can_use_cuda());
    controller().cancel.store(false, Ordering::Release);
    *controller().status.write() = CalibrationStatus {
        running: true,
        phase: "measuring".into(),
        language: "en".into(),
        reference: reference.into(),
        ..Default::default()
    };
    let result = measure(&ctx, Some(root.clone()), samples, reference, "en");
    let mut report = controller().status.read().clone();
    report.running = false;
    report.phase = if result.is_ok() { "complete" } else { "failed" }.into();
    report.error = result.as_ref().err().cloned();
    std::fs::write(
        root.join("docs/implementation-co-resident-calibration.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "scope": "One labelled English sample; 1 warmup + 3 measured joint ASR and LLM runs. No microphone, insertion, downloads or applied cache. Not representative multilingual evidence.",
            "hardware": crate::capability::capabilities_uncached(),
            "report": report,
        })).expect("evidence JSON"),
    ).expect("save local evidence");
    result.expect("live calibration path failed");
    assert!(
        !report.candidates.is_empty(),
        "Smoke filter admitted no candidates"
    );
    assert!(report
        .candidates
        .iter()
        .all(|row| row.choice.runtime == "python"
            && row.choice.model == "0.6b"
            && row.choice.device == "cuda"
            && row.choice.precision == "bf16"
            && row.choice.refinement_model == "qwen3.5-0.8b"));
    let winner = report
        .winner_id
        .as_ref()
        .expect("No co-resident candidate met quality and stability gates");
    assert!(report
        .candidates
        .iter()
        .any(|row| &row.id == winner && row.eligible));
    drop(ctx);
    std::fs::remove_dir_all(temporary).expect("remove disposable test context");
}
