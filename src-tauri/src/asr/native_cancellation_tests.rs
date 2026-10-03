//! A held localhost response exercises cancellation during a real HTTP decode.
use super::*;
use std::io::BufRead;

#[test]
fn native_cancel_finishes_while_the_server_withholds_its_response() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (closed_tx, closed_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = std::io::BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.starts_with("POST /v1/chat/completions "));
        let mut length = 0;
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length: ") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        accepted_tx.send(()).unwrap();
        reader
            .get_mut()
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut extra = [0u8; 1];
        let closed = match reader.read(&mut extra) {
            Ok(0) => true,
            Err(error) => matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            ),
            _ => false,
        };
        let _ = closed_tx.send(closed);
        // Cancellation is tested before this channel allows any response.
        // A finite watchdog also guarantees cleanup after a failed assertion.
        let _ = release_rx.recv_timeout(Duration::from_secs(5));
        let result = r#"{"choices":[{"message":{"content":"language English<asr_text>late result"},"finish_reason":"stop"}]}"#;
        let _ = write!(
            reader.get_mut(),
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{result}",
            result.len()
        );
    });
    let mut engine = NativeAsrEngine::default();
    *engine.runtime.client.write() =
        crate::rewrite::FlowClient::new_url(format!("http://{address}"), Duration::from_secs(2));
    engine.push_audio(&vec![0.1; SAMPLE_RATE]).unwrap();
    let cancel = engine.cancellation_signal().unwrap();
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    let decoding = std::thread::spawn(move || {
        let result = engine.stop_stream();
        let _ = finished_tx.send(result);
    });
    let accepted = accepted_rx.recv_timeout(Duration::from_secs(5));
    if accepted.is_err() {
        let _ = release_tx.send(());
        server.join().unwrap();
        decoding.join().unwrap();
        panic!("native request did not reach the held-response fixture: {accepted:?}");
    }
    cancel();
    let promptly_finished = finished_rx.recv_timeout(Duration::from_millis(500));
    let connection_closed = closed_rx.recv_timeout(Duration::from_millis(500));
    // Release and join all resources before checking the result, so the old
    // implementation fails in under a second instead of hanging for 180 s.
    let _ = release_tx.send(());
    server.join().unwrap();
    decoding.join().unwrap();
    assert!(
        matches!(&promptly_finished, Ok(Ok(text)) if text.is_empty()),
        "cancel must finish without waiting for the server response: {promptly_finished:?}"
    );
    assert!(
        matches!(connection_closed, Ok(true)),
        "cancelled decode must release its HTTP connection: {connection_closed:?}"
    );
}
