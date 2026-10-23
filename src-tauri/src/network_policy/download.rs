//! Interruptible pinned-artifact transfers shared by runtime and model setup.
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub struct ArtifactDownload<'a> {
    pub url: &'a str,
    pub sha256: &'a str,
    pub exact_bytes: Option<u64>,
    pub partial: &'a Path,
    pub destination: &'a Path,
}

#[derive(Clone, Copy, Debug)]
pub struct TransferProgress {
    pub phase: &'static str,
    pub bytes: u64,
    pub total: Option<u64>,
}

/// Bridge for the existing dedicated installer threads. A local runtime owns
/// the response until it completes or cancels; no detached network worker survives.
pub fn fetch_verified_blocking(
    spec: ArtifactDownload<'_>,
    cancel: &AtomicBool,
    progress: impl FnMut(TransferProgress),
) -> Result<(), String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("Could not start artifact transfer: {error}"))?
        .block_on(fetch_verified(spec, cancel, progress))
}

/// The caller owns one destination/partial pair until this future terminates.
/// Cancellation interrupts both response headers and stalled response bodies.
pub async fn fetch_verified(
    spec: ArtifactDownload<'_>,
    cancel: &AtomicBool,
    progress: impl FnMut(TransferProgress),
) -> Result<(), String> {
    let url = reqwest::Url::parse(spec.url).map_err(|error| error.to_string())?;
    super::validate_download_url(&url)?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .redirect(super::redirect_policy())
        .build()
        .map_err(|error| error.to_string())?;
    fetch_inner(&client, spec, cancel, progress, super::check_download).await
}

async fn interruptible<T>(
    future: impl std::future::Future<Output = T>,
    cancel: &AtomicBool,
    policy: fn() -> Result<(), String>,
) -> Result<T, String> {
    tokio::pin!(future);
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            biased;
            _ = tick.tick() => {
                if cancel.load(Ordering::Acquire) { return Err("Download cancelled".into()); }
                policy()?;
            }
            result = &mut future => return Ok(result),
        }
    }
}

async fn matches_hash(
    path: &Path,
    expected: &str,
    cancel: &AtomicBool,
    policy: fn() -> Result<(), String>,
) -> Result<bool, String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| error.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("Download cancelled".into());
        }
        policy()?;
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hex::encode(hash.finalize()).eq_ignore_ascii_case(expected))
}

fn resume_offset(
    response: &reqwest::Response,
    offset: u64,
    expected: Option<u64>,
) -> Result<u64, String> {
    if response.status() == reqwest::StatusCode::OK {
        return Ok(0);
    }
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(format!("Download failed: HTTP {}", response.status()));
    }
    let value = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or("Missing resumed Content-Range")?;
    let (range, total) = value
        .strip_prefix("bytes ")
        .and_then(|value| value.split_once('/'))
        .ok_or("Invalid resumed Content-Range")?;
    let (start, end) = range.split_once('-').ok_or("Invalid resumed byte range")?;
    let start = start.parse::<u64>().map_err(|_| "Invalid resumed offset")?;
    let end = end.parse::<u64>().map_err(|_| "Invalid resumed end")?;
    let total = total.parse::<u64>().map_err(|_| "Invalid resumed total")?;
    if start != offset
        || end < start
        || end >= total
        || expected.is_some_and(|expected| expected != total)
    {
        return Err("Resumed byte range does not match the pinned artifact".into());
    }
    if response
        .content_length()
        .is_some_and(|length| length != end - start + 1)
    {
        return Err("Resumed length does not match Content-Range".into());
    }
    Ok(offset)
}

struct Traffic {
    host: String,
    bytes: u64,
}
impl Drop for Traffic {
    fn drop(&mut self) {
        super::record(&self.host, self.bytes);
    }
}

async fn fetch_inner(
    client: &reqwest::Client,
    spec: ArtifactDownload<'_>,
    cancel: &AtomicBool,
    mut progress: impl FnMut(TransferProgress),
    policy: fn() -> Result<(), String>,
) -> Result<(), String> {
    if spec.sha256.len() != 64
        || !spec.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        || spec.exact_bytes == Some(0)
    {
        return Err("Artifact needs a pinned SHA-256 and nonzero size".into());
    }
    if spec.destination.is_file()
        && spec.exact_bytes.is_none_or(|bytes| {
            std::fs::metadata(spec.destination).is_ok_and(|metadata| metadata.len() == bytes)
        })
        // Reading an already-downloaded artifact is local work. Offline mode
        // blocks the first actual transfer below, while cached repair remains
        // available and still verifies every byte and cancellation.
        && matches_hash(spec.destination, spec.sha256, cancel, || Ok(())).await?
    {
        progress(TransferProgress {
            phase: "committing",
            bytes: spec.exact_bytes.unwrap_or(0),
            total: spec.exact_bytes,
        });
        if cancel.load(Ordering::Acquire) {
            return Err("Download cancelled".into());
        }
        return Ok(());
    }
    for attempt in 0..3 {
        policy()?;
        if cancel.load(Ordering::Acquire) {
            return Err("Download cancelled".into());
        }
        let mut offset = tokio::fs::metadata(spec.partial)
            .await
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        if offset > 0
            && spec.exact_bytes.is_none_or(|expected| offset == expected)
            && matches_hash(spec.partial, spec.sha256, cancel, policy).await?
        {
            break;
        }
        if spec.exact_bytes.is_some_and(|expected| offset >= expected) {
            tokio::fs::remove_file(spec.partial)
                .await
                .map_err(|error| error.to_string())?;
            offset = 0;
        }
        let mut request = client.get(spec.url);
        if offset > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
        }
        let response = interruptible(request.send(), cancel, policy).await?;
        let retry_after = response
            .as_ref()
            .ok()
            .and_then(|response| response.headers().get(reqwest::header::RETRY_AFTER))
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .map(|seconds| Duration::from_secs(seconds.min(30)))
            .unwrap_or(Duration::from_millis(250 * (attempt + 1)));
        let outcome = async {
            let mut response =
                response.map_err(|error| crate::rewrite::runtime_install::error_chain(&error))?;
            let offset = resume_offset(&response, offset, spec.exact_bytes)?;
            let total = spec.exact_bytes.or_else(|| {
                response
                    .content_length()
                    .map(|bytes| bytes.saturating_add(offset))
            });
            let maximum = spec.exact_bytes.unwrap_or(4 * 1024 * 1024 * 1024);
            let mut traffic = Traffic {
                host: response.url().host_str().unwrap_or_default().into(),
                bytes: 0,
            };
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(offset > 0)
                .truncate(offset == 0)
                .open(spec.partial)
                .await
                .map_err(|error| error.to_string())?;
            let mut bytes = offset;
            progress(TransferProgress {
                phase: "downloading",
                bytes,
                total,
            });
            let mut last_emit = std::time::Instant::now();
            let received = async {
                while let Some(chunk) = interruptible(response.chunk(), cancel, policy)
                    .await?
                    .map_err(|error| crate::rewrite::runtime_install::error_chain(&error))?
                {
                    traffic.bytes = traffic.bytes.saturating_add(chunk.len() as u64);
                    bytes = bytes
                        .checked_add(chunk.len() as u64)
                        .ok_or("Artifact size overflow")?;
                    if bytes > maximum {
                        return Err("Download exceeded the pinned artifact size".into());
                    }
                    file.write_all(&chunk)
                        .await
                        .map_err(|error| error.to_string())?;
                    if traffic.bytes == chunk.len() as u64
                        || last_emit.elapsed() >= Duration::from_millis(250)
                        || total == Some(bytes)
                    {
                        progress(TransferProgress {
                            phase: "downloading",
                            bytes,
                            total,
                        });
                        last_emit = std::time::Instant::now();
                    }
                }
                Ok::<(), String>(())
            }
            .await;
            file.flush().await.map_err(|error| error.to_string())?;
            received?;
            file.sync_all().await.map_err(|error| error.to_string())?;
            if spec.exact_bytes.is_some_and(|expected| expected != bytes) {
                return Err("Download ended before the pinned artifact was complete".into());
            }
            Ok(())
        }
        .await;
        match outcome {
            Ok(()) => break,
            Err(error) => {
                if cancel.load(Ordering::Acquire) {
                    return Err("Download cancelled".into());
                }
                policy()?;
                if attempt == 2 {
                    return Err(error);
                }
                interruptible(tokio::time::sleep(retry_after), cancel, policy).await?;
            }
        }
    }
    progress(TransferProgress {
        phase: "verifying",
        bytes: tokio::fs::metadata(spec.partial)
            .await
            .map(|metadata| metadata.len())
            .unwrap_or(0),
        total: spec.exact_bytes,
    });
    if !matches_hash(spec.partial, spec.sha256, cancel, policy).await? {
        let _ = tokio::fs::remove_file(spec.partial).await;
        return Err("Artifact SHA-256 verification failed; retry the download".into());
    }
    progress(TransferProgress {
        phase: "committing",
        bytes: tokio::fs::metadata(spec.partial)
            .await
            .map(|metadata| metadata.len())
            .unwrap_or(0),
        total: spec.exact_bytes,
    });
    if cancel.load(Ordering::Acquire) {
        return Err("Download cancelled".into());
    }
    policy()?;
    // On failure the incumbent and verified partial both survive. Never remove
    // the working destination before the replacement has been committed.
    tokio::fs::rename(spec.partial, spec.destination)
        .await
        .map_err(|error| format!("Could not activate verified artifact: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("reflow-transfer-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn allowed() -> Result<(), String> {
        Ok(())
    }
    fn digest(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    #[tokio::test]
    async fn offline_reuses_only_a_complete_verified_cache_without_a_request() {
        let root = Fixture::new();
        let destination = root.0.join("artifact");
        let partial = root.0.join("artifact.partial");
        tokio::fs::write(&destination, b"cached").await.unwrap();
        let expected = digest(b"cached");
        let cancel = AtomicBool::new(false);
        for valid in [true, false] {
            if !valid {
                tokio::fs::write(&destination, b"broken").await.unwrap();
            }
            let result = fetch_inner(
                &reqwest::Client::new(),
                ArtifactDownload {
                    url: "https://github.com/example/nonexistent",
                    sha256: &expected,
                    exact_bytes: Some(6),
                    partial: &partial,
                    destination: &destination,
                },
                &cancel,
                |_| {},
                || Err("offline".into()),
            )
            .await;
            if valid {
                result.unwrap();
            } else {
                assert_eq!(result.unwrap_err(), "offline");
            }
        }
        assert!(!partial.exists());
    }

    async fn server(responses: Vec<String>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/artifact", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                }
                requests.push(String::from_utf8(request).unwrap().to_ascii_lowercase());
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            }
            requests
        });
        (url, task)
    }

    #[tokio::test]
    async fn resumes_valid_ranges_and_restarts_when_the_server_ignores_them() {
        for (status, range, body) in [
            (
                "206 Partial Content",
                "Content-Range: bytes 3-5/6\r\n",
                "def",
            ),
            ("200 OK", "", "abcdef"),
        ] {
            let fixture = Fixture::new();
            let partial = fixture.0.join("artifact.part");
            let destination = fixture.0.join("artifact");
            std::fs::write(&partial, b"abc").unwrap();
            std::fs::write(&destination, b"incumbent").unwrap();
            let (url, server) = server(vec![format!(
                "HTTP/1.1 {status}\r\n{range}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )])
            .await;
            let hash = digest(b"abcdef");
            fetch_inner(
                &reqwest::Client::new(),
                ArtifactDownload {
                    url: &url,
                    sha256: &hash,
                    exact_bytes: Some(6),
                    partial: &partial,
                    destination: &destination,
                },
                &AtomicBool::new(false),
                |_| {},
                allowed,
            )
            .await
            .unwrap();
            assert_eq!(std::fs::read(&destination).unwrap(), b"abcdef");
            assert!(!partial.exists());
            assert!(server.await.unwrap()[0].contains("range: bytes=3-"));
        }
    }

    #[tokio::test]
    async fn checksum_failure_never_replaces_the_incumbent() {
        let fixture = Fixture::new();
        let partial = fixture.0.join("artifact.part");
        let destination = fixture.0.join("artifact");
        std::fs::write(&destination, b"incumbent").unwrap();
        let (url, server) = server(vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nwrong!".into(),
        ])
        .await;
        let hash = digest(b"abcdef");
        let error = fetch_inner(
            &reqwest::Client::new(),
            ArtifactDownload {
                url: &url,
                sha256: &hash,
                exact_bytes: Some(6),
                partial: &partial,
                destination: &destination,
            },
            &AtomicBool::new(false),
            |_| {},
            allowed,
        )
        .await
        .unwrap_err();
        assert!(error.contains("SHA-256"));
        assert_eq!(std::fs::read(&destination).unwrap(), b"incumbent");
        assert!(!partial.exists());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn interrupted_body_resumes_from_flushed_bytes_with_a_validated_range() {
        let fixture = Fixture::new();
        let partial = fixture.0.join("artifact.part");
        let destination = fixture.0.join("artifact");
        let (url, server) = server(vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabc".into(),
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 3-5/6\r\nContent-Length: 3\r\nConnection: close\r\n\r\ndef".into(),
        ]).await;
        fetch_inner(
            &reqwest::Client::new(),
            ArtifactDownload {
                url: &url,
                sha256: &digest(b"abcdef"),
                exact_bytes: Some(6),
                partial: &partial,
                destination: &destination,
            },
            &AtomicBool::new(false),
            |_| {},
            allowed,
        )
        .await
        .unwrap();
        let requests = server.await.unwrap();
        assert!(!requests[0].contains("range:"));
        assert!(requests[1].contains("range: bytes=3-"));
        assert_eq!(std::fs::read(destination).unwrap(), b"abcdef");
    }

    #[tokio::test]
    async fn incorrect_resumed_ranges_never_append_or_replace_working_files() {
        let fixture = Fixture::new();
        let partial = fixture.0.join("artifact.part");
        let destination = fixture.0.join("artifact");
        std::fs::write(&partial, b"abc").unwrap();
        std::fs::write(&destination, b"incumbent").unwrap();
        let (url, server) = server(vec![
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-2/6\r\nContent-Length: 3\r\nConnection: close\r\n\r\ndef".into(); 3
        ]).await;
        let error = fetch_inner(
            &reqwest::Client::new(),
            ArtifactDownload {
                url: &url,
                sha256: &digest(b"abcdef"),
                exact_bytes: Some(6),
                partial: &partial,
                destination: &destination,
            },
            &AtomicBool::new(false),
            |_| {},
            allowed,
        )
        .await
        .unwrap_err();
        assert!(error.contains("byte range"));
        assert_eq!(std::fs::read(partial).unwrap(), b"abc");
        assert_eq!(std::fs::read(destination).unwrap(), b"incumbent");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn cancellation_interrupts_stalled_headers_without_an_orphan_download() {
        let fixture = Fixture::new();
        let partial = fixture.0.join("artifact.part");
        let destination = fixture.0.join("artifact");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/artifact", listener.local_addr().unwrap());
        let (accepted, connection) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            accepted.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        let cancel = AtomicBool::new(false);
        let hash = digest(b"abcdef");
        let client = reqwest::Client::new();
        let transfer = fetch_inner(
            &client,
            ArtifactDownload {
                url: &url,
                sha256: &hash,
                exact_bytes: Some(6),
                partial: &partial,
                destination: &destination,
            },
            &cancel,
            |_| {},
            allowed,
        );
        tokio::pin!(transfer);
        tokio::select! { _ = connection => {}, result = &mut transfer => panic!("unexpected completion {result:?}") }
        cancel.store(true, Ordering::Release);
        assert!(tokio::time::timeout(Duration::from_secs(1), transfer)
            .await
            .unwrap()
            .unwrap_err()
            .contains("cancelled"));
        assert!(!destination.exists());
        server.abort();
    }

    #[tokio::test]
    async fn cancel_or_offline_interrupts_a_stalled_body_and_keeps_resumable_bytes() {
        use crate::network_policy::{
            apply_policy, check_gate, OFFLINE, OFFLINE_ERROR, ONLINE, POLICY_SAVE_ERROR,
            POLICY_SAVE_FAILED,
        };
        use std::sync::atomic::AtomicU8;
        static POLICY: AtomicU8 = AtomicU8::new(ONLINE);
        fn policy() -> Result<(), String> {
            check_gate(&POLICY)
        }
        for reason in [None, Some(OFFLINE), Some(POLICY_SAVE_FAILED)] {
            POLICY.store(ONLINE, Ordering::SeqCst);
            let fixture = Fixture::new();
            let partial = fixture.0.join("artifact.part");
            let destination = fixture.0.join("artifact");
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/artifact", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0u8; 1024];
                assert!(socket.read(&mut buffer).await.unwrap() > 0);
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nabc")
                    .await
                    .unwrap();
                std::future::pending::<()>().await;
            });
            let (sent, received) = tokio::sync::oneshot::channel();
            let mut sent = Some(sent);
            let cancel = AtomicBool::new(false);
            let hash = digest(b"abcdef");
            let client = reqwest::Client::new();
            let transfer = fetch_inner(
                &client,
                ArtifactDownload {
                    url: &url,
                    sha256: &hash,
                    exact_bytes: Some(6),
                    partial: &partial,
                    destination: &destination,
                },
                &cancel,
                |progress| {
                    if progress.bytes == 3 {
                        if let Some(sender) = sent.take() {
                            let _ = sender.send(());
                        }
                    }
                },
                policy,
            );
            tokio::pin!(transfer);
            tokio::select! { _ = received => {}, result = &mut transfer => panic!("transfer ended before stall: {result:?}") }
            match reason {
                Some(POLICY_SAVE_FAILED) => {
                    let invalid_dir = fixture.0.join("invalid-policy-dir");
                    std::fs::write(&invalid_dir, b"incumbent").unwrap();
                    assert!(apply_policy(false, &invalid_dir, &POLICY).is_err());
                    assert_eq!(std::fs::read(invalid_dir).unwrap(), b"incumbent");
                }
                Some(state) => POLICY.store(state, Ordering::SeqCst),
                None => cancel.store(true, Ordering::Release),
            }
            let error = tokio::time::timeout(Duration::from_secs(1), transfer)
                .await
                .unwrap()
                .unwrap_err();
            assert!(error.contains(match reason {
                Some(POLICY_SAVE_FAILED) => POLICY_SAVE_ERROR,
                Some(_) => OFFLINE_ERROR,
                None => "cancelled",
            }));
            assert_eq!(std::fs::read(&partial).unwrap(), b"abc");
            assert!(!destination.exists());
            server.abort();
        }
    }
}
