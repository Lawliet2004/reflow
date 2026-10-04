use super::*;
use crate::asr::engine::{ASREngine, InferenceCancellation};
use crate::state::AppStateEnum;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Default)]
struct DecodeControl {
    entered: tokio::sync::Notify,
    cancelled: AtomicBool,
    complete: AtomicBool,
}

struct ControlledAsr(Arc<DecodeControl>);
impl ASREngine for ControlledAsr {
    fn initialize(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn load_model_with_precision(&mut self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn unload_model(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn is_model_loaded(&self) -> bool {
        true
    }
    fn start_stream(&mut self, _: &str, _: &[String]) -> Result<(), String> {
        self.0.cancelled.store(false, Ordering::Release);
        Ok(())
    }
    fn push_audio(&mut self, _: &[f32]) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn get_partial_transcript(&mut self) -> Result<String, String> {
        Ok(String::new())
    }
    fn stop_stream(&mut self) -> Result<String, String> {
        self.0.entered.notify_one();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.0.cancelled.load(Ordering::Acquire)
            && !self.0.complete.load(Ordering::Acquire)
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok("Discard this cancelled transcript".into())
    }
    fn cancel_stream(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn cancellation_signal(&self) -> Option<InferenceCancellation> {
        let control = Arc::clone(&self.0);
        Some(Arc::new(move || {
            control.cancelled.store(true, Ordering::Release)
        }))
    }
    fn get_detected_language(&self) -> String {
        "en".into()
    }
    fn get_backend_name(&self) -> String {
        "controlled-asr".into()
    }
}

async fn connect_socket(address: std::net::SocketAddr, token: &str) -> tokio::net::TcpStream {
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    let handshake = format!(
        "GET /v1/stream HTTP/1.1\r\nHost: {address}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nAuthorization: Bearer {token}\r\n\r\n"
    );
    socket.write_all(handshake.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !response.ends_with(b"\r\n\r\n") {
            response.push(socket.read_u8().await.unwrap());
        }
    })
    .await
    .unwrap();
    assert!(response.starts_with(b"HTTP/1.1 101"));
    socket
}

async fn send_frame(socket: &mut tokio::net::TcpStream, opcode: u8, payload: &[u8]) {
    let mask = [1, 2, 3, 4];
    let mut frame = vec![0x80 | opcode];
    if payload.len() < 126 {
        frame.push(0x80 | payload.len() as u8);
    } else {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    }
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(i, byte)| byte ^ mask[i % 4]),
    );
    socket.write_all(&frame).await.unwrap();
}

async fn receive_text(socket: &mut tokio::net::TcpStream) -> Value {
    assert_eq!(socket.read_u8().await.unwrap() & 0x0f, 1);
    let length = socket.read_u8().await.unwrap();
    let length = match length & 0x7f {
        126 => socket.read_u16().await.unwrap() as usize,
        127 => socket.read_u64().await.unwrap() as usize,
        value => value as usize,
    };
    assert!(length < 4096);
    let mut bytes = vec![0; length];
    socket.read_exact(&mut bytes).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[derive(Clone, Copy, Debug)]
enum Interrupt {
    Cancel,
    Disconnect,
    Revoke,
    Disable,
    Shutdown,
}

async fn wait_for_phone_audio(ctx: &AppContext) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while ctx.recording_pcm.lock().total_samples() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

async fn processing_interrupt(action: Interrupt, desktop_stop: bool) {
    let ctx = AppContext::bootstrap_test(std::env::temp_dir().join(format!(
        "reflow_processing_interrupt_{}",
        uuid::Uuid::new_v4()
    )));
    ctx.settings_store
        .merge_update(json!({"api_enabled":true}))
        .unwrap();
    let control = Arc::new(DecodeControl::default());
    ctx.asr_handle
        .swap_engine(Box::new(ControlledAsr(control.clone())))
        .await
        .unwrap();
    let offer = ctx.pairing.rotate_code();
    let (token, device) = ctx.pairing.pair(&offer.code, "interrupt phone").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown, generation) = tokio::sync::watch::channel(false);
    let app = router(ctx.clone()).layer(Extension(generation));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut socket = connect_socket(address, &token).await;
    let mut events = ctx.bus.subscribe();
    send_frame(&mut socket, 1, br#"{"type":"start","inject":false}"#).await;
    let ready = tokio::time::timeout(Duration::from_secs(3), receive_text(&mut socket))
        .await
        .unwrap();
    assert_eq!(ready["type"], "ready");
    let old = *ctx.current_session_id.read();
    if desktop_stop {
        assert_eq!(*ctx.session_phone_token.read(), Some(token.clone()));
    }
    let pcm: Vec<u8> = (0..1600).flat_map(|_| 16000i16.to_le_bytes()).collect();
    send_frame(&mut socket, 2, &pcm).await;
    let desktop_processing = if desktop_stop {
        wait_for_phone_audio(&ctx).await;
        let ctx = ctx.clone();
        Some(tokio::spawn(async move { session::stop(&ctx, true).await }))
    } else {
        send_frame(&mut socket, 1, br#"{"type":"stop"}"#).await;
        None
    };
    tokio::time::timeout(Duration::from_secs(3), control.entered.notified())
        .await
        .unwrap();
    match action {
        Interrupt::Cancel => send_frame(&mut socket, 1, br#"{"type":"cancel"}"#).await,
        Interrupt::Disconnect => socket.shutdown().await.unwrap(),
        Interrupt::Revoke => {
            ctx.pairing.revoke(&device.id).unwrap();
        }
        Interrupt::Disable => {
            ctx.settings_store
                .merge_update(json!({"api_enabled":false}))
                .unwrap();
        }
        Interrupt::Shutdown => {
            shutdown.send(true).unwrap();
        }
    }
    let cancelled = tokio::time::timeout(Duration::from_secs(2), async {
        while !control.cancelled.load(Ordering::Acquire)
            || ctx.current_session_id.read().is_some()
            || *ctx.state_enum.read() != AppStateEnum::Ready
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    server.abort();
    assert!(
        cancelled.is_ok(),
        "{action:?} must interrupt processing before the model returns"
    );
    assert_eq!(*ctx.state_enum.read(), AppStateEnum::Ready);
    if let Some(processing) = desktop_processing {
        let outcome = tokio::time::timeout(Duration::from_secs(2), processing)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(outcome.raw.is_empty());
        assert!(!outcome.injected);
    }
    assert!(ctx.history_store.get_entries(10, 0).unwrap().is_empty());
    assert!(
        !std::iter::from_fn(|| events.try_recv().ok()).any(|event| matches!(
            event,
            DoryEvent::SessionFinished { .. } | DoryEvent::Injection(_)
        ))
    );
    // Late closure of the cancelled connection cannot reclaim the next owner.
    let current = session::start_external_owned(&ctx, Some("en".into()))
        .await
        .unwrap();
    assert_ne!(Some(current), old);
    drop(socket);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(*ctx.current_session_id.read(), Some(current));
    session::cancel_owned_wait(&ctx, Some(current))
        .await
        .unwrap();
}

#[tokio::test]
async fn websocket_processing_cancel_interrupts_asr() {
    processing_interrupt(Interrupt::Cancel, false).await;
}
#[tokio::test]
async fn websocket_processing_disconnect_interrupts_asr() {
    processing_interrupt(Interrupt::Disconnect, false).await;
}
#[tokio::test]
async fn websocket_processing_revocation_interrupts_asr() {
    processing_interrupt(Interrupt::Revoke, false).await;
}
#[tokio::test]
async fn websocket_processing_disable_interrupts_asr() {
    processing_interrupt(Interrupt::Disable, false).await;
}
#[tokio::test]
async fn websocket_processing_shutdown_interrupts_asr() {
    processing_interrupt(Interrupt::Shutdown, false).await;
}

#[tokio::test]
async fn desktop_processing_phone_cancel_interrupts_asr() {
    processing_interrupt(Interrupt::Cancel, true).await;
}
#[tokio::test]
async fn desktop_processing_phone_disconnect_interrupts_asr() {
    processing_interrupt(Interrupt::Disconnect, true).await;
}
#[tokio::test]
async fn desktop_processing_phone_revocation_interrupts_asr() {
    processing_interrupt(Interrupt::Revoke, true).await;
}

#[tokio::test]
async fn websocket_processing_final_reaches_owner_once() {
    let ctx = AppContext::bootstrap_test(std::env::temp_dir().join(format!(
        "reflow_processing_success_{}",
        uuid::Uuid::new_v4()
    )));
    ctx.settings_store
        .merge_update(json!({"api_enabled":true,"processing_mode":"raw"}))
        .unwrap();
    let control = Arc::new(DecodeControl::default());
    ctx.asr_handle
        .swap_engine(Box::new(ControlledAsr(control.clone())))
        .await
        .unwrap();
    let offer = ctx.pairing.rotate_code();
    let (token, _) = ctx.pairing.pair(&offer.code, "successful phone").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = router(ctx.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut socket = connect_socket(address, &token).await;
    send_frame(&mut socket, 1, br#"{"type":"start","inject":false}"#).await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), receive_text(&mut socket))
            .await
            .unwrap()["type"],
        "ready"
    );
    let pcm: Vec<u8> = (0..1600).flat_map(|_| 16000i16.to_le_bytes()).collect();
    send_frame(&mut socket, 2, &pcm).await;
    send_frame(&mut socket, 1, br#"{"type":"stop"}"#).await;
    tokio::time::timeout(Duration::from_secs(3), control.entered.notified())
        .await
        .unwrap();
    control.complete.store(true, Ordering::Release);
    // Completion includes cold hardware probes (each allows five seconds) and
    // history writes. This checks delivery/ownership, not a latency benchmark.
    let final_message = tokio::time::timeout(Duration::from_secs(30), receive_text(&mut socket))
        .await
        .unwrap();
    assert_eq!(final_message["type"], "final");
    assert_eq!(final_message["raw"], "Discard this cancelled transcript");
    assert_eq!(*ctx.state_enum.read(), AppStateEnum::Ready);
    assert!(ctx.current_session_id.read().is_none());
    assert_eq!(ctx.history_store.get_entries(10, 0).unwrap().len(), 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), receive_text(&mut socket))
            .await
            .is_err(),
        "Final must not be duplicated by the bus completion event"
    );
    drop(socket);
    server.abort();
}

async fn completion_after_authority_loss(disable: bool, desktop_stop: bool) {
    let ctx = AppContext::bootstrap_test(std::env::temp_dir().join(format!(
        "reflow_processing_completion_auth_{}",
        uuid::Uuid::new_v4()
    )));
    ctx.settings_store
        .merge_update(json!({"api_enabled":true,"processing_mode":"raw"}))
        .unwrap();
    let control = Arc::new(DecodeControl::default());
    ctx.asr_handle
        .swap_engine(Box::new(ControlledAsr(control.clone())))
        .await
        .unwrap();
    let offer = ctx.pairing.rotate_code();
    let (token, device) = ctx.pairing.pair(&offer.code, "completion phone").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = router(ctx.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut socket = connect_socket(address, &token).await;
    let mut events = ctx.bus.subscribe();
    send_frame(&mut socket, 1, br#"{"type":"start","inject":false}"#).await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), receive_text(&mut socket))
            .await
            .unwrap()["type"],
        "ready"
    );
    let pcm: Vec<u8> = (0..1600).flat_map(|_| 16000i16.to_le_bytes()).collect();
    send_frame(&mut socket, 2, &pcm).await;
    let desktop_processing = if desktop_stop {
        assert_eq!(*ctx.session_phone_token.read(), Some(token.clone()));
        wait_for_phone_audio(&ctx).await;
        let ctx = ctx.clone();
        Some(tokio::spawn(async move { session::stop(&ctx, true).await }))
    } else {
        send_frame(&mut socket, 1, br#"{"type":"stop"}"#).await;
        None
    };
    tokio::time::timeout(Duration::from_secs(3), control.entered.notified())
        .await
        .unwrap();
    // Finish within the one-second poll interval after removing authority.
    if disable {
        ctx.settings_store
            .merge_update(json!({"api_enabled":false}))
            .unwrap();
    } else {
        ctx.pairing.revoke(&device.id).unwrap();
    }
    control.complete.store(true, Ordering::Release);
    let mut byte = [0];
    let read = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte))
        .await
        .unwrap()
        .unwrap();
    server.abort();
    assert_eq!(
        read, 0,
        "An unauthorized completion must close without transmitting a result"
    );
    if let Some(processing) = desktop_processing {
        let outcome = tokio::time::timeout(Duration::from_secs(2), processing)
            .await
            .unwrap()
            .unwrap();
        assert!(
            outcome.is_err(),
            "Desktop processing must retain the phone authority"
        );
    }
    assert!(ctx.history_store.get_entries(10, 0).unwrap().is_empty());
    assert!(
        !std::iter::from_fn(|| events.try_recv().ok()).any(|event| matches!(
            event,
            DoryEvent::SessionFinished { .. } | DoryEvent::Injection(_)
        ))
    );
}

#[tokio::test]
async fn websocket_processing_completion_rechecks_revoked_authority() {
    completion_after_authority_loss(false, false).await;
}

#[tokio::test]
async fn websocket_processing_completion_rechecks_disabled_api() {
    completion_after_authority_loss(true, false).await;
}

#[tokio::test]
async fn desktop_processing_phone_completion_rechecks_revoked_authority() {
    completion_after_authority_loss(false, true).await;
}

#[tokio::test]
async fn desktop_processing_phone_completion_rechecks_disabled_api() {
    completion_after_authority_loss(true, true).await;
}

#[test]
fn processing_completion_boundary_rechecks_authority_after_the_last_poll() {
    for disable in [false, true] {
        let ctx = AppContext::bootstrap_test(std::env::temp_dir().join(format!(
            "reflow_processing_completion_boundary_{}",
            uuid::Uuid::new_v4()
        )));
        ctx.settings_store
            .merge_update(json!({"api_enabled":true}))
            .unwrap();
        let offer = ctx.pairing.rotate_code();
        let (token, device) = ctx.pairing.pair(&offer.code, "completion phone").unwrap();
        let scope = ListenerScope { localhost: true };
        let outcome = || {
            Ok(session::StopOutcome {
                raw: "private dictated text".into(),
                final_text: "Private dictated text.".into(),
                language: "en".into(),
                injected: false,
            })
        };
        assert!(processing_message(&ctx, &token, scope, outcome()).is_some());
        // Authority is lost after the loop's poll but before processing returns.
        if disable {
            ctx.settings_store
                .merge_update(json!({"api_enabled":false}))
                .unwrap();
        } else {
            ctx.pairing.revoke(&device.id).unwrap();
        }
        assert!(
            processing_message(&ctx, &token, scope, outcome()).is_none(),
            "A completion must recheck authority immediately before disclosure"
        );
    }
}
