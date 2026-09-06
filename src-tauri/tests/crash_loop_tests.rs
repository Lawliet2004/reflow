use reflow_lib::asr::sidecar::Qwen3AsrSidecar;
use reflow_lib::asr::ASREngine;

#[test]
fn test_sidecar_crash_loop_circuit_breaker_trips_after_3_crashes() {
    let mut sidecar = Qwen3AsrSidecar::new();

    // Initially closed
    assert!(!sidecar.is_circuit_breaker_open());

    // 1st crash
    sidecar.record_crash_and_check_breaker("Simulated crash 1");
    assert!(!sidecar.is_circuit_breaker_open());

    // 2nd crash
    sidecar.record_crash_and_check_breaker("Simulated crash 2");
    assert!(!sidecar.is_circuit_breaker_open());

    // 3rd crash within window -> trips breaker
    sidecar.record_crash_and_check_breaker("Simulated crash 3");
    assert!(sidecar.is_circuit_breaker_open());

    // Status is updated to circuit_breaker_tripped
    let status = sidecar.engine_status();
    assert_eq!(status.backend, "circuit_breaker_tripped");
    assert!(!status.loaded);
    assert!(status.error.unwrap().contains("circuit breaker tripped"));

    // initialize() rejects execution while breaker is open
    let init_res = sidecar.initialize();
    assert!(init_res.is_err());
    assert!(init_res.unwrap_err().contains("circuit breaker is open"));
}
