use reflow_lib::asr::actor::AsrHandle;
use reflow_lib::asr::MockASREngine;
use std::time::Duration;

#[tokio::test]
async fn test_asr_actor_graceful_lifecycle_and_shutdown() {
    let mock = Box::new(MockASREngine::new());
    let handle = AsrHandle::spawn(mock, 32);

    // 1. Initialize
    let init_res = handle.initialize().await;
    assert!(init_res.is_ok());

    // 2. Load model
    let load_res = handle
        .load_model_with_precision("test-model", "cpu", "fp32")
        .await;
    assert!(load_res.is_ok());

    let status = handle.engine_status();
    assert!(status.loaded);

    // 3. Start and stop stream
    let session_id = handle.next_session_id();
    let start_res = handle.start_stream(session_id, "en", &[]).await;
    assert!(start_res.is_ok());

    let stop_res = handle.stop_stream(session_id).await;
    assert!(stop_res.is_ok());

    // 4. Graceful unload and teardown
    let unload_res = handle.unload_model().await;
    assert!(unload_res.is_ok());

    let final_status = handle.engine_status();
    assert!(!final_status.loaded);

    // Ensure actor thread shuts down cleanly when handle is dropped
    drop(handle);
    tokio::time::sleep(Duration::from_millis(50)).await;
}
