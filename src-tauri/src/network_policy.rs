//! The only outbound network boundary: explicit, pinned artifact downloads.
//! Local ASR/LLM HTTP never passes through this module.
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

mod download;
pub use download::{fetch_verified, fetch_verified_blocking, ArtifactDownload, TransferProgress};

const ONLINE: u8 = 0;
const OFFLINE: u8 = 1;
const POLICY_SAVE_FAILED: u8 = 2;
static DOWNLOAD_POLICY: AtomicU8 = AtomicU8::new(OFFLINE);
static DATA_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
static JOURNAL_LOCK: Mutex<()> = Mutex::new(());
static CONFIGURE_LOCK: Mutex<()> = Mutex::new(());
pub const OFFLINE_ERROR: &str = "Offline mode is on";
const POLICY_SAVE_ERROR: &str = "Downloads are unavailable because Reflow could not save its network policy. Check that Reflow's data folder is writable, then retry saving the download setting.";

/// Call at startup, before any model load, and after settings are saved.
/// The small policy document lets an already-running Python sidecar see changes.
pub fn configure(offline: bool, data_dir: &Path) -> Result<(), String> {
    let _configuration = CONFIGURE_LOCK.lock();
    *DATA_DIR.lock() = Some(data_dir.to_owned());
    apply_policy(offline, data_dir, &DOWNLOAD_POLICY)
}

fn apply_policy(offline: bool, data_dir: &Path, gate: &AtomicU8) -> Result<(), String> {
    // Revoke immediately; grant only after the sidecar policy was saved.
    // A failed online transition must never enable Rust downloads.
    if offline {
        gate.store(OFFLINE, Ordering::SeqCst);
    }
    let result = std::fs::create_dir_all(data_dir)
        .map_err(|e| e.to_string())
        .and_then(|()| {
            write_policy(
                data_dir,
                &serde_json::json!({"offline_mode":offline}).to_string(),
            )
        });
    gate.store(
        if result.is_err() {
            POLICY_SAVE_FAILED
        } else if offline {
            OFFLINE
        } else {
            ONLINE
        },
        Ordering::SeqCst,
    );
    result
}

fn write_policy(data_dir: &Path, value: &str) -> Result<(), String> {
    crate::platform::persistence::write_atomic(
        &data_dir.join("network-policy.json"),
        value.as_bytes(),
    )
    .map_err(|e| e.to_string())
}

pub fn require_online(offline: bool) -> Result<(), String> {
    if offline {
        Err(OFFLINE_ERROR.into())
    } else {
        Ok(())
    }
}

pub fn check_download() -> Result<(), String> {
    check_gate(&DOWNLOAD_POLICY)
}

fn check_gate(gate: &AtomicU8) -> Result<(), String> {
    // Admission and its diagnostic are one atomic snapshot, so concurrent
    // checks cannot confuse a failed policy save with the user's preference.
    match gate.load(Ordering::SeqCst) {
        ONLINE => Ok(()),
        OFFLINE => Err(OFFLINE_ERROR.into()),
        _ => Err(POLICY_SAVE_ERROR.into()),
    }
}

/// Fail fast when the filesystem holding `dir` cannot fit the download plus a
/// 10% margin. Best effort: when no mounted disk matches the path, allow the
/// download and let the eventual write error surface instead.
pub fn check_disk_space(dir: &Path, expected_bytes: u64) -> Result<(), String> {
    let required = expected_bytes.saturating_add(expected_bytes / 10);
    // Unix paths can cross filesystems through a symlink. Windows canonical
    // paths use a different drive-prefix form; retain its existing path lookup.
    #[cfg(unix)]
    let directory = dir.canonicalize().unwrap_or_else(|_| dir.to_owned());
    #[cfg(not(unix))]
    let directory = dir.to_owned();
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let Some(disk) = disks
        .iter()
        .filter(|disk| directory.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().as_os_str().len())
    else {
        return Ok(());
    };
    let free = disk.available_space();
    if free < required {
        return Err(format!(
            "Not enough disk space: need {:.1} GB, {:.1} GB free",
            required as f64 / 1e9,
            free as f64 / 1e9
        ));
    }
    Ok(())
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
            | "us.aws.cdn.hf.co"
            | "us.gcp.cdn.hf.co"
            | "pypi.org"
            | "files.pythonhosted.org"
            | "download.pytorch.org"
            | "download-r2.pytorch.org"
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
    pub fn headers(&self) -> &reqwest::header::HeaderMap {
        self.response.headers()
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

#[cfg(test)]
mod policy_tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn disk_space_uses_the_actual_filesystem_for_temporary_and_symlinked_paths() {
        use std::os::unix::{ffi::OsStrExt, fs::symlink};
        let root = std::env::temp_dir().join(format!("reflow-disk-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let linked = root.join("linked-storage");
        symlink(std::env::current_dir().unwrap(), &linked).unwrap();
        for path in [&root, &linked] {
            let native = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
            // SAFETY: statvfs contains only C scalar fields; the path is a
            // terminated string and its output points to writable storage.
            let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
            assert_eq!(unsafe { libc::statvfs(native.as_ptr(), &mut stat) }, 0);
            let available = (u128::from(stat.f_bavail) * u128::from(stat.f_frsize))
                .min(u128::from(u64::MAX)) as u64;
            assert!(
                check_disk_space(path, available.saturating_add(available / 5).max(1)).is_err(),
                "{path:?}"
            );
            assert!(check_disk_space(path, available / 2).is_ok(), "{path:?}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_policy_persistence_keeps_the_rust_download_gate_closed() {
        let root = std::env::temp_dir().join(format!("reflow-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let gate = AtomicU8::new(ONLINE);
        let blocked = root.join("not-a-directory");
        std::fs::write(&blocked, b"incumbent").unwrap();
        for offline in [false, true] {
            gate.store(ONLINE, Ordering::SeqCst);
            assert!(apply_policy(offline, &blocked, &gate).is_err());
            let error = check_gate(&gate).unwrap_err();
            assert_eq!(error, POLICY_SAVE_ERROR);
            assert!(error.contains("policy"), "{error}");
            assert!(!error.contains(OFFLINE_ERROR), "{error}");
            assert_eq!(std::fs::read(&blocked).unwrap(), b"incumbent");
        }
        let destination = root.join("network-policy.json");
        std::fs::create_dir(&destination).unwrap();
        gate.store(ONLINE, Ordering::SeqCst);
        assert!(apply_policy(false, &root, &gate).is_err());
        assert_eq!(check_gate(&gate).unwrap_err(), POLICY_SAVE_ERROR);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        std::fs::remove_dir(destination).unwrap();
        for offline in [true, false, true] {
            apply_policy(offline, &root, &gate).unwrap();
            assert_eq!(
                check_gate(&gate),
                if offline {
                    Err(OFFLINE_ERROR.into())
                } else {
                    Ok(())
                }
            );
            let policy: serde_json::Value =
                serde_json::from_slice(&std::fs::read(root.join("network-policy.json")).unwrap())
                    .unwrap();
            assert_eq!(policy["offline_mode"], offline);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn policy_replacement_keeps_one_complete_document_visible_to_readers() {
        let root = std::env::temp_dir().join(format!("reflow-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        write_policy(&root, r#"{"offline_mode":true}"#).unwrap();
        let reader_root = root.clone();
        let reader = std::thread::spawn(move || {
            for _ in 0..1000 {
                let bytes = std::fs::read(reader_root.join("network-policy.json")).unwrap();
                let policy: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert!(policy["offline_mode"].is_boolean());
            }
        });
        for i in 0..100 {
            write_policy(
                &root,
                &serde_json::json!({"offline_mode":i % 2 == 0}).to_string(),
            )
            .unwrap();
        }
        reader.join().unwrap();
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
