//! The only outbound network boundary: explicit, pinned artifact downloads.
//! Local ASR/LLM HTTP never passes through this module.
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

static OFFLINE: AtomicBool = AtomicBool::new(true);
static DATA_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
static JOURNAL_LOCK: Mutex<()> = Mutex::new(());
pub const OFFLINE_ERROR: &str = "Offline mode is on";

/// Call at startup, before any model load, and after settings are saved.
/// The small policy document lets an already-running Python sidecar see changes.
pub fn configure(offline: bool, data_dir: &Path) -> Result<(), String> {
    OFFLINE.store(offline, Ordering::SeqCst);
    *DATA_DIR.lock() = Some(data_dir.to_owned());
    std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let value = serde_json::json!({"offline_mode":offline});
    let temporary = data_dir.join("network-policy.json.tmp");
    std::fs::write(&temporary, value.to_string()).map_err(|e| e.to_string())?;
    // Windows rename cannot replace an existing target. A missing policy is
    // fail-closed in Python, so the brief replacement interval is safe.
    let destination = data_dir.join("network-policy.json");
    if destination.exists() {
        std::fs::remove_file(&destination).map_err(|e| e.to_string())?;
    }
    std::fs::rename(temporary, destination).map_err(|e| e.to_string())
}

pub fn require_online(offline: bool) -> Result<(), String> {
    if offline {
        Err(OFFLINE_ERROR.into())
    } else {
        Ok(())
    }
}

pub fn check_download() -> Result<(), String> {
    require_online(OFFLINE.load(Ordering::SeqCst))
}

pub fn allowed_host(host: &str) -> bool {
    matches!(
        host,
        "huggingface.co"
            | "github.com"
            | "release-assets.githubusercontent.com"
            | "objects.githubusercontent.com"
            | "cdn-lfs.huggingface.co"
            | "cdn-lfs-us-1.hf.co"
            | "cdn-lfs-eu-1.hf.co"
            | "cas-bridge.xethub.hf.co"
    )
}

pub fn check_download_url(url: &reqwest::Url) -> Result<(), String> {
    check_download()?;
    validate_download_url(url)
}

pub fn validate_download_url(url: &reqwest::Url) -> Result<(), String> {
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.host_str().is_some_and(allowed_host)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Downloads are restricted to the pinned GitHub and Hugging Face hosts.".into());
    }
    Ok(())
}

pub fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if let Some(previous) = attempt.previous().last() {
            record(previous.host_str().unwrap_or_default(), 0);
        }
        if attempt.previous().len() >= 10 {
            attempt.error("Too many download redirects")
        } else if let Err(error) = check_download_url(attempt.url()) {
            attempt.error(error)
        } else {
            attempt.follow()
        }
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkEntry {
    pub timestamp: String,
    pub host: String,
    pub bytes: u64,
}

fn record(host: &str, bytes: u64) {
    let Some(directory) = DATA_DIR.lock().clone() else {
        return;
    };
    // Store no URLs, query strings, tokens, filenames, or request bodies.
    if !allowed_host(host) {
        return;
    }
    let entry = NetworkEntry {
        timestamp: chrono::Utc::now().to_rfc3339(),
        host: host.into(),
        bytes,
    };
    let _guard = JOURNAL_LOCK.lock();
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(directory.join("network-journal.jsonl"))?;
        let mut line = serde_json::to_vec(&entry)?;
        line.push(b'\n');
        file.write_all(&line)
    })();
    if let Err(error) = result {
        log::warn!("Could not write network journal: {error}");
    }
}

/// Body bytes are counted as received, including partial transfers and failures.
#[derive(Debug)]
pub struct DownloadResponse {
    response: reqwest::blocking::Response,
    bytes: u64,
}

impl DownloadResponse {
    pub fn status(&self) -> reqwest::StatusCode {
        self.response.status()
    }
    pub fn content_length(&self) -> Option<u64> {
        self.response.content_length()
    }
}
impl Read for DownloadResponse {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        check_download().map_err(std::io::Error::other)?;
        let count = self.response.read(buffer)?;
        self.bytes += count as u64;
        Ok(count)
    }
}
impl Drop for DownloadResponse {
    fn drop(&mut self) {
        record(
            self.response.url().host_str().unwrap_or_default(),
            self.bytes,
        );
    }
}

pub fn send_download(
    request: reqwest::blocking::RequestBuilder,
) -> Result<DownloadResponse, String> {
    let (client, request) = request.build_split();
    let request = request.map_err(|e| e.to_string())?;
    check_download_url(request.url())?;
    let host = request.url().host_str().unwrap_or_default().to_owned();
    match client.execute(request) {
        Ok(response) => Ok(DownloadResponse { response, bytes: 0 }),
        Err(error) => {
            record(
                error.url().and_then(|url| url.host_str()).unwrap_or(&host),
                0,
            );
            Err(crate::rewrite::runtime_install::error_chain(&error))
        }
    }
}

/// Read at most the newest 2 MiB / 1,000 entries; malformed lines are ignored.
pub fn read_journal(data_dir: &Path, limit: usize) -> Result<Vec<NetworkEntry>, String> {
    let path = data_dir.join("network-journal.jsonl");
    if !path.exists() {
        return Ok(vec![]);
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    let start = length.saturating_sub(2 * 1024 * 1024);
    file.seek(SeekFrom::Start(start))
        .map_err(|e| e.to_string())?;
    let mut buffer = String::new();
    file.take(2 * 1024 * 1024)
        .read_to_string(&mut buffer)
        .map_err(|e| e.to_string())?;
    let entries = buffer
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<NetworkEntry>(line).ok())
        .filter(|entry| allowed_host(&entry.host) && entry.timestamp.len() <= 64)
        .take(limit.min(1_000))
        .collect();
    Ok(entries)
}

#[tauri::command]
pub fn get_network_journal(limit: Option<usize>) -> Result<Vec<NetworkEntry>, String> {
    read_journal(
        &crate::platform::PlatformSys::get_app_dir(),
        limit.unwrap_or(100),
    )
}
