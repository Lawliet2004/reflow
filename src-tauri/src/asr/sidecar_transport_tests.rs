//! Transport regressions use a real child pipe, without loading a model.
use super::*;

/// The reader intentionally leaves stdin untouched. Its finite lifetime is a
/// watchdog, so the pre-fix synchronous write cannot hang the entire test run.
fn stalled_reader() -> Child {
    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new("powershell");
        command.args([
            "-NoProfile",
            "-Command",
            "[Console]::Out.WriteLine('ready'); Start-Sleep -Seconds 3",
        ]);
        command.creation_flags(0x08000000);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("sh");
        command.args(["-c", "printf 'ready\\n'; exec sleep 3"]);
        command
    };
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start the local stalled-reader fixture");
    let mut ready = String::new();
    BufReader::new(child.stdout.as_mut().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready.trim(), "ready");
    child
}

#[test]
fn stalled_stdin_write_obeys_the_command_deadline() {
    let mut child = stalled_reader();
    let mut engine = Qwen3AsrSidecar::new();
    engine.stdin = child
        .stdin
        .take()
        .map(|stdin| Arc::new(parking_lot::Mutex::new(stdin)));
    engine.responses = child.stdout.take().map(spawn_response_reader);
    engine.child = Some(child);
    // This exceeds anonymous-pipe capacity on supported platforms, ensuring
    // that the no-reader fixture blocks an unbounded synchronous writer.
    let command = json!({"cmd": "start_stream", "vocabulary": "x".repeat(8 * 1024 * 1024)});
    let budget = Duration::from_millis(50);
    let started = std::time::Instant::now();
    let result = engine.send_command_timeout(command, budget);
    let elapsed = started.elapsed();
    // Complete teardown before asserting: even the pre-fix implementation
    // releases its writer when the fixture's watchdog closes the input pipe.
    if let Some(mut child) = engine.child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    engine.stdin = None;
    engine.responses = None;
    assert!(
        elapsed < Duration::from_millis(500),
        "a {budget:?} command blocked {elapsed:?} before reaching its read timeout"
    );
    assert!(
        result.as_ref().is_err_and(|error| is_engine_timeout(error)),
        "stalled writes must produce the same recognizable timeout as stalled replies: {result:?}"
    );
}

#[test]
fn cancellation_callback_never_waits_for_a_stalled_pipe_writer() {
    // Prepare bytes before starting the fixture's watchdog. Debug JSON encoding
    // of a large command can exceed the setup deadline on slower CI hosts.
    let frame = vec![b'x'; 8 * 1024 * 1024];
    let mut child = stalled_reader();
    let mut engine = Qwen3AsrSidecar::new();
    engine.stdin = child
        .stdin
        .take()
        .map(|stdin| Arc::new(parking_lot::Mutex::new(stdin)));
    engine.responses = child.stdout.take().map(spawn_response_reader);
    engine.child = Some(child);
    let writer = engine.writer().unwrap();
    let stdin = Arc::clone(engine.stdin.as_ref().unwrap());
    let cancel = engine.cancellation_signal().unwrap();
    let blocked = writer.enqueue(frame).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let observed_blocked_writer = loop {
        if stdin.try_lock().is_none() {
            break true;
        }
        if std::time::Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let started = std::time::Instant::now();
    cancel();
    let elapsed = started.elapsed();
    // Always close the child before asserting, releasing the blocked OS write.
    engine.kill_child();
    let result = blocked.recv_timeout(Duration::from_secs(2));
    assert!(
        observed_blocked_writer,
        "fixture never filled its input pipe"
    );
    assert!(
        elapsed < Duration::from_millis(100),
        "cancellation callback waited {elapsed:?}"
    );
    assert!(writer.cancelled.load(Ordering::Acquire));
    assert!(
        matches!(result, Ok(Err(_))),
        "fixture write did not block until teardown: {result:?}"
    );
}

#[test]
fn cancelled_python_stop_discards_text_without_waiting_for_a_stalled_reader() {
    let mut child = stalled_reader();
    let mut engine = Qwen3AsrSidecar::new();
    engine.stdin = child
        .stdin
        .take()
        .map(|stdin| Arc::new(parking_lot::Mutex::new(stdin)));
    engine.responses = child.stdout.take().map(spawn_response_reader);
    engine.child = Some(child);
    engine.writer().unwrap();
    let cancel = engine.cancellation_signal().unwrap();
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    let stopping = std::thread::spawn(move || {
        let _ = finished_tx.send(engine.stop_stream());
    });
    cancel();
    let promptly_finished = finished_rx.recv_timeout(Duration::from_millis(750));
    stopping.join().unwrap();
    assert!(
        matches!(&promptly_finished, Ok(Ok(text)) if text.is_empty()),
        "cancel must discard this request without awaiting the stalled reader: {promptly_finished:?}"
    );
}
