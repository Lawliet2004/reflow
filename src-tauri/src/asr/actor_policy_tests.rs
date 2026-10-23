//! Model-free ownership tests. The Python fixture uses only stdlib and loopback.
use super::*;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

struct Fixture(PathBuf);

impl Fixture {
    fn new(port: u16) -> Self {
        let root =
            std::env::temp_dir().join(format!("reflow-policy-actor-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("network-policy.json"),
            br#"{"offline_mode":false}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("qwen3_asr_runtime.py"),
            r#"
import json, os, pathlib, socket, sys, threading, time
root = pathlib.Path(__file__).parent
(root / 'pid').write_text(str(os.getpid()))
def network():
    if not __PORT__:
        return
    if json.loads((root / 'network-policy.json').read_text())['offline_mode']:
        return
    with socket.create_connection(('127.0.0.1', __PORT__)) as connection:
        while not json.loads((root / 'network-policy.json').read_text())['offline_mode']:
            connection.sendall(b'x')
            time.sleep(0.01)
threading.Thread(target=network, daemon=True).start()
for line in sys.stdin:
    message = json.loads(line)
    if message['cmd'] == 'stop_stream':
        (root / 'blocked').write_text('waiting')
        time.sleep(30)
        continue
    print(json.dumps({'id': message['id'], 'status': 'ok', 'pong': True,
                      'loaded': True, 'is_loading': False, 'backend': 'fixture',
                      'cuda_available': False, 'cuda_probe_pending': False}), flush=True)
"#
            .replace("__PORT__", &port.to_string()),
        )
        .unwrap();
        Self(root)
    }

    fn engine(&self) -> Qwen3AsrSidecar {
        let mut engine = Qwen3AsrSidecar::new();
        engine.set_resource_dir(self.0.clone());
        engine.set_skip_dep_check(true);
        engine
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "fixture did not reach the required state"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn accept(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let mut connection = None;
    wait_until(|| match listener.accept() {
        Ok((stream, _)) => {
            // Windows can inherit the listener's nonblocking mode. The fixture
            // reads below use explicit timeouts on a blocking connection.
            stream.set_nonblocking(false).unwrap();
            connection = Some(stream);
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => false,
        Err(error) => panic!("loopback fixture failed: {error}"),
    });
    connection.unwrap()
}

#[test]
fn failed_policy_save_interrupts_a_busy_actor_and_closes_its_owned_connection() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let fixture = Fixture::new(listener.local_addr().unwrap().port());
    let unrelated_fixture = Fixture::new(0);
    let mut unrelated = unrelated_fixture.engine();
    unrelated.initialize().unwrap();
    let handle = AsrHandle::spawn(Box::new(fixture.engine()), 8);
    handle.initialize_blocking().unwrap();
    let mut connection = accept(&listener);
    connection
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut byte = [0; 1];
    connection.read_exact(&mut byte).unwrap();
    connection
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let session = handle.next_session_id();
    handle.start_stream_blocking(session, "en", &[]).unwrap();
    let stopping = handle.clone();
    let blocked = std::thread::spawn(move || stopping.stop_stream_blocking(session));
    wait_until(|| fixture.0.join("blocked").is_file());
    let bad_parent = fixture.0.join("not-a-directory");
    std::fs::write(&bad_parent, b"incumbent").unwrap();
    let started = Instant::now();
    let failure = handle.update_network_policy(|| {
        crate::platform::persistence::write_atomic(
            &bad_parent.join("network-policy.json"),
            br#"{"offline_mode":true}"#,
        )
        .map_err(|error| error.to_string())
    });
    assert!(failure.is_err());
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(!handle.engine_status().loaded);
    assert!(handle
        .engine_status()
        .error
        .unwrap()
        .contains("network policy"));
    assert_eq!(
        std::fs::read(fixture.0.join("network-policy.json")).unwrap(),
        br#"{"offline_mode":false}"#
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = [0; 1024];
    loop {
        match connection.read(&mut bytes) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                ) =>
            {
                break
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => panic!("unexpected loopback error: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "the old online runtime retained its connection"
        );
    }
    assert!(blocked.join().unwrap().is_err());
    assert!(handle
        .initialize_blocking()
        .unwrap_err()
        .contains("network policy"));
    assert!(
        unrelated.engine_status().loaded,
        "revocation must target only its owned process"
    );
    unrelated.unload_model().unwrap();
}

#[test]
fn policy_failure_before_start_survives_engine_swap_and_a_valid_repair_allows_restart() {
    let fixture = Fixture::new(0);
    let handle = AsrHandle::spawn(Box::new(fixture.engine()), 8);
    assert!(handle
        .update_network_policy(|| Err("policy write failed".into()))
        .is_err());
    assert!(handle
        .initialize_blocking()
        .unwrap_err()
        .contains("network policy"));
    assert!(!fixture.0.join("pid").exists());
    handle
        .swap_engine_blocking(Box::new(fixture.engine()))
        .unwrap();
    assert!(handle
        .initialize_blocking()
        .unwrap_err()
        .contains("network policy"));
    assert!(!fixture.0.join("pid").exists());
    let recovered = handle
        .update_network_policy(|| {
            crate::platform::persistence::write_atomic(
                &fixture.0.join("network-policy.json"),
                br#"{"offline_mode":true}"#,
            )
            .map_err(|error| error.to_string())
        })
        .unwrap();
    assert!(recovered);
    assert!(
        !handle.engine_status().loaded,
        "saving policy alone cannot claim ASR readiness"
    );
    handle.initialize_blocking().unwrap();
    assert!(handle.engine_status().loaded);
    assert!(fixture.0.join("pid").exists());
    assert!(!handle.update_network_policy(|| Ok(())).unwrap());
    assert!(
        handle.engine_status().loaded,
        "an ordinary save must not reload a healthy runtime"
    );
    handle.unload_model_blocking().unwrap();
}

#[test]
fn policy_failure_does_not_unload_an_engine_without_an_outbound_sidecar() {
    let handle = AsrHandle::new_mock();
    handle.initialize_blocking().unwrap();
    handle
        .load_model_with_precision_blocking("fixture", "cpu", "auto")
        .unwrap();
    assert!(handle.is_model_loaded());
    assert!(handle
        .update_network_policy(|| Err("policy write failed".into()))
        .is_err());
    assert!(handle.is_model_loaded());
    assert!(!handle.update_network_policy(|| Ok(())).unwrap());
}
