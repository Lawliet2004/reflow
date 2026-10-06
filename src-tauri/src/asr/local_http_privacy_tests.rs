use super::*;
use crate::rewrite::{FlowClient, RewriteRequest};
use std::io::BufRead;
use std::net::TcpListener;

const CHILD: &str = "REFLOW_LOCAL_HTTP_PRIVACY_CHILD";
const TEST: &str =
    "asr::native::privacy_tests::local_inference_bypasses_proxies_and_never_follows_redirects";

struct HttpFixture {
    url: String,
    requests: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl HttpFixture {
    fn new(status: u16, location: Option<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let collected = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = std::thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                let (stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                };
                // Winsock accepts can inherit the listener's nonblocking mode.
                // Request parsing waits for headers/body under socket timeouts.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("").to_owned();
                let mut length = 0;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length: ")
                    {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                let body = if bytes.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::from_slice(&bytes).unwrap()
                };
                collected.lock().push((path.clone(), body));
                let payload = if status != 200 {
                    json!({})
                } else if path == "/apply-template" {
                    json!({"prompt": "fixture rendered template"})
                } else if path == "/tokenize" {
                    json!({"tokens": [1, 2, 3]})
                } else {
                    json!({"choices": [{"message": {"content": "fixture transcript"}, "finish_reason": "stop"}]})
                }
                .to_string();
                let redirect = location
                    .as_ref()
                    .map_or(String::new(), |url| format!("Location: {url}\r\n"));
                write!(reader.get_mut(),
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{redirect}Connection: close\r\n\r\n{payload}", payload.len()).unwrap();
            }
        });
        Self {
            url,
            requests,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn rewrite_request() -> RewriteRequest {
    RewriteRequest {
        text: "Private dictated words and captured context must stay on this computer. ".repeat(3),
        cleanup_level: "medium".into(),
        style: "neutral".into(),
        dictation_mode: "normal".into(),
        vocabulary: Vec::new(),
        app_process: String::new(),
        model_id: "qwen3.5-0.8b".into(),
    }
}

fn native_request(url: &str) -> Result<String, String> {
    let mut engine = NativeAsrEngine::default();
    *engine.runtime.client.write() = FlowClient::new_url(url.into(), Duration::from_secs(2));
    engine.push_audio(&vec![0.1; SAMPLE_RATE]).unwrap();
    engine.stop_stream()
}

#[test]
fn http_fixture_waits_for_request_bytes_after_accept() {
    let fixture = HttpFixture::new(200, None);
    let mut stream =
        std::net::TcpStream::connect(fixture.url.strip_prefix("http://").unwrap()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    // The listener accepts before the client's first write. A socket inheriting
    // nonblocking mode must not be mistaken for a failed/closed connection.
    std::thread::sleep(Duration::from_millis(100));
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 "), "{response:?}");
    assert_eq!(fixture.requests.lock().len(), 1);
}

#[test]
fn local_inference_bypasses_proxies_and_never_follows_redirects() {
    if std::env::var(CHILD).as_deref() != Ok("1") {
        // Proxy configuration is passed only to a fresh, single-test process.
        // Other tests may have initialized FLOW_HTTP or be using the environment.
        let proxy = HttpFixture::new(502, None);
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", TEST, "--nocapture"])
            .env(CHILD, "1")
            .env("NO_PROXY", "")
            .env("no_proxy", "")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        for name in [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ] {
            command.env(name, &proxy.url);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("privacy child exceeded its deadline");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "privacy child failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            proxy.requests.lock().is_empty(),
            "private requests reached the configured proxy"
        );
        return;
    }

    let direct = HttpFixture::new(200, None);
    let client = FlowClient::new_url(direct.url.clone(), Duration::from_secs(2));
    assert_eq!(
        client.rewrite(&rewrite_request()).unwrap(),
        "fixture transcript"
    );
    assert_eq!(native_request(&direct.url).unwrap(), "fixture transcript");
    assert!(crate::rewrite::server::local_health_client()
        .unwrap()
        .get(format!("{}/health", direct.url))
        .send()
        .unwrap()
        .status()
        .is_success());
    let requests = direct.requests.lock();
    assert!(requests.iter().any(|(path, _)| path == "/apply-template"));
    assert!(requests.iter().any(|(path, _)| path == "/tokenize"));
    assert!(requests.iter().any(|(path, _)| path == "/health"));
    let completions = requests
        .iter()
        .filter(|(path, _)| path == "/v1/chat/completions")
        .collect::<Vec<_>>();
    assert_eq!(completions.len(), 2);
    assert!(completions[0]
        .1
        .to_string()
        .contains("Private dictated words"));
    assert!(
        completions[1].1["messages"][1]["content"][0]["input_audio"]["data"]
            .as_str()
            .is_some_and(|data| !data.is_empty())
    );
    drop(requests);

    let destination = HttpFixture::new(200, None);
    for status in [302, 307] {
        let redirect = HttpFixture::new(status, Some(format!("{}/private-leak", destination.url)));
        let client = FlowClient::new_url(redirect.url.clone(), Duration::from_secs(2));
        assert!(client.rewrite(&rewrite_request()).is_err());
        assert!(native_request(&redirect.url).is_err());
        assert_eq!(
            crate::rewrite::server::local_health_client()
                .unwrap()
                .get(format!("{}/health", redirect.url))
                .send()
                .unwrap()
                .status()
                .as_u16(),
            status
        );
        assert!(
            destination.requests.lock().is_empty(),
            "local request followed HTTP {status} to another endpoint"
        );
    }
}
