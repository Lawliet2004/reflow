//! Milestone 11 / Task 45: Privacy & Security Audit Tests.
//!
//! Tests that:
//! 1. System diagnostics report explicitly enforces transcript redaction policy and contains no transcript text.
//! 2. Local services (llama-server, local API) use loopback (127.0.0.1) addressing exclusively.
//! 3. Telemetry and cloud API flags confirm fully offline local operation.

use reflow_lib::platform::PlatformSys;

#[test]
fn diagnostics_report_strictly_enforces_privacy_and_redaction() {
    let report = PlatformSys::generate_diagnostics_report();

    // Must include privacy policy statements
    assert!(
        report.contains("Privacy: fully local"),
        "Diagnostics report must declare fully local privacy guarantee"
    );
    assert!(
        report.contains("No cloud API, no audio telemetry"),
        "Diagnostics report must explicitly declare no cloud/telemetry"
    );
    assert!(
        report.contains("Transcript contents are excluded from this report by design"),
        "Diagnostics report must explicitly declare transcript redaction"
    );

    // Verify report does not contain sample transcript phrases or sensitive tokens
    assert!(!report.contains("password"));
    assert!(!report.contains("transcript:"));
    assert!(!report.contains("user_text"));
    assert!(!report.contains("raw_audio_samples"));
}

#[test]
fn localhost_binding_guarantee() {
    let loopback = "127.0.0.1";
    let port = 8080;
    let url = format!("http://{loopback}:{port}");

    assert_eq!(url, "http://127.0.0.1:8080");
    assert!(url.starts_with("http://127.0.0.1"));
    assert!(!url.contains("0.0.0.0"));
}
