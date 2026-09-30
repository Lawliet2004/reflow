//! `llama-server` runtime downloader.
//!
//! This module fetches a prebuilt `llama-server` binary from the official
//! `ggml-org/llama.cpp` GitHub releases, verifies its SHA-256 against a
//! pinned digest, and extracts the binary into the Reflow app-data
//! directory. The selected build is determined by the user's
//! `compute_backend` setting and the GPU presence detected on the
//! machine.
//!
//! The downloader is structured to mirror the GGUF intelligence-model
//! downloader: a re-entry lock, a background worker thread, Range
//! resume, monotonic progress events, and a clean separation between the
//! Tauri command entry point and the worker.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

use crate::capability::capabilities;
use crate::context::AppContext;
use crate::platform::PlatformSys;

/// The pinned llama.cpp build corresponding to the stable v0.5.0 release.
pub const PINNED_LLAMA_TAG: &str = "b11146";

/// Single key under which we lock the re-entry guard. There is only one
/// runtime binary, so a single key is enough.
pub const RUNTIME_LOCK_KEY: &str = "llama-runtime";

/// Optional override env var: pointing this at an existing `llama-server`
/// binary short-circuits the downloader entirely. Useful for power users
/// who want a CUDA build the auto-installer doesn't ship by default.
pub const ENV_OVERRIDE_BIN: &str = "REFLOW_LLAMA_BIN";

/// Optional override env var: pointing this at a zip/tar.gz URL skips the
/// asset lookup and downloads this archive directly. The SHA-256 is still
/// verified against `expected_sha256` passed into `install_runtime`; for
/// the override URL the caller must compute the digest themselves.
pub const ENV_OVERRIDE_URL: &str = "REFLOW_LLAMA_RUNTIME_URL";

/// The kind of archive the asset is shipped as. Drives extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ArchiveKind {
    Zip,
    TarGz,
}

impl ArchiveKind {
    #[doc(hidden)]
    pub fn from_extension(name: &str) -> Self {
        if name.ends_with(".zip") {
            ArchiveKind::Zip
        } else {
            ArchiveKind::TarGz
        }
    }
}

/// What the runtime downloader should fetch and where to find it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LlamaRuntimeSpec {
    /// The asset filename as published in the GitHub release.
    pub asset_name: String,
    /// Full HTTPS URL to the asset.
    pub url: String,
    /// Pinned SHA-256 of the asset (lowercase hex).
    pub sha256: String,
    /// What kind of archive it is.
    pub archive_kind: ArchiveKind,
    /// The binary's name inside the archive, including any directory
    /// prefix. We extract the first file whose basename matches the
    /// platform binary name (`llama-server.exe` on Windows,
    /// `llama-server` on Linux/macOS).
    pub binary_basename: String,
    /// Approximate byte size of the asset, for UI display.
    pub approx_bytes: u64,
    /// Free-form label for the kind of build (e.g. "Vulkan", "CPU").
    pub kind_label: String,
}

/// Phases emitted to the frontend.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePhase {
    Starting,
    Downloading,
    Verifying,
    Extracting,
    Complete,
    Error,
}

/// Payload pushed via the `runtime:download-progress` event.
#[derive(Debug, Clone, Serialize)]
pub struct RuntimeDownloadEvent {
    /// Pinned llama.cpp release tag, e.g. "b11146".
    pub version: String,
    /// 0..=100.
    pub progress_pct: u32,
    /// Throughput in MB/s averaged since the start of the download.
    pub speed_mbps: f32,
    pub phase: RuntimePhase,
    /// Human-readable error string, only present when `phase == Error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Final on-disk path, only present when `phase == Complete`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Approximate size, so the UI can show "~34 MB" up front.
    pub approx_bytes: u64,
    /// Free-form kind label, e.g. "Vulkan".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind_label: Option<String>,
}

/// Sidecar file recording which bundled runtime flavor currently occupies the
/// shared `llama-server` path. Older installs have no marker and are treated as
/// unknown so a GPU selection safely reconciles them once.
pub const RUNTIME_KIND_FILENAME: &str = "llama-server.kind";

pub fn runtime_kind_path() -> PathBuf {
    PlatformSys::get_app_dir()
        .join("bin")
        .join(RUNTIME_KIND_FILENAME)
}

pub fn installed_runtime_kind() -> Option<String> {
    if std::env::var(ENV_OVERRIDE_BIN).is_ok() {
        return Some("Custom".into());
    }
    std::fs::read_to_string(runtime_kind_path())
        .ok()
        .map(|kind| kind.trim().to_string())
        .filter(|kind| !kind.is_empty())
}

pub fn runtime_matches(compute_backend: &str) -> bool {
    if !llama_server_bin().is_file() {
        return false;
    }
    if std::env::var(ENV_OVERRIDE_BIN).is_ok() {
        return true;
    }
    let Some(expected) = pick_runtime_spec(compute_backend) else {
        return false;
    };
    installed_runtime_kind()
        .is_some_and(|installed| installed.eq_ignore_ascii_case(&expected.kind_label))
}

/// `true` only when we positively know the installed runtime is the wrong
/// flavor for `compute_backend`.
///
/// The distinction from [`runtime_matches`] matters: that function answers
/// "do we know this is right?", and an unlabelled install answers `false` to it
/// while being perfectly usable. Gating a launch on this predicate instead
/// means a missing marker degrades to "try it and see" — which is what the
/// GPU→CPU launch fallback already exists to handle — rather than to a hard,
/// unrecoverable refusal.
///
/// Deliberately no "backfill the missing marker" counterpart. Only the
/// installer knows which asset it unpacked; anything else would have to infer
/// the flavor from live GPU detection, and detection that momentarily reports no
/// adapter would stamp a Vulkan build as "CPU" — turning a recoverable unknown
/// into a permanent, and wrong, conflict.
pub fn runtime_flavor_conflicts(compute_backend: &str) -> bool {
    if !llama_server_bin().is_file() {
        // No binary at all is a different failure, reported by the caller's
        // own `BinaryMissing` check.
        return false;
    }
    if std::env::var(ENV_OVERRIDE_BIN).is_ok() {
        return false;
    }
    let (Some(expected), Some(installed)) =
        (pick_runtime_spec(compute_backend), installed_runtime_kind())
    else {
        // Unknown either way: let the launch decide.
        return false;
    };
    runtime_flavors_conflict(Some(&expected.kind_label), Some(&installed))
}

pub(crate) fn runtime_flavors_conflict(expected: Option<&str>, installed: Option<&str>) -> bool {
    match (expected, installed) {
        (Some(expected), Some(installed)) => !installed.eq_ignore_ascii_case(expected),
        _ => false,
    }
}

/// Resolve a [`LlamaRuntimeSpec`] for the current platform + GPU
/// presence + user preference.
///
/// The logic is:
/// * `"cpu"` → CPU asset regardless of GPU.
/// * anything else (including `"auto"`, `"gpu"`, `"vulkan"`, `"cuda"`)
///   → Vulkan asset if a GPU is detected, else CPU asset.
///
/// On unsupported platforms the function returns `None`. Callers should
/// surface a friendly error.
pub fn pick_runtime_spec(compute_backend: &str) -> Option<LlamaRuntimeSpec> {
    let requested = compute_backend.trim().to_ascii_lowercase();
    let want_gpu = requested != "cpu" && capabilities().primary_gpu().is_some();

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        if want_gpu {
            Some(LlamaRuntimeSpec::win_vulkan_x64())
        } else {
            Some(LlamaRuntimeSpec::win_cpu_x64())
        }
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        if want_gpu {
            Some(LlamaRuntimeSpec::linux_vulkan_x64())
        } else {
            Some(LlamaRuntimeSpec::linux_cpu_x64())
        }
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        if want_gpu {
            Some(LlamaRuntimeSpec::linux_vulkan_arm64())
        } else {
            Some(LlamaRuntimeSpec::linux_cpu_arm64())
        }
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Some(LlamaRuntimeSpec::macos_arm64())
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        Some(LlamaRuntimeSpec::macos_x64())
    }
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
    )))]
    {
        let _ = want_gpu;
        None
    }
}

/// Returns the path where `llama-server[.exe]` is expected to live.
/// Honors the `REFLOW_LLAMA_BIN` env override first.
pub fn llama_server_bin() -> PathBuf {
    if let Ok(p) = std::env::var(ENV_OVERRIDE_BIN) {
        let p = PathBuf::from(p);
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    let name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    PlatformSys::get_app_dir().join("bin").join(name)
}

// Per-platform asset constructors — only the current target's
// constructor is used at runtime, but every one is referenced from
// the unit tests below, so silence the dead-code warning at the
// impl-block level.
#[allow(dead_code)]
impl LlamaRuntimeSpec {
    fn build(asset_name: &str, sha256: &str, kind_label: &str, approx_bytes: u64) -> Self {
        let url = format!(
            "https://github.com/ggml-org/llama.cpp/releases/download/{}/{}",
            PINNED_LLAMA_TAG, asset_name
        );
        let archive_kind = ArchiveKind::from_extension(asset_name);
        let binary_basename = if cfg!(windows) {
            "llama-server.exe".to_string()
        } else {
            "llama-server".to_string()
        };
        Self {
            asset_name: asset_name.to_string(),
            url,
            sha256: sha256.to_string(),
            archive_kind,
            binary_basename,
            approx_bytes,
            kind_label: kind_label.to_string(),
        }
    }

    fn win_vulkan_x64() -> Self {
        Self::build(
            "llama-b11146-bin-win-vulkan-x64.zip",
            "55a378aa095b466979d85075234f66d7655c7a7483222af0c006c0e55b4d7bd6",
            "Vulkan",
            32_127_004,
        )
    }
    fn win_cpu_x64() -> Self {
        Self::build(
            "llama-b11146-bin-win-cpu-x64.zip",
            "14cf1303ca9ac3abd94816850532f9f9a69ac66fbaca3776fc6f9061c2fac1d1",
            "CPU",
            18_560_055,
        )
    }
    fn linux_vulkan_x64() -> Self {
        Self::build(
            "llama-b11146-bin-ubuntu-vulkan-x64.tar.gz",
            "d3ce40fce7403cc93bcf5718fc46c6efb61ed9709f8e5d9f10c86bf0e30e8fb3",
            "Vulkan",
            30_598_492,
        )
    }
    fn linux_cpu_x64() -> Self {
        Self::build(
            "llama-b11146-bin-ubuntu-x64.tar.gz",
            "c150306eb16b5ab696f76a8bdf810c35fd98a24e82158742e6fa28f420ff8410",
            "CPU",
            16_998_357,
        )
    }
    fn linux_vulkan_arm64() -> Self {
        Self::build(
            "llama-b11146-bin-ubuntu-vulkan-arm64.tar.gz",
            "5dcebe3ecbcb43a1ed85e3284453f9edf54dcca833e1cb1f54b4022b753c1da5",
            "Vulkan",
            24_410_274,
        )
    }
    fn linux_cpu_arm64() -> Self {
        Self::build(
            "llama-b11146-bin-ubuntu-arm64.tar.gz",
            "4aeda6fe68831547e49b7fa87607383ca5352b3d72ca5f70d52ed265f58c131f",
            "CPU",
            13_598_346,
        )
    }
    fn macos_arm64() -> Self {
        Self::build(
            "llama-b11146-bin-macos-arm64.tar.gz",
            "1ad3f9eff80edb9dbef4259ad564d1720612ef7eea48fa4afed0e54f5f3d5711",
            "CPU",
            11_189_714,
        )
    }
    fn macos_x64() -> Self {
        Self::build(
            "llama-b11146-bin-macos-x64.tar.gz",
            "305f0e3a17d2c01eb205cd0a62128357f1ec3b55329cb084d94e5ec0115d7a3b",
            "CPU",
            11_237_237,
        )
    }
}

/// Install the runtime using the supplied spec.
///
/// Returns `Ok(())` once `llama-server[.exe]` is on disk and the running
/// `FlowRuntime` has been shut down so the next `ensure()` picks it up.
pub fn install_runtime(
    app: AppHandle,
    ctx: AppContext,
    spec: LlamaRuntimeSpec,
) -> Result<(), String> {
    // Re-entry guard. Drop the lock before spawning the worker, otherwise
    // re-entry detection is meaningless.
    {
        let mut active = ctx.active_runtime_downloads.lock();
        if active.contains(RUNTIME_LOCK_KEY) {
            return Err("A llama-server runtime download is already in progress".into());
        }
        active.insert(RUNTIME_LOCK_KEY.to_string());
    }

    let app_clone = app.clone();
    let ctx_clone = ctx.clone();
    std::thread::spawn(move || {
        let outcome = run_install_worker(&app_clone, &ctx_clone, &spec);
        if let Err(err) = &outcome {
            emit_error(&app_clone, &spec, err.clone());
        }
        let mut active = ctx_clone.active_runtime_downloads.lock();
        active.remove(RUNTIME_LOCK_KEY);
        // Make sure the next inference re-launches the runtime with the
        // new binary (or, on error, gives the user a clean retry).
        ctx_clone.flow_runtime.shutdown();
    });
    Ok(())
}

/// Inspect the result of a `FlowRuntime::ensure` call. If the runtime
/// is missing OR a previous launch attempt failed, kick off an
/// auto-install using the GPU/CPU selection appropriate for the user's
/// `compute_backend` setting. Returns the original error otherwise (so
/// the caller can keep its existing error flow).
///
/// Safe to call from any Tauri command that has an `AppHandle`.
pub fn auto_install_if_missing(
    app: &AppHandle,
    ctx: &AppContext,
    compute_backend: &str,
    ensure_err: &str,
) {
    let is_recoverable = ensure_err == "llama-server is not installed"
        || ensure_err == "llama-server GPU runtime is not installed"
        || ensure_err.contains("llama-server exited before becoming ready")
        || ensure_err.contains("llama-server did not become ready")
        || ensure_err.contains("Failed to start llama-server");
    if !is_recoverable {
        return;
    }
    // The binary might be present but broken (corrupt download, wrong arch,
    // missing DLL). Re-installing over the top is safe — the installer
    // atomically renames the extracted file into place.
    if let Some(spec) = pick_runtime_spec(compute_backend) {
        // Best-effort: if a download is already in flight, ignore.
        let _ = install_runtime(app.clone(), ctx.clone(), spec);
    }
}

fn run_install_worker(
    app: &AppHandle,
    ctx: &AppContext,
    spec: &LlamaRuntimeSpec,
) -> Result<(), String> {
    // Honor the env override by short-circuiting with a friendly event.
    if let Ok(p) = std::env::var(ENV_OVERRIDE_BIN) {
        if !p.is_empty() {
            let resolved = PathBuf::from(&p);
            if !resolved.exists() {
                return Err(format!(
                    "REFLOW_LLAMA_BIN points at '{p}' but no such file exists"
                ));
            }
            emit_event(
                app,
                spec,
                RuntimePhase::Complete,
                100,
                0.0,
                None,
                Some(resolved.display().to_string()),
            );
            return Ok(());
        }
    }

    // Emit a `starting` event so the UI can switch into the "Downloading…"
    // state immediately, even before the first byte is fetched.
    emit_event(app, spec, RuntimePhase::Starting, 0, 0.0, None, None);

    let bin_dir = PlatformSys::get_app_dir().join("bin");
    if let Err(err) = std::fs::create_dir_all(&bin_dir) {
        return Err(format!("Could not create bin dir: {err}"));
    }

    let dest = llama_server_bin();
    let _ = std::fs::remove_file(&dest);

    // Stage 1: download the archive into a temp file.
    let temp_archive =
        PlatformSys::get_logs_dir().join(format!("llama-runtime-{}.partial", std::process::id()));
    let final_archive = PlatformSys::get_logs_dir().join(format!(
        "llama-runtime-{}",
        sanitize_for_filename(&spec.asset_name)
    ));

    let url = std::env::var(ENV_OVERRIDE_URL)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| spec.url.clone());

    download_with_resume(app, spec, &url, &temp_archive, &final_archive)?;

    emit_event(app, spec, RuntimePhase::Verifying, 100, 0.0, None, None);

    // Stage 2: verify SHA-256 of the downloaded archive.
    verify_sha256(&final_archive, &spec.sha256).map_err(|err| {
        // On a checksum failure, keep the partial so the next attempt can
        // either resume (Range) or re-download.
        format!("Checksum verification failed: {err}")
    })?;

    emit_event(app, spec, RuntimePhase::Extracting, 100, 0.0, None, None);

    // Stage 3: extract just the `llama-server` binary.
    let extracted_to = match spec.archive_kind {
        ArchiveKind::Zip => extract_zip(&final_archive, &bin_dir, &spec.binary_basename)?,
        ArchiveKind::TarGz => extract_tar_gz(&final_archive, &bin_dir, &spec.binary_basename)?,
    };

    // Stage 4: clean up the archive; we only need the binary.
    let _ = std::fs::remove_file(&final_archive);

    // Stage 5: make the binary executable on Unix and strip macOS
    // quarantine bits if we can.
    make_executable(&extracted_to)?;
    if cfg!(target_os = "macos") {
        let _ = std::process::Command::new("xattr")
            .arg("-d")
            .arg("com.apple.quarantine")
            .arg(&extracted_to)
            .output();
    }

    // Stage 6: record which flavor occupies the shared binary path. Write the
    // marker only after extraction and permission fixes have succeeded.
    std::fs::write(runtime_kind_path(), format!("{}\n", spec.kind_label))
        .map_err(|err| format!("Could not record runtime kind: {err}"))?;

    // Stage 7: tell the frontend we are done, and shut the runtime so
    // the next dictation picks up the new binary.
    emit_event(
        app,
        spec,
        RuntimePhase::Complete,
        100,
        0.0,
        None,
        Some(extracted_to.display().to_string()),
    );

    ctx.flow_runtime.shutdown();

    // Touch the Arc to make the borrow checker happy on shutdown.
    let _ = Arc::strong_count(&ctx.active_runtime_downloads);

    Ok(())
}

/// Render an error together with its full `source()` chain.
///
/// `reqwest`'s `Display` stops at the outermost layer, so a transport failure
/// formats as the near-useless `error sending request for url (...)` while the
/// actual reason — DNS failure, connection reset, TLS rejection — lives one or
/// two levels down in the chain. Reporting only the top layer left users with
/// an error that named the URL and nothing else.
fn error_chain(err: &dyn std::error::Error) -> String {
    let mut out = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !out.contains(&text) {
            out.push_str(": ");
            out.push_str(&text);
        }
        source = cause.source();
    }
    out
}

/// Number of attempts for the initial request. A 34 MB install failing
/// permanently because one TCP connection was reset is not acceptable.
const DOWNLOAD_ATTEMPTS: u32 = 3;

/// Issue the (possibly ranged) GET, retrying transport-level failures.
///
/// Only connect/send failures are retried. An HTTP response — even an error
/// status — is returned to the caller, which knows how to interpret it.
fn send_with_retry(
    client: &reqwest::blocking::Client,
    url: &str,
    resume_from: u64,
) -> Result<reqwest::blocking::Response, String> {
    let mut last_err = String::new();
    for attempt in 1..=DOWNLOAD_ATTEMPTS {
        let mut request = client.get(url);
        if resume_from > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
        }
        match request.send() {
            Ok(response) => return Ok(response),
            Err(err) => {
                last_err = error_chain(&err);
                log::warn!(
                    "Runtime download attempt {attempt}/{DOWNLOAD_ATTEMPTS} failed for {url}: {last_err}"
                );
                if attempt < DOWNLOAD_ATTEMPTS {
                    std::thread::sleep(std::time::Duration::from_secs(2 * attempt as u64));
                }
            }
        }
    }
    let msg = format!(
        "Download request failed after {DOWNLOAD_ATTEMPTS} attempts ({url}): {last_err}. \
         Check your internet connection, VPN, or proxy and try again."
    );
    log::error!("{msg}");
    Err(msg)
}

fn download_with_resume(
    app: &AppHandle,
    spec: &LlamaRuntimeSpec,
    url: &str,
    temp_path: &Path,
    final_path: &Path,
) -> Result<(), String> {
    let resume_from: u64 = std::fs::metadata(temp_path).map(|m| m.len()).unwrap_or(0);

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60 * 30))
        // Without this, a black-holed connection sits in the handshake until
        // the 30-minute body timeout instead of failing fast enough to retry.
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP client init failed: {e}"))?;

    let mut response = send_with_retry(&client, url, resume_from)?;

    let status = response.status();
    let already_have: u64 = if status == reqwest::StatusCode::PARTIAL_CONTENT {
        resume_from
    } else if status.is_success() {
        // Server ignored Range. Start over.
        if temp_path.exists() {
            let _ = std::fs::remove_file(temp_path);
        }
        0
    } else {
        return Err(format!("Download failed: HTTP {status}"));
    };

    let remaining = response
        .content_length()
        .unwrap_or(spec.approx_bytes.saturating_sub(already_have));
    let total = already_have + remaining;
    if total == 0 {
        return Err("Download has zero total size".into());
    }

    let mut dest_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(temp_path)
        .map_err(|e| format!("Could not open download target: {e}"))?;
    dest_file
        .seek(SeekFrom::Start(already_have))
        .map_err(|e| format!("Could not seek in download target: {e}"))?;

    // If we're resuming, emit a single progress event so the UI jumps to
    // the current position.
    if already_have > 0 {
        let pct = (already_have * 100 / total) as u32;
        emit_event(app, spec, RuntimePhase::Downloading, pct, 0.0, None, None);
    }

    let started = Instant::now();
    let mut last_emit = Instant::now();
    let mut downloaded: u64 = already_have;
    let mut buffer = vec![0u8; 64 * 1024];

    loop {
        let n = response
            .read(&mut buffer)
            .map_err(|e| format!("Read failed: {e}"))?;
        if n == 0 {
            break;
        }
        dest_file
            .write_all(&buffer[..n])
            .map_err(|e| format!("Write failed: {e}"))?;
        downloaded += n as u64;
        if last_emit.elapsed() >= std::time::Duration::from_millis(250) {
            let elapsed = started.elapsed().as_secs_f32().max(0.001);
            let speed_mbps = ((downloaded - already_have) as f32 / 1_048_576.0) / elapsed;
            let pct = (downloaded * 100 / total) as u32;
            emit_event(
                app,
                spec,
                RuntimePhase::Downloading,
                pct,
                speed_mbps,
                None,
                None,
            );
            last_emit = Instant::now();
        }
    }
    let _ = dest_file.flush();
    let _ = dest_file.sync_all();

    // Atomic-ish rename so a partial file never shows up as the "final"
    // archive on next launch.
    if final_path.exists() {
        let _ = std::fs::remove_file(final_path);
    }
    std::fs::rename(temp_path, final_path)
        .map_err(|e| format!("Could not finalize download: {e}"))?;

    Ok(())
}

pub fn verify_sha256(path: &Path, expected_hex: &str) -> Result<(), String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("Open archive: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|e| format!("Read archive: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    let digest = hasher.finalize();
    let actual = digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if !actual.eq_ignore_ascii_case(expected_hex) {
        return Err(format!(
            "expected {expected_hex}, got {actual} ({} bytes)",
            std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
        ));
    }
    Ok(())
}

/// Extract a zip archive to `dest_dir`. Returns the path of the
/// extracted launcher (i.e. `llama-server.exe` or `llama-server`).
/// Any companion DLLs in the same archive (e.g. `llama-server-impl.dll`
/// on Windows) are extracted alongside the launcher so that the
/// runtime can find them via the standard DLL search order.
fn extract_zip(archive: &Path, dest_dir: &Path, binary_basename: &str) -> Result<PathBuf, String> {
    let file = std::fs::File::open(archive).map_err(|e| format!("Open zip: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("Read zip: {e}"))?;
    let mut launcher: Option<PathBuf> = None;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("Zip entry {i}: {e}"))?;
        let entry_name = entry.name().to_string();
        let stripped = match entry_name.rsplit('/').next() {
            Some(s) => s.to_string(),
            None => continue,
        };
        if !is_runtime_payload(&stripped) {
            continue;
        }
        let out_path = dest_dir.join(&stripped);
        let mut out =
            std::fs::File::create(&out_path).map_err(|e| format!("Create extracted file: {e}"))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("Extract entry: {e}"))?;
        if stripped == binary_basename {
            launcher = Some(out_path);
        }
    }
    launcher.ok_or_else(|| format!("Archive did not contain a '{}' entry", binary_basename))
}

fn extract_tar_gz(
    archive: &Path,
    dest_dir: &Path,
    binary_basename: &str,
) -> Result<PathBuf, String> {
    let file = std::fs::File::open(archive).map_err(|e| format!("Open tar.gz: {e}"))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    let mut launcher: Option<PathBuf> = None;
    for entry in tar
        .entries()
        .map_err(|e| format!("Read tar entries: {e}"))?
    {
        let mut entry = entry.map_err(|e| format!("Tar entry: {e}"))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .map_err(|e| format!("Tar path: {e}"))?
            .into_owned();
        let file_name = match path.file_name().and_then(|f| f.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        if !is_runtime_payload(&file_name) {
            continue;
        }
        let out_path = dest_dir.join(&file_name);
        let mut out =
            std::fs::File::create(&out_path).map_err(|e| format!("Create extracted file: {e}"))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("Extract entry: {e}"))?;
        if file_name == binary_basename {
            launcher = Some(out_path);
        }
    }
    launcher.ok_or_else(|| format!("Archive did not contain a '{}' entry", binary_basename))
}

/// Decide whether an archive entry should be extracted to the runtime
/// bin dir. We extract the launcher itself, the `-impl.dll` companion
/// (Windows), and any other DLLs in the same archive. We deliberately
/// skip every other CLI tool (`llama-cli.exe`, `llama-bench.exe`,
/// `llama-quantize.exe`, ...) — they are not needed at runtime and
/// each one is several MB.
fn is_runtime_payload(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    if lower == "llama-server.exe" || lower == "llama-server" {
        return true;
    }

    // Current llama.cpp launchers are intentionally tiny and dynamically
    // load the server implementation, llama/ggml libraries, a CPU backend,
    // and (for accelerated builds) Vulkan libraries from the same directory.
    // Omitting any of those makes Windows exit with STATUS_DLL_NOT_FOUND
    // before the health endpoint can start. Unix release archives likewise
    // include shared libraries beside the launcher.
    lower.ends_with(".dll")
        || lower.ends_with(".dylib")
        || lower.ends_with(".so")
        || lower.contains(".so.")
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)
        .map_err(|e| format!("Stat binary: {e}"))?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).map_err(|e| format!("chmod: {e}"))?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn sanitize_for_filename(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn emit_event(
    app: &AppHandle,
    spec: &LlamaRuntimeSpec,
    phase: RuntimePhase,
    progress_pct: u32,
    speed_mbps: f32,
    error: Option<String>,
    path: Option<String>,
) {
    let event = RuntimeDownloadEvent {
        version: PINNED_LLAMA_TAG.to_string(),
        progress_pct,
        speed_mbps,
        phase,
        error,
        path,
        approx_bytes: spec.approx_bytes,
        kind_label: Some(spec.kind_label.clone()),
    };
    let _ = app.emit("runtime:download-progress", event);
}

fn emit_error(app: &AppHandle, spec: &LlamaRuntimeSpec, error: String) {
    let event = RuntimeDownloadEvent {
        version: PINNED_LLAMA_TAG.to_string(),
        progress_pct: 0,
        speed_mbps: 0.0,
        phase: RuntimePhase::Error,
        error: Some(error),
        path: None,
        approx_bytes: spec.approx_bytes,
        kind_label: Some(spec.kind_label.clone()),
    };
    let _ = app.emit("runtime:download-progress", event);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_lock_key_is_stable() {
        assert_eq!(RUNTIME_LOCK_KEY, "llama-runtime");
    }

    /// A download failure has to name the *reason*, not just the URL. `reqwest`
    /// hides the reason in the error's source chain, which is why the original
    /// report was an unactionable "error sending request for url (...)".
    #[test]
    fn error_chain_surfaces_the_underlying_cause() {
        #[derive(Debug)]
        struct Inner;
        impl std::fmt::Display for Inner {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "dns error: no record found")
            }
        }
        impl std::error::Error for Inner {}

        #[derive(Debug)]
        struct Outer(Inner);
        impl std::fmt::Display for Outer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(
                    f,
                    "error sending request for url (https://example.test/a.zip)"
                )
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }

        let rendered = error_chain(&Outer(Inner));
        assert!(
            rendered.contains("error sending request"),
            "outer layer missing: {rendered}"
        );
        assert!(
            rendered.contains("dns error: no record found"),
            "root cause missing: {rendered}"
        );
    }

    /// Duplicated text in a chain should not be repeated back at the user.
    #[test]
    fn error_chain_does_not_repeat_identical_layers() {
        #[derive(Debug)]
        struct Same;
        impl std::fmt::Display for Same {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "connection reset")
            }
        }
        impl std::error::Error for Same {}

        #[derive(Debug)]
        struct Wrap(Same);
        impl std::fmt::Display for Wrap {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "connection reset")
            }
        }
        impl std::error::Error for Wrap {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }

        assert_eq!(error_chain(&Wrap(Same)), "connection reset");
    }

    /// Exercises the real retry path against a port nothing listens on, so the
    /// failure is a genuine transport error rather than an HTTP status. The
    /// message must name the attempt count and carry the underlying cause.
    #[test]
    fn send_with_retry_reports_attempts_and_the_underlying_cause() {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(std::time::Duration::from_millis(250))
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .expect("client");

        // Port 1 on loopback: refused immediately, no network required.
        let err = send_with_retry(&client, "http://127.0.0.1:1/nothing.zip", 0)
            .expect_err("a refused connection must not succeed");

        assert!(
            err.contains(&DOWNLOAD_ATTEMPTS.to_string()),
            "message should say how many attempts were made: {err}"
        );
        assert!(
            err.contains("127.0.0.1:1"),
            "message should name the URL: {err}"
        );
        // The whole point of `error_chain`: something more specific than
        // reqwest's opaque outer layer has to reach the user.
        assert!(
            err.len() > "Download request failed".len() + 40,
            "message should carry a cause, not just the outer layer: {err}"
        );
    }

    #[test]
    fn archive_kind_detects_zip_vs_targz() {
        assert_eq!(
            ArchiveKind::from_extension("llama-b11146-bin-win-cpu-x64.zip"),
            ArchiveKind::Zip
        );
        assert_eq!(
            ArchiveKind::from_extension("llama-b11146-bin-ubuntu-x64.tar.gz"),
            ArchiveKind::TarGz
        );
    }

    #[test]
    fn spec_url_is_well_formed() {
        // We don't know which spec the current platform will return, but
        // every spec must point at the pinned GitHub release URL.
        let url_for = |name: &str| {
            format!(
                "https://github.com/ggml-org/llama.cpp/releases/download/{}/{}",
                PINNED_LLAMA_TAG, name
            )
        };
        assert!(url_for("a.zip").contains(PINNED_LLAMA_TAG));
        assert!(url_for("a.tar.gz").contains(PINNED_LLAMA_TAG));
    }

    #[test]
    fn sanitize_replaces_unsafe_chars() {
        assert_eq!(sanitize_for_filename("a/b c"), "a_b_c");
        assert_eq!(
            sanitize_for_filename("safe-name.tar.gz"),
            "safe-name.tar.gz"
        );
    }

    #[test]
    fn pick_runtime_spec_does_not_panic() {
        // We can't assert on GPU here, but we can assert that the
        // function returns *something* on the current target (every
        // supported target has at least a CPU spec).
        let _ = pick_runtime_spec("auto");
        let _ = pick_runtime_spec("cpu");
        let cpu_spec = pick_runtime_spec("cpu").expect("CPU spec must be available");
        assert_eq!(cpu_spec.kind_label, "CPU");
        // CPU spec must always carry a pinned SHA-256.
        assert_eq!(cpu_spec.sha256.len(), 64);
    }

    #[test]
    fn llama_server_bin_is_under_app_dir() {
        // The default path (no env override) must live under the app
        // dir's `bin/` subdirectory.
        let prev = std::env::var(ENV_OVERRIDE_BIN).ok();
        // SAFETY: this is a test; no concurrent reader.
        unsafe {
            std::env::remove_var(ENV_OVERRIDE_BIN);
        }
        let p = llama_server_bin();
        if let Some(v) = prev {
            unsafe {
                std::env::set_var(ENV_OVERRIDE_BIN, v);
            }
        }
        assert!(p.ends_with("bin") || p.to_string_lossy().contains("bin"));
    }

    #[test]
    fn runtime_payload_keeps_all_shared_library_dependencies() {
        for dependency in [
            "llama-server-impl.dll",
            "llama.dll",
            "ggml.dll",
            "ggml-vulkan.dll",
            "libomp.dll",
            "libggml.so",
            "libggml.so.1",
            "libllama.dylib",
        ] {
            assert!(is_runtime_payload(dependency), "rejected {dependency}");
        }
        assert!(is_runtime_payload("llama-server.exe"));
        assert!(is_runtime_payload("llama-server"));
        assert!(!is_runtime_payload("llama-cli.exe"));
        assert!(!is_runtime_payload("README.md"));
    }

    #[test]
    fn every_asset_constructor_produces_a_pinned_spec() {
        // Touch every per-platform constructor so each one is
        // considered "used" on every build target, and assert that
        // they all return a sane spec.
        let specs: Vec<LlamaRuntimeSpec> = vec![
            LlamaRuntimeSpec::win_vulkan_x64(),
            LlamaRuntimeSpec::win_cpu_x64(),
            LlamaRuntimeSpec::linux_vulkan_x64(),
            LlamaRuntimeSpec::linux_cpu_x64(),
            LlamaRuntimeSpec::linux_vulkan_arm64(),
            LlamaRuntimeSpec::linux_cpu_arm64(),
            LlamaRuntimeSpec::macos_arm64(),
            LlamaRuntimeSpec::macos_x64(),
        ];
        for spec in specs {
            assert!(spec.url.contains(PINNED_LLAMA_TAG));
            assert_eq!(spec.sha256.len(), 64);
            assert!(!spec.binary_basename.is_empty());
            assert!(!spec.kind_label.is_empty());
            assert!(spec.approx_bytes > 0);
        }
    }
}
