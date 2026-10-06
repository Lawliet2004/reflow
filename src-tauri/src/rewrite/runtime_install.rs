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

#[cfg(test)]
#[path = "runtime_restore_tests.rs"]
mod restore_tests;

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
    if std::env::var(ENV_OVERRIDE_BIN).is_ok_and(|path| !path.is_empty()) {
        return Some("Custom".into());
    }
    if let Some(manifest) =
        super::runtime_inventory::active_manifest(&PlatformSys::get_app_dir().join("bin"))
    {
        return (!manifest.kind.is_empty() && manifest.kind != "Unknown").then_some(manifest.kind);
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
    if std::env::var(ENV_OVERRIDE_BIN).is_ok_and(|path| !path.is_empty()) {
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
    if std::env::var(ENV_OVERRIDE_BIN).is_ok_and(|path| !path.is_empty()) {
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
    let root = PlatformSys::get_app_dir().join("bin");
    if let Some(binary) = super::runtime_inventory::active_binary(&root) {
        return binary;
    }
    let name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    root.join(name)
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
    crate::network_policy::require_online(ctx.settings_store.get().offline_mode)?;
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
    // missing DLL). Keep the installed runtime until the archive is verified,
    // and stop its process before replacing its executable and shared libraries.
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

    // A new immutable generation is extracted, hashed, and smoke-tested while
    // the current runtime continues serving. No live DLL is overwritten.
    let generation = stage_archive(&final_archive, &bin_dir, spec)?;
    let extracted_to = super::runtime_inventory::entry_binary(&bin_dir, &generation)?;
    promote_for_context(ctx, || {
        super::runtime_inventory::promote(&bin_dir, &generation.id)
    })?;

    // Stage 4: clean up the archive; we only need the binary.
    let _ = std::fs::remove_file(&final_archive);

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
pub(crate) fn error_chain(err: &dyn std::error::Error) -> String {
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
) -> Result<crate::network_policy::DownloadResponse, String> {
    let parsed_url = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
    crate::network_policy::check_download_url(&parsed_url)?;
    let mut last_err = String::new();
    for attempt in 1..=DOWNLOAD_ATTEMPTS {
        let mut request = client.get(url);
        if resume_from > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
        }
        match crate::network_policy::send_download(request) {
            Ok(response) => return Ok(response),
            Err(err) => {
                last_err = err;
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
    crate::network_policy::check_download()?;
    let resume_from: u64 = std::fs::metadata(temp_path).map(|m| m.len()).unwrap_or(0);

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60 * 30))
        // Without this, a black-holed connection sits in the handshake until
        // the 30-minute body timeout instead of failing fast enough to retry.
        .connect_timeout(std::time::Duration::from_secs(30))
        .redirect(crate::network_policy::redirect_policy())
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

fn digest_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn smoke_command(
    mut command: std::process::Command,
    timeout: std::time::Duration,
) -> Result<(), String> {
    use std::process::Stdio;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let deadline = Instant::now() + timeout;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Runtime smoke check failed to start: {e}"))?;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!("Runtime --version exited with {status}; missing or incompatible dependencies"))
                }
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Runtime --version exceeded its smoke-check deadline".into());
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Runtime smoke-check wait failed: {error}"));
            }
        }
    }
}

fn smoke_version(binary: &Path) -> Result<(), String> {
    let mut command = std::process::Command::new(binary);
    command.arg("--version");
    if let Some(dir) = binary.parent() {
        command.current_dir(dir);
    }
    smoke_command(command, std::time::Duration::from_secs(10))
}

fn stage_archive(
    archive: &Path,
    root: &Path,
    spec: &LlamaRuntimeSpec,
) -> Result<super::runtime_inventory::RuntimeManifest, String> {
    use super::runtime_inventory::{RuntimeFile, RuntimeManifest};
    // This helper also verifies integrity when exercised outside the downloader.
    verify_sha256(archive, &spec.sha256)?;
    let generations = root.join("runtimes");
    std::fs::create_dir_all(&generations).map_err(|e| e.to_string())?;
    let suffix = uuid::Uuid::new_v4();
    let id = format!(
        "{}-{}-{suffix}",
        PINNED_LLAMA_TAG,
        sanitize_for_filename(&spec.kind_label)
    );
    let staging = generations.join(format!(".staging-{suffix}"));
    std::fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let outcome = (|| {
        let binary = match spec.archive_kind {
            ArchiveKind::Zip => extract_zip(archive, &staging, &spec.binary_basename)?,
            ArchiveKind::TarGz => extract_tar_gz(archive, &staging, &spec.binary_basename)?,
        };
        make_executable(&binary)?;
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("xattr")
                .args(["-d", "com.apple.quarantine"])
                .arg(&binary)
                .output();
        }
        smoke_version(&binary)?;
        let mut files = vec![];
        for entry in std::fs::read_dir(&staging).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                return Err("Runtime generation contains a non-file payload".into());
            }
            let filename = entry
                .file_name()
                .into_string()
                .map_err(|_| "Runtime filename is not UTF-8")?;
            super::runtime_inventory::safe_name(&filename)?;
            files.push(RuntimeFile {
                filename,
                sha256: digest_file(&entry.path())?,
                bytes: entry.metadata().map_err(|e| e.to_string())?.len(),
            });
        }
        files.sort_by(|a, b| a.filename.cmp(&b.filename));
        let manifest = RuntimeManifest {
            id: id.clone(),
            version: PINNED_LLAMA_TAG.into(),
            kind: spec.kind_label.clone(),
            asset: spec.asset_name.clone(),
            archive_sha256: spec.sha256.clone(),
            binary: spec.binary_basename.clone(),
            files,
        };
        std::fs::write(
            staging.join("runtime-manifest.json"),
            serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(&staging, generations.join(&id))
            .map_err(|e| format!("Could not finalize runtime generation: {e}"))?;
        Ok(manifest)
    })();
    if outcome.is_err() {
        let _ = std::fs::remove_dir_all(staging);
    }
    outcome
}

fn promote_for_context(
    ctx: &AppContext,
    promote: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let _session = ctx
        .session_operation
        .try_lock()
        .map_err(|_| "Wait for the active dictation before activating a runtime.".to_string())?;
    if !matches!(
        *ctx.state_enum.read(),
        crate::state::AppStateEnum::Ready | crate::state::AppStateEnum::Idle
    ) {
        return Err("Runtime is staged; finish the active dictation before activating it.".into());
    }
    let native = ctx.asr_runtime.lock().as_str() == "native";
    promote_with_asr_restore(
        &ctx.asr_handle,
        native,
        || ctx.flow_runtime.shutdown(),
        promote,
    )
}

fn promote_with_asr_restore(
    asr: &crate::asr::AsrHandle,
    native: bool,
    stop_flow: impl FnOnce(),
    promote: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let restore = if native {
        asr.loaded_model_request_blocking()?
    } else {
        None
    };
    let maintenance = super::runtime_inventory::Maintenance::begin()?;
    // New ensure calls fail while pending, so unloading the actor cannot wait
    // behind a launch blocked on the promotion's exclusive lock.
    stop_flow();
    if native {
        asr.unload_model_blocking()?;
    }
    let outcome = {
        let _exclusive = maintenance.exclusive();
        promote()
    };
    // A native model uses FlowRuntime::ensure: reopening its server while
    // maintenance is still pending would reject the restoration itself.
    drop(maintenance);
    if let Some(request) = restore {
        if let Err(error) = asr.load_model_with_precision_blocking(
            &request.model_dir,
            &request.backend,
            &request.precision,
        ) {
            let restore_error =
                format!("Native ASR could not be restored after runtime activation: {error}");
            return Err(match outcome {
                Ok(()) => restore_error,
                Err(promotion_error) => format!("{promotion_error}; {restore_error}"),
            });
        }
    }
    outcome
}

pub fn get_runtime_inventory() -> Vec<super::runtime_inventory::RuntimeEntry> {
    let root = PlatformSys::get_app_dir().join("bin");
    let mut entries = super::runtime_inventory::inventory(&root);
    if let Ok(binary) = std::env::var(ENV_OVERRIDE_BIN) {
        if !binary.is_empty() {
            for entry in &mut entries {
                entry.active = false;
                entry.rollback_available = false;
            }
            let healthy = Path::new(&binary).is_file();
            entries.push(super::runtime_inventory::RuntimeEntry {
                id: "custom".into(),
                version: "external".into(),
                kind: "Custom".into(),
                binary_path: binary,
                active: true,
                rollback_available: false,
                healthy,
                error: (!healthy).then(|| "Custom runtime launcher is missing".into()),
            });
        }
    }
    entries
}

pub fn rollback_runtime(
    ctx: &AppContext,
) -> Result<Vec<super::runtime_inventory::RuntimeEntry>, String> {
    if std::env::var(ENV_OVERRIDE_BIN).is_ok_and(|path| !path.is_empty()) {
        return Err("Remove REFLOW_LLAMA_BIN before rolling back the managed runtime.".into());
    }
    let root = PlatformSys::get_app_dir().join("bin");
    let expected_pointer = super::runtime_inventory::read_pointer(&root)?;
    let previous = super::runtime_inventory::rollback_target(&root)?;
    smoke_version(&super::runtime_inventory::entry_binary(&root, &previous)?)?;
    promote_for_context(ctx, || {
        if super::runtime_inventory::read_pointer(&root)? != expected_pointer {
            return Err(
                "Runtime inventory changed during rollback verification; retry rollback.".into(),
            );
        }
        super::runtime_inventory::rollback(&root)
    })?;
    Ok(get_runtime_inventory())
}

pub fn repair_runtime(app: AppHandle, ctx: AppContext) -> Result<(), String> {
    crate::network_policy::require_online(ctx.settings_store.get().offline_mode)?;
    if std::env::var(ENV_OVERRIDE_BIN).is_ok_and(|path| !path.is_empty()) {
        return Err("Remove REFLOW_LLAMA_BIN before repairing the managed runtime.".into());
    }
    let backend = ctx.settings_store.get().refinement.device;
    let spec =
        pick_runtime_spec(&backend).ok_or("No runtime package is available for this platform")?;
    {
        let mut active = ctx.active_runtime_downloads.lock();
        if !active.insert(RUNTIME_LOCK_KEY.to_string()) {
            return Err("A runtime download is already in progress".into());
        }
    }
    let outcome = run_install_worker(&app, &ctx, &spec);
    if let Err(error) = &outcome {
        emit_error(&app, &spec, error.clone());
    }
    ctx.active_runtime_downloads.lock().remove(RUNTIME_LOCK_KEY);
    outcome
}

/// Extract a zip archive to `dest_dir`. Returns the path of the
/// extracted launcher (i.e. `llama-server.exe` or `llama-server`).
/// Any companion DLLs in the same archive (e.g. `llama-server-impl.dll`
/// on Windows) are extracted alongside the launcher so that the
/// runtime can find them via the standard DLL search order.
fn extract_zip(archive: &Path, dest_dir: &Path, binary_basename: &str) -> Result<PathBuf, String> {
    let file = std::fs::File::open(archive).map_err(|e| format!("Open zip: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("Read zip: {e}"))?;
    let mut payload_names = std::collections::HashSet::new();
    let mut has_launcher = false;
    for index in 0..zip.len() {
        let entry = zip
            .by_index(index)
            .map_err(|e| format!("Zip entry {index}: {e}"))?;
        let name = entry.name().rsplit('/').next().unwrap_or_default();
        if !is_runtime_payload(name) {
            continue;
        }
        super::runtime_inventory::safe_name(name)?;
        if entry.is_dir() || !payload_names.insert(name.to_ascii_lowercase()) {
            return Err(format!("Ambiguous runtime archive payload: {name}"));
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(format!("Runtime ZIP payload cannot be a symlink: {name}"));
        }
        has_launcher |= name == binary_basename;
    }
    if !has_launcher {
        return Err(format!(
            "Archive did not contain a '{binary_basename}' entry"
        ));
    }
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
        super::runtime_inventory::safe_name(&stripped)?;
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
    let mut aliases = Vec::new();
    let mut payload_names = std::collections::HashSet::new();
    for entry in tar
        .entries()
        .map_err(|e| format!("Read tar entries: {e}"))?
    {
        let mut entry = entry.map_err(|e| format!("Tar entry: {e}"))?;
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
        super::runtime_inventory::safe_name(&file_name)?;
        if !payload_names.insert(file_name.to_ascii_lowercase()) {
            return Err(format!("Ambiguous runtime archive payload: {file_name}"));
        }
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            let target = entry
                .link_name()
                .map_err(|e| e.to_string())?
                .ok_or("Runtime library alias has no target")?;
            if target.components().any(|component| {
                !matches!(
                    component,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )
            }) {
                return Err("Runtime library alias points outside its archive".into());
            }
            let target = target
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("Runtime library alias target is invalid")?
                .to_string();
            super::runtime_inventory::safe_name(&target)?;
            if !is_runtime_payload(&target) {
                return Err("Runtime alias targets a non-runtime payload".into());
            }
            aliases.push((file_name, target));
            continue;
        }
        if !kind.is_file() {
            return Err(format!(
                "Runtime payload is not a regular file: {file_name}"
            ));
        }
        let out_path = dest_dir.join(&file_name);
        let mut out =
            std::fs::File::create(&out_path).map_err(|e| format!("Create extracted file: {e}"))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("Extract entry: {e}"))?;
        if file_name == binary_basename {
            launcher = Some(out_path);
        }
    }
    // Materialize shared-library aliases as verified file copies. This works
    // without Windows symlink privileges and retains Unix SONAME filenames.
    while !aliases.is_empty() {
        let before = aliases.len();
        let mut unresolved = Vec::new();
        for (name, target) in aliases {
            let source = dest_dir.join(&target);
            if !source.is_file() {
                unresolved.push((name, target));
                continue;
            }
            let destination = dest_dir.join(&name);
            std::fs::copy(&source, &destination)
                .map_err(|e| format!("Runtime alias copy failed: {e}"))?;
            if name == binary_basename {
                launcher = Some(destination);
            }
        }
        if unresolved.len() == before {
            return Err("Runtime aliases have missing or cyclic targets".into());
        }
        aliases = unresolved;
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
    fn unusable_staged_launcher_preserves_the_active_pointer_and_cleans_staging() {
        let root =
            std::env::temp_dir().join(format!("reflow-runtime-stage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(super::super::runtime_inventory::binary_name()),
            b"working incumbent",
        )
        .unwrap();
        super::super::runtime_inventory::write_pointer(
            &root,
            &super::super::runtime_inventory::RuntimePointer {
                current: "legacy".into(),
                previous: None,
            },
        )
        .unwrap();
        let pointer = std::fs::read(root.join("runtime-active.json")).unwrap();
        let archive = root.join("invalid-launcher.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        let binary = super::super::runtime_inventory::binary_name();
        zip.start_file(binary, zip::write::FileOptions::default())
            .unwrap();
        zip.write_all(b"not an executable").unwrap();
        zip.finish().unwrap();
        let spec = LlamaRuntimeSpec {
            asset_name: "fixture.zip".into(),
            url: String::new(),
            sha256: digest_file(&archive).unwrap(),
            archive_kind: ArchiveKind::Zip,
            binary_basename: binary.into(),
            approx_bytes: 1,
            kind_label: "CPU".into(),
        };
        let result = stage_archive(&archive, &root, &spec);
        let unchanged = std::fs::read(root.join("runtime-active.json")).unwrap() == pointer;
        let remaining = std::fs::read_dir(root.join("runtimes")).unwrap().count();
        let incumbent = std::fs::read(root.join(binary)).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert!(result.is_err());
        assert!(unchanged);
        assert_eq!(remaining, 0);
        assert_eq!(incumbent, b"working incumbent");
    }

    #[test]
    fn runtime_smoke_check_reaps_a_hung_process() {
        #[cfg(windows)]
        let command = {
            let mut command = std::process::Command::new("powershell");
            command.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"]);
            command
        };
        #[cfg(not(windows))]
        let command = {
            let mut command = std::process::Command::new("sh");
            command.args(["-c", "exec sleep 60"]);
            command
        };
        let started = Instant::now();
        let result = smoke_command(command, std::time::Duration::from_millis(100));
        assert!(result.unwrap_err().contains("deadline"));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "smoke fixture must not survive its timeout"
        );
    }

    #[test]
    fn tar_shared_library_alias_is_retained_as_a_runtime_dependency() {
        let root =
            std::env::temp_dir().join(format!("reflow-runtime-tar-{}", uuid::Uuid::new_v4()));
        let dest = root.join("bin");
        std::fs::create_dir_all(&dest).unwrap();
        let archive = root.join("runtime.tar.gz");
        let gzip = flate2::write::GzEncoder::new(
            std::fs::File::create(&archive).unwrap(),
            flate2::Compression::default(),
        );
        let mut tar = tar::Builder::new(gzip);
        for (name, bytes) in [
            ("llama-server", b"launcher".as_slice()),
            ("libggml.so.1.0", b"library".as_slice()),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o755);
            header.set_size(bytes.len() as u64);
            header.set_cksum();
            tar.append_data(&mut header, name, bytes).unwrap();
        }
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_link(&mut header, "libggml.so.1", "libggml.so.1.0")
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        let result = extract_tar_gz(&archive, &dest, "llama-server");
        let alias = std::fs::read(dest.join("libggml.so.1"));
        std::fs::remove_dir_all(root).unwrap();
        assert!(result.is_ok());
        assert_eq!(
            alias.unwrap(),
            b"library",
            "versioned library aliases are required by the dynamic loader"
        );
    }

    #[test]
    fn zip_payload_name_cannot_escape_the_staging_directory() {
        let root =
            std::env::temp_dir().join(format!("reflow-runtime-zip-path-{}", uuid::Uuid::new_v4()));
        let dest = root.join("bin");
        std::fs::create_dir_all(&dest).unwrap();
        let archive = root.join("unsafe.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        for name in ["llama-server.exe", "..\\escaped.dll"] {
            zip.start_file(name, zip::write::FileOptions::default())
                .unwrap();
            zip.write_all(b"fixture").unwrap();
        }
        zip.finish().unwrap();
        let result = extract_zip(&archive, &dest, "llama-server.exe");
        let escaped = root.join("escaped.dll").exists();
        std::fs::remove_dir_all(root).unwrap();
        assert!(
            result.is_err(),
            "unsafe flattened archive filename must be rejected"
        );
        assert!(!escaped);
    }

    #[test]
    fn incomplete_archive_does_not_overwrite_an_existing_runtime_dependency() {
        let root =
            std::env::temp_dir().join(format!("reflow-runtime-red-{}", uuid::Uuid::new_v4()));
        let dest = root.join("bin");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("ggml.dll"), b"working old runtime").unwrap();
        let archive = root.join("broken.zip");
        let file = std::fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("ggml.dll", zip::write::FileOptions::default())
            .unwrap();
        zip.write_all(b"incomplete replacement").unwrap();
        zip.finish().unwrap();
        let result = extract_zip(&archive, &dest, "llama-server.exe");
        let preserved = std::fs::read(dest.join("ggml.dll")).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert!(result.is_err());
        assert_eq!(
            preserved, b"working old runtime",
            "failed extraction destroyed a dependency of the active runtime"
        );
    }

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
    fn send_with_retry_preserves_transport_causes_and_rejects_unpinned_requests() {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(std::time::Duration::from_millis(250))
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .expect("client");
        // A local refused connection is safe and exercises the cause chain.
        let error = client
            .get("http://127.0.0.1:1/nothing.zip")
            .send()
            .unwrap_err();
        let cause = error_chain(&error);
        assert!(cause.contains("127.0.0.1:1"));
        assert!(cause.len() > error.to_string().len());
        // Production retries must fail immediately for a blocked request.
        let error = send_with_retry(&client, "http://127.0.0.1:1/nothing.zip", 0).unwrap_err();
        assert!(error == crate::network_policy::OFFLINE_ERROR || error.contains("restricted"));
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
