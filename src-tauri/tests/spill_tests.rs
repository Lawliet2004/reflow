//! Milestone 1 / Task 7: VRAM spill must be a first-class status, distinct from
//! OOM and from success.
//!
//! The detection predicate itself lives in the Python runtime (it needs
//! `torch.cuda.memory_reserved()`); these tests cover the Rust side's handling
//! of the verdict, which is what decides whether a configuration ships.

use reflow_lib::asr::{EngineStatus, LoadFailureKind};
use reflow_lib::session::{injection_message, InjectionMessageInput};

fn clean_loaded() -> EngineStatus {
    EngineStatus {
        loaded: true,
        device: "cuda".into(),
        backend: "Qwen3-ASR 0.6B · CUDA bf16".into(),
        spill_checked: true,
        spill_detected: false,
        precision: "bf16".into(),
        ..Default::default()
    }
}

#[test]
fn a_clean_cuda_load_is_healthy_on_device() {
    let status = clean_loaded();
    assert!(status.is_healthy_on_device());
    assert!(status.failure_kind().is_none());
}

/// The signature 4 GB failure: 1.7B BF16 loads successfully and runs ten times
/// slower. It must not be reported as a healthy configuration.
#[test]
fn a_spilled_load_is_not_healthy_even_though_it_loaded() {
    let status = EngineStatus {
        spill_detected: true,
        spill_reasons: vec![
            "torch reserved 3900 MB but device free VRAM fell by only 2900 MB".into(),
        ],
        ..clean_loaded()
    };
    assert!(status.loaded, "the load itself did succeed");
    assert!(
        !status.is_healthy_on_device(),
        "a spilled load must never count as healthy"
    );
    assert!(
        !status.spill_reasons.is_empty(),
        "the verdict must be explained"
    );
}

/// An unverified load is not the same as a verified-clean one. Treating it as
/// clean is how an unmeasured configuration gets cached as validated.
#[test]
fn an_unchecked_load_is_not_treated_as_clean() {
    let status = EngineStatus {
        spill_checked: false,
        spill_detected: false,
        ..clean_loaded()
    };
    assert!(
        !status.is_healthy_on_device(),
        "spill_checked=false must not read as verified-clean"
    );
}

#[test]
fn cpu_loads_report_no_spill_and_are_not_claimed_as_device_healthy() {
    // A CPU load cannot spill, but it also is not "healthy on device".
    let status = EngineStatus {
        device: "cpu".into(),
        backend: "Qwen3-ASR 0.6B · CPU cpu".into(),
        spill_checked: false,
        ..clean_loaded()
    };
    assert!(!status.spill_detected);
    assert!(!status.is_healthy_on_device());
}

#[test]
fn spill_is_distinguishable_from_oom_and_timeout() {
    assert_eq!(LoadFailureKind::parse("spill"), LoadFailureKind::Spill);
    assert_eq!(LoadFailureKind::parse("oom"), LoadFailureKind::Oom);
    assert_eq!(LoadFailureKind::parse("timeout"), LoadFailureKind::Timeout);
    assert_eq!(
        LoadFailureKind::parse("unsupported_precision"),
        LoadFailureKind::UnsupportedPrecision
    );
    assert_eq!(LoadFailureKind::parse("nonsense"), LoadFailureKind::Unknown);

    // Three genuinely different conditions must not collapse into one.
    assert_ne!(LoadFailureKind::Spill, LoadFailureKind::Oom);
    assert_ne!(LoadFailureKind::Spill, LoadFailureKind::Timeout);
}

/// Spill has to invalidate a benchmark result. If it were recorded as a
/// success, the benchmark-driven profile resolver would select it.
#[test]
fn spill_invalidates_a_benchmark_result() {
    assert!(LoadFailureKind::Spill.invalidates_benchmark());
    assert!(LoadFailureKind::Oom.invalidates_benchmark());
    assert!(LoadFailureKind::Timeout.invalidates_benchmark());
    assert!(LoadFailureKind::Crash.invalidates_benchmark());

    // A missing model or an unsupported precision is a configuration problem,
    // not an invalid measurement: there is nothing to invalidate.
    assert!(!LoadFailureKind::WeightsMissing.invalidates_benchmark());
    assert!(!LoadFailureKind::UnsupportedPrecision.invalidates_benchmark());
}

/// Retrying a spilled configuration cannot help — it will spill again.
#[test]
fn permanent_failures_are_not_retried() {
    assert!(LoadFailureKind::Spill.is_permanent());
    assert!(LoadFailureKind::UnsupportedPrecision.is_permanent());
    assert!(LoadFailureKind::WeightsMissing.is_permanent());

    // These can be transient: another process may have freed memory, or the
    // runtime may simply have been busy.
    assert!(!LoadFailureKind::Oom.is_permanent());
    assert!(!LoadFailureKind::Timeout.is_permanent());
    assert!(!LoadFailureKind::Crash.is_permanent());
}

#[test]
fn every_failure_kind_has_actionable_remediation() {
    for kind in [
        LoadFailureKind::Spill,
        LoadFailureKind::Oom,
        LoadFailureKind::UnsupportedPrecision,
        LoadFailureKind::WeightsMissing,
        LoadFailureKind::Timeout,
        LoadFailureKind::Crash,
        LoadFailureKind::Unknown,
    ] {
        let text = kind.remediation();
        assert!(!text.is_empty(), "{kind:?} has no remediation");
        assert!(
            text.len() > 20,
            "{kind:?} remediation is too terse to act on: {text}"
        );
    }
    // Spill's remediation must point at the actual fix, not at memory pressure.
    assert!(LoadFailureKind::Spill
        .remediation()
        .contains("does not fit"));
}

#[test]
fn engine_status_round_trips_through_json() {
    let status = EngineStatus {
        spill_detected: true,
        spill_reasons: vec!["reason one".into(), "reason two".into()],
        spill_checked: true,
        failure_kind: Some("spill".into()),
        warmup_rtf: Some(3.4),
        load_seconds: 18.5,
        ..clean_loaded()
    };
    let json = serde_json::to_string(&status).expect("serialize");
    let back: EngineStatus = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.spill_detected, status.spill_detected);
    assert_eq!(back.spill_reasons, status.spill_reasons);
    assert_eq!(back.failure_kind(), Some(LoadFailureKind::Spill));
    assert_eq!(back.warmup_rtf, Some(3.4));

    // A payload from an older sidecar that lacks the new fields must still
    // deserialize, defaulting to "not checked".
    let legacy = r#"{"loaded":true,"device":"cuda","backend":"x","vram_mb":0,
        "is_downloading":false,"download_progress_pct":0,"error":null}"#;
    let old: EngineStatus = serde_json::from_str(legacy).expect("legacy deserialize");
    assert!(!old.spill_checked);
    assert!(!old.spill_detected);
    assert!(!old.is_healthy_on_device());
}

/// Task 16: a truncated dictation must say so in the line the user reads.
#[test]
fn truncation_warning_reaches_the_user_facing_message() {
    let with_warning = injection_message(&InjectionMessageInput {
        silent_mic: false,
        capture_device: "Headset Mic",
        transcript_empty: false,
        fallback_copy: false,
        paste_chord: "",
        rewriter_error: None,
        wants_flow: true,
        rewriter_used: true,
        asr_warning: Some("Only the first 120s of this 180s dictation was transcribed."),
    });
    assert!(with_warning.starts_with("Inserted"));
    assert!(
        with_warning.contains("Only the first 120s"),
        "the truncation notice must be visible: {with_warning}"
    );

    // No warning: the message is unchanged.
    let plain = injection_message(&InjectionMessageInput {
        asr_warning: None,
        ..InjectionMessageInput {
            silent_mic: false,
            capture_device: "Headset Mic",
            transcript_empty: false,
            fallback_copy: false,
            paste_chord: "",
            rewriter_error: None,
            wants_flow: true,
            rewriter_used: true,
            asr_warning: None,
        }
    });
    assert_eq!(plain, "Inserted");
}

#[test]
fn no_speech_does_not_get_a_truncation_warning_appended() {
    let message = injection_message(&InjectionMessageInput {
        silent_mic: false,
        capture_device: "",
        transcript_empty: true,
        fallback_copy: false,
        paste_chord: "",
        rewriter_error: None,
        wants_flow: false,
        rewriter_used: false,
        asr_warning: Some("Only the first 120s was transcribed."),
    });
    assert_eq!(
        message, "No speech",
        "a truncation notice on an empty transcript is noise"
    );
}

/// The clipboard fallback used to be mentioned in only one of the two
/// near-duplicate message cascades.
#[test]
fn clipboard_fallback_is_reported_consistently() {
    let message = injection_message(&InjectionMessageInput {
        silent_mic: false,
        capture_device: "Mic",
        transcript_empty: false,
        fallback_copy: true,
        paste_chord: "Ctrl+V",
        rewriter_error: None,
        wants_flow: false,
        rewriter_used: false,
        asr_warning: None,
    });
    assert_eq!(message, "Copied — press Ctrl+V");
}
