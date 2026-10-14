//! One-click managed speech-runtime setup.
//!
//! Downloads a pinned python-build-standalone interpreter, creates a venv in
//! the app data directory (`<app_dir>/python-runtime`, which
//! [`super::sidecar::Qwen3AsrSidecar::find_python`] already probes first), and
//! installs the speech dependencies into it. All heavy work happens on a
//! background thread; progress is streamed to the UI via
//! `python-runtime:setup-progress` events.

use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::context::AppContext;
use crate::platform::PlatformSys;

/// Pinned python-build-standalone release (>= 7 days old when chosen).
/// CPython 3.12.15, `install_only` tar.gz — plain gzip so no zstd dependency.
const PBS_TAG: &str = "20261003";
const PBS_PYTHON_VERSION: &str = "3.12.15";

/// Disk estimate for the full managed environment. CUDA torch wheels are
/// multiple GB; CPU-only torch plus the pinned requirements is ~2 GB.
const DISK_ESTIMATE_CUDA: u64 = 7_000_000_000;
const DISK_ESTIMATE_CPU: u64 = 2_000_000_000;
/// Interpreter + sherpa-onnx only (x86_64 macOS — no usable torch wheels).
const DISK_ESTIMATE_NO_TORCH: u64 = 800_000_000;

const RUNTIME_DIR_NAME: &str = "python-runtime";
const BASE_DIR_NAME: &str = "python-base";
const RECEIPT_NAME: &str = "reflow-runtime.json";
const PROGRESS_EVENT: &str = "python-runtime:setup-progress";
/// Internal sentinel — a cancelled run reports `cancelled`, not `error`.
const CANCELLED: &str = "python runtime setup cancelled";

pub struct PythonBuildPin {
    pub target: &'static str,
    pub asset: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub approx_bytes: u64,
}

const PINS: &[PythonBuildPin] = &[
    PythonBuildPin {
        target: "x86_64-unknown-linux-gnu",
        asset: "cpython-3.12.15+20261003-x86_64-unknown-linux-gnu-install_only.tar.gz",
        url: "https://github.com/astral-sh/python-build-standalone/releases/download/20261003/cpython-3.12.15%2B20261003-x86_64-unknown-linux-gnu-install_only.tar.gz",
        sha256: "f937814031eab4698ca6d07ec606ede1825768f3f3e99af76d9db3900bee03c5",
        approx_bytes: 66_924_371,
    },
    PythonBuildPin {
        target: "aarch64-unknown-linux-gnu",
        asset: "cpython-3.12.15+20261003-aarch64-unknown-linux-gnu-install_only.tar.gz",
        url: "https://github.com/astral-sh/python-build-standalone/releases/download/20261003/cpython-3.12.15%2B20261003-aarch64-unknown-linux-gnu-install_only.tar.gz",
        sha256: "95c01982c9fcb9d95b0acfdb5eb8a6e0099dd11edf062314a474228d93f2b765",
        approx_bytes: 52_330_698,
    },
    PythonBuildPin {
        target: "x86_64-apple-darwin",
        asset: "cpython-3.12.15+20261003-x86_64-apple-darwin-install_only.tar.gz",
        url: "https://github.com/astral-sh/python-build-standalone/releases/download/20261003/cpython-3.12.15%2B20261003-x86_64-apple-darwin-install_only.tar.gz",
        sha256: "a8fd7a91852f19b6d959793ef41fad048631ccb2a334a9ecdf573255298f7978",
        approx_bytes: 24_850_741,
    },
    PythonBuildPin {
        target: "aarch64-apple-darwin",
        asset: "cpython-3.12.15+20261003-aarch64-apple-darwin-install_only.tar.gz",
        url: "https://github.com/astral-sh/python-build-standalone/releases/download/20261003/cpython-3.12.15%2B20261003-aarch64-apple-darwin-install_only.tar.gz",
        sha256: "316a463172740e71d8dca1f2730784e325f3f720941137b5d674d5801a632213",
        approx_bytes: 25_146_462,
    },
    PythonBuildPin {
        target: "x86_64-pc-windows-msvc",
        asset: "cpython-3.12.15+20261003-x86_64-pc-windows-msvc-install_only.tar.gz",
        url: "https://github.com/astral-sh/python-build-standalone/releases/download/20261003/cpython-3.12.15%2B20261003-x86_64-pc-windows-msvc-install_only.tar.gz",
        sha256: "4b6f0beebbb695a0f3ea237b8c3eaa5bd424f47a7bc25b2fbe3a43390c770f08",
        approx_bytes: 46_509_797,
    },
];

pub(crate) fn pin_for(target: &str) -> Option<&'static PythonBuildPin> {
    PINS.iter().find(|pin| pin.target == target)
}

pub(crate) fn target_triple() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return "x86_64-unknown-linux-gnu";
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        return "aarch64-unknown-linux-gnu";
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return "x86_64-apple-darwin";
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return "aarch64-apple-darwin";
    }
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        return "x86_64-pc-windows-msvc";
    }
    #[allow(unreachable_code)]
    "unsupported"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TorchVariant {
    /// torch wheels from the CUDA pytorch index (NVIDIA on Linux/Windows).
    Cuda,
    /// torch wheels from the CPU pytorch index (no NVIDIA, Linux/Windows —
    /// far smaller download and install than the default CUDA-enabled PyPI build).
    Cpu,
    /// Whatever PyPI serves (macOS — torch has no CUDA build there anyway).
    Pypi,
    /// No torch at all — torch stopped shipping Intel-mac wheels, so x86_64
    /// macOS only gets the sherpa-onnx (Zipformer) speech path.
    None_,
}

impl TorchVariant {
    fn index_url(&self) -> Option<&'static str> {
        match self {
            Self::Cuda => Some("https://download.pytorch.org/whl/cu130"),
            Self::Cpu => Some("https://download.pytorch.org/whl/cpu"),
            Self::Pypi | Self::None_ => None,
        }
    }
    fn receipt_name(&self) -> &'static str {
        match self {
            Self::Cuda => "cu130",
            Self::Cpu => "cpu",
            Self::Pypi => "default",
            Self::None_ => "none",
        }
    }
}

/// The torch wheel flavor, or `None_` when the pinned torch has no wheel for
/// the platform at all. x86_64 macOS is the latter: torch publishes no Intel
/// mac wheels since it dropped that tier, and the CUDA/CPU pytorch indexes
/// are Linux/Windows-only anyway.
pub(crate) fn torch_variant(target: &str, nvidia_present: bool) -> TorchVariant {
    match target {
        "x86_64-apple-darwin" => TorchVariant::None_,
        _ if target.contains("apple-darwin") => TorchVariant::Pypi,
        _ if nvidia_present && (target.contains("linux") || target.contains("windows")) => {
            TorchVariant::Cuda
        }
        _ => TorchVariant::Cpu,
    }
}

/// Extract `X.Y.Z` from a `name==X.Y.Z` pin line in requirements.txt. The
/// managed environment installs torch from a platform index first, so the
/// `-r` install later treats it as already satisfied.
pub(crate) fn pinned_version(requirements: &str, package: &str) -> Option<String> {
    let prefix = format!("{package}==");
    requirements.lines().map(str::trim).find_map(|line| {
        let rest = line.strip_prefix(&prefix)?;
        Some(
            rest.split_whitespace()
                .next()?
                .trim_end_matches(';')
                .to_string(),
        )
    })
}

pub(crate) fn torch_version(requirements: &str) -> Option<String> {
    pinned_version(requirements, "torch")
}

/// A path is only deletable when it resolves to exactly `<app_dir>/<name>` —
/// a real directory, not a link. `canonicalize` alone follows `python-runtime`
/// symlinks, so a link pointing at a sibling (e.g. `models`) would pass a
/// parent check and delete the wrong tree.
pub(crate) fn managed_child_dir(app_dir: &Path, dir: &Path) -> Result<PathBuf, String> {
    let base = app_dir
        .canonicalize()
        .map_err(|e| format!("Could not resolve the app data directory: {e}"))?;
    if let Ok(meta) = std::fs::symlink_metadata(dir) {
        if meta.file_type().is_symlink() {
            return Err(format!(
                "Refusing to delete {}: it is a symbolic link",
                dir.display()
            ));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err(format!(
                    "Refusing to delete {}: it is a junction/reparse point",
                    dir.display()
                ));
            }
        }
    }
    let resolved = dir
        .canonicalize()
        .map_err(|e| format!("Could not resolve {}: {e}", dir.display()))?;
    let name = dir
        .file_name()
        .ok_or_else(|| format!("Refusing to delete {}: no directory name", dir.display()))?;
    let expected = base.join(name);
    if resolved != expected {
        return Err(format!(
            "Refusing to delete {}: it does not resolve to the managed directory {}",
            dir.display(),
            expected.display()
        ));
    }
    Ok(resolved)
}

fn remove_managed_dir(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    let resolved = managed_child_dir(&PlatformSys::get_app_dir(), dir)?;
    std::fs::remove_dir_all(&resolved)
        .map_err(|e| format!("Could not remove {}: {e}", resolved.display()))
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    for entry in entries.flatten() {
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if meta.is_dir() {
            total += dir_size(&entry.path());
        } else {
            total += meta.len();
        }
    }
    total
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeReceipt {
    pub pbs_tag: String,
    pub python_version: String,
    pub torch_variant: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PythonRuntimeStatus {
    /// The managed venv exists and `reflow-runtime.json` is present.
    pub managed_installed: bool,
    pub receipt: Option<RuntimeReceipt>,
    /// `REFLOW_PYTHON` or a saved `asr.python_path` overrides discovery —
    /// managed setup is pointless in that configuration.
    pub overridden: bool,
    pub setting_up: bool,
    /// Some interpreter `find_python` can already run (managed or otherwise).
    pub python_available: bool,
    /// Bytes reclaimable by `remove_python_runtime` (runtime + base dirs).
    pub size_bytes: u64,
    /// The torch wheel flavor setup would install: "cu130" | "cpu" |
    /// "default" (PyPI) | "none" (no torch wheel for this platform — only the
    /// sherpa-onnx speech path gets set up).
    pub torch_variant: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SetupProgressEvent {
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_pct: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn emit_progress(
    app: &AppHandle,
    phase: &str,
    progress_pct: Option<u32>,
    detail: Option<String>,
    error: Option<String>,
) {
    let _ = app.emit(
        PROGRESS_EVENT,
        SetupProgressEvent {
            phase: phase.to_string(),
            progress_pct,
            detail,
            error,
        },
    );
}

static SETUP_ACTIVE: AtomicBool = AtomicBool::new(false);
static SETUP_CANCEL: AtomicBool = AtomicBool::new(false);
/// Set once the worker deletes the previous `python-runtime` — only then is
/// the runtime dir this attempt's to clean up on failure/cancel.
static SETUP_CLAIMED_RUNTIME: AtomicBool = AtomicBool::new(false);
/// The currently spawned child (venv/pip) so Cancel can kill it mid-run.
static SETUP_CHILD: Mutex<Option<Child>> = Mutex::new(None);

fn cancelled() -> Result<(), String> {
    if SETUP_CANCEL.load(Ordering::SeqCst) {
        Err(CANCELLED.to_string())
    } else {
        Ok(())
    }
}

fn read_receipt(runtime_dir: &Path) -> Option<RuntimeReceipt> {
    let bytes = std::fs::read(runtime_dir.join(RECEIPT_NAME)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn runtime_status(ctx: &AppContext) -> PythonRuntimeStatus {
    let app_dir = PlatformSys::get_app_dir();
    let runtime_dir = app_dir.join(RUNTIME_DIR_NAME);
    let interpreter = runtime_dir.join(if cfg!(windows) {
        "Scripts\\python.exe"
    } else {
        "bin/python"
    });
    let receipt = read_receipt(&runtime_dir);
    let overridden = std::env::var("REFLOW_PYTHON")
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
        || ctx
            .settings_store
            .get()
            .asr
            .python_path
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
    PythonRuntimeStatus {
        managed_installed: interpreter.exists() && receipt.is_some(),
        receipt,
        overridden,
        setting_up: SETUP_ACTIVE.load(Ordering::SeqCst),
        python_available: crate::asr::sidecar::Qwen3AsrSidecar::find_python().is_some(),
        size_bytes: dir_size(&runtime_dir) + dir_size(&app_dir.join(BASE_DIR_NAME)),
        torch_variant: resolved_torch_variant().receipt_name().to_string(),
    }
}

#[tauri::command]
pub fn get_python_runtime_status(ctx: State<'_, AppContext>) -> PythonRuntimeStatus {
    runtime_status(ctx.inner())
}

#[tauri::command]
pub fn setup_python_runtime(app: AppHandle, ctx: State<'_, AppContext>) -> Result<(), String> {
    if SETUP_ACTIVE.swap(true, Ordering::SeqCst) {
        return Err("Speech runtime setup is already in progress.".to_string());
    }
    SETUP_CANCEL.store(false, Ordering::SeqCst);
    SETUP_CLAIMED_RUNTIME.store(false, Ordering::SeqCst);
    let app_clone = app.clone();
    let ctx_clone = ctx.inner().clone();
    std::thread::spawn(move || {
        let resource_dir = app_clone.path().resource_dir().ok();
        let emit = |phase: &str,
                    progress_pct: Option<u32>,
                    detail: Option<String>,
                    error: Option<String>| {
            emit_progress(&app_clone, phase, progress_pct, detail, error);
        };
        let outcome = run_setup_worker(&emit, resource_dir.as_deref());
        // Order matters: clean up anything this attempt built, release the
        // setting_up flag, THEN publish the terminal event — a UI status
        // refetch triggered by the event must never observe setting_up=true.
        if let Some(mut child) = SETUP_CHILD.lock().unwrap().take() {
            let _ = child.kill();
        }
        match outcome {
            Ok(()) => {
                SETUP_ACTIVE.store(false, Ordering::SeqCst);
                // Restart the sidecar on the new interpreter. A running
                // dictation blocks the reload — the user can reload from
                // Model settings in that case.
                if let Err(error) =
                    crate::commands::reload_model_inner(app_clone.clone(), &ctx_clone)
                {
                    log::warn!("Runtime installed; speech model reload deferred: {error}");
                }
                emit_progress(&app_clone, "complete", Some(100), None, None);
            }
            Err(err) if err == CANCELLED => {
                // A killed child leaves a half-built venv — never let it be
                // mistaken for a working interpreter. Only the runtime dir is
                // removed when this attempt actually replaced it; a cancel
                // before stage 3 leaves a pre-existing install intact.
                if SETUP_CLAIMED_RUNTIME.load(Ordering::SeqCst) {
                    if let Err(cleanup) =
                        remove_managed_dir(&PlatformSys::get_app_dir().join(RUNTIME_DIR_NAME))
                    {
                        log::warn!("Could not clean up cancelled runtime setup: {cleanup}");
                    }
                }
                SETUP_ACTIVE.store(false, Ordering::SeqCst);
                emit_progress(&app_clone, "cancelled", None, None, None);
            }
            Err(err) => {
                if SETUP_CLAIMED_RUNTIME.load(Ordering::SeqCst) {
                    if let Err(cleanup) =
                        remove_managed_dir(&PlatformSys::get_app_dir().join(RUNTIME_DIR_NAME))
                    {
                        log::warn!("Could not clean up failed runtime setup: {cleanup}");
                    }
                }
                SETUP_ACTIVE.store(false, Ordering::SeqCst);
                emit_progress(&app_clone, "error", None, None, Some(err));
            }
        }
        SETUP_CLAIMED_RUNTIME.store(false, Ordering::SeqCst);
    });
    Ok(())
}

/// Kill the in-flight setup child *and* its children. pip spawns console
/// helpers; killing only the direct child leaves them downloading.
fn kill_setup_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        // No Job Object (Win32_System_JobObjects isn't enabled) — taskkill's
        // tree kill covers pip's console subprocesses instead.
        use std::os::windows::process::CommandExt;
        let mut taskkill = Command::new("taskkill");
        taskkill
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .creation_flags(0x08000000); // CREATE_NO_WINDOW
        let _ = taskkill.output();
    }
    #[cfg(unix)]
    {
        // Spawned with process_group(0) — kill the whole group. The `--`
        // separator is required by util-linux kill, which would otherwise
        // parse `-<pgid>` as options.
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", child.id())])
            .output();
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[tauri::command]
pub fn cancel_python_runtime_setup() -> Result<(), String> {
    SETUP_CANCEL.store(true, Ordering::SeqCst);
    // Kill in place: the worker still needs the Child slot to wait() the
    // exit status and observe the cancelled flag itself.
    if let Some(child) = SETUP_CHILD.lock().unwrap().as_mut() {
        kill_setup_tree(child);
    }
    Ok(())
}

/// App-exit teardown: flag the worker cancelled and reap any in-flight
/// install child so a pip subprocess cannot outlive the app.
pub fn teardown_on_exit() {
    SETUP_CANCEL.store(true, Ordering::SeqCst);
    if let Some(mut child) = SETUP_CHILD.lock().unwrap().take() {
        kill_setup_tree(&mut child);
    }
}

#[tauri::command]
pub fn remove_python_runtime(ctx: State<'_, AppContext>) -> Result<u64, String> {
    if SETUP_ACTIVE.load(Ordering::SeqCst) {
        return Err("Speech runtime setup is in progress. Cancel it first.".into());
    }
    // Recording, processing, injecting — anything mid-dictation owns the
    // sidecar right now.
    if matches!(
        *ctx.state_enum.read(),
        crate::state::AppStateEnum::Recording
            | crate::state::AppStateEnum::Processing
            | crate::state::AppStateEnum::Injecting
    ) {
        return Err("Finish the current dictation before removing the speech runtime.".into());
    }
    let app_dir = PlatformSys::get_app_dir();
    let freed = dir_size(&app_dir.join(RUNTIME_DIR_NAME)) + dir_size(&app_dir.join(BASE_DIR_NAME));
    // Stop the sidecar first: unload kills the child process and waits for
    // it to exit — on Windows the interpreter's exes/DLLs are locked while
    // the process lives, so deleting first would fail or half-delete.
    ctx.asr_handle
        .shutdown_blocking(std::time::Duration::from_secs(10))
        .map_err(|error| format!("Could not stop the speech runtime: {error}"))?;
    remove_managed_dir(&app_dir.join(RUNTIME_DIR_NAME))?;
    remove_managed_dir(&app_dir.join(BASE_DIR_NAME))?;
    Ok(freed)
}

/// The variant this machine would install — the worker's actual decision,
/// shared with `runtime_status` so the UI can describe the right package set.
/// `REFLOW_TORCH_VARIANT` forces the wheel flavor (testing, debugging).
fn resolved_torch_variant() -> TorchVariant {
    let nvidia = crate::capability::capabilities().cuda.nvidia_gpu_present;
    match std::env::var("REFLOW_TORCH_VARIANT").as_deref() {
        Ok("cu130") => TorchVariant::Cuda,
        Ok("cpu") => TorchVariant::Cpu,
        Ok("default") | Ok("pypi") => TorchVariant::Pypi,
        _ => torch_variant(target_triple(), nvidia),
    }
}

/// The actual install. `emit` receives (phase, progress_pct, detail, error)
/// event fields; keeping it a closure means the worker is testable without a
/// running Tauri app.
type SetupEmit<'a> = &'a dyn Fn(&str, Option<u32>, Option<String>, Option<String>);

pub(crate) fn run_setup_worker(emit: SetupEmit, resource_dir: Option<&Path>) -> Result<(), String> {
    crate::network_policy::check_download()?;
    cancelled()?;

    let app_dir = PlatformSys::get_app_dir();
    std::fs::create_dir_all(&app_dir)
        .map_err(|e| format!("Could not create app data directory: {e}"))?;
    #[cfg(windows)]
    check_long_paths(&app_dir)?;

    let variant = resolved_torch_variant();
    let estimate = match variant {
        TorchVariant::Cuda => DISK_ESTIMATE_CUDA,
        TorchVariant::None_ => DISK_ESTIMATE_NO_TORCH,
        _ => DISK_ESTIMATE_CPU,
    };
    crate::network_policy::check_disk_space(&app_dir, estimate)?;

    let pin = pin_for(target_triple()).ok_or_else(|| {
        format!(
            "This platform ({}) has no pinned Python build.",
            target_triple()
        )
    })?;

    // Stage 1: download + verify the interpreter archive.
    let cache_dir = PlatformSys::get_cache_dir();
    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("Could not create download cache dir: {e}"))?;
    let partial = cache_dir.join(format!("{}.partial", pin.asset));
    let archive = cache_dir.join(pin.asset);
    download_archive(emit, pin, &partial, &archive)?;
    cancelled()?;
    crate::rewrite::runtime_install::verify_sha256(&archive, pin.sha256)?;

    // Stage 2: extract the `python/` tree into python-base via a staging dir.
    emit("extracting", None, None, None);
    let staging = app_dir.join(format!("{BASE_DIR_NAME}.staging"));
    let base_dir = app_dir.join(BASE_DIR_NAME);
    remove_managed_dir(&staging)?;
    std::fs::create_dir_all(&staging)
        .map_err(|e| format!("Could not create extract staging dir: {e}"))?;
    extract_archive(&archive, &staging)?;
    remove_managed_dir(&base_dir)?;
    std::fs::rename(&staging, &base_dir)
        .map_err(|e| format!("Could not move the extracted Python into place: {e}"))?;
    cancelled()?;

    // Stage 3: create the venv the sidecar discovers as python-runtime.
    emit("creating_env", None, None, None);
    let runtime_dir = app_dir.join(RUNTIME_DIR_NAME);
    // Only now does this attempt own the runtime directory — failures or
    // cancels before this point must leave a previously installed
    // python-runtime alone.
    remove_managed_dir(&runtime_dir)?;
    SETUP_CLAIMED_RUNTIME.store(true, Ordering::SeqCst);
    let base_python = base_dir.join("python").join(if cfg!(windows) {
        "python.exe"
    } else {
        "bin/python3"
    });
    run_setup_command(
        emit,
        "creating_env",
        &base_python,
        &[
            "-m".to_string(),
            "venv".to_string(),
            runtime_dir.to_string_lossy().into_owned(),
        ],
    )?;
    cancelled()?;

    let venv_python = runtime_dir.join(if cfg!(windows) {
        "Scripts\\python.exe"
    } else {
        "bin/python"
    });

    // Stage 4: torch from the platform index so NVIDIA machines get CUDA
    // wheels and everyone else gets the far smaller CPU build.
    let requirements = crate::asr::sidecar::Qwen3AsrSidecar::find_runtime_script(resource_dir)
        .map(|script| script.with_file_name("requirements.txt"))
        .filter(|path| path.exists());
    let requirements =
        requirements.ok_or("Could not locate the bundled model-runtime/requirements.txt")?;
    let requirements_text = std::fs::read_to_string(&requirements)
        .map_err(|e| format!("Could not read requirements.txt: {e}"))?;
    if variant != TorchVariant::None_ {
        crate::network_policy::check_download()
            .map_err(|_| "Downloads were turned off before the torch stage.".to_string())?;
        if let Some(version) = torch_version(&requirements_text) {
            emit("installing_torch", None, None, None);
            let mut args = vec![
                "-m".to_string(),
                "pip".to_string(),
                "install".to_string(),
                "--no-cache-dir".to_string(),
                "--disable-pip-version-check".to_string(),
                format!("torch=={version}"),
            ];
            if let Some(index) = variant.index_url() {
                args.push("--index-url".to_string());
                args.push(index.to_string());
            }
            run_setup_command(emit, "installing_torch", &venv_python, &args)?;
            cancelled()?;
        }
    }

    crate::network_policy::check_download()
        .map_err(|_| "Downloads were turned off before the package stage.".to_string())?;
    // Stage 5: the pinned speech dependencies. On torch-less platforms
    // (x86_64 macOS) Qwen/Phonon requirements cannot be satisfied — install
    // the sherpa-onnx (Zipformer) set instead.
    emit("installing_packages", None, None, None);
    if variant == TorchVariant::None_ {
        let zipformer_reqs = requirements.with_file_name("requirements-zipformer.txt");
        if !zipformer_reqs.exists() {
            return Err(
                "Could not locate the bundled model-runtime/requirements-zipformer.txt".into(),
            );
        }
        // Downloads import httpx + huggingface_hub even on the torch-free
        // path — install them pinned when requirements.txt names a version.
        let mut args = vec![
            "-m".to_string(),
            "pip".to_string(),
            "install".to_string(),
            "--no-cache-dir".to_string(),
            "--disable-pip-version-check".to_string(),
            "numpy".to_string(),
        ];
        for package in ["httpx", "huggingface_hub"] {
            match pinned_version(&requirements_text, package) {
                Some(version) => args.push(format!("{package}=={version}")),
                None => args.push(package.to_string()),
            }
        }
        args.push("-r".to_string());
        args.push(zipformer_reqs.to_string_lossy().into_owned());
        run_setup_command(emit, "installing_packages", &venv_python, &args)?;
    } else {
        run_setup_command(
            emit,
            "installing_packages",
            &venv_python,
            &[
                "-m".to_string(),
                "pip".to_string(),
                "install".to_string(),
                "--no-cache-dir".to_string(),
                "--disable-pip-version-check".to_string(),
                "-r".to_string(),
                requirements.to_string_lossy().into_owned(),
            ],
        )?;
    }
    cancelled()?;

    // Stage 6: verify the modules the applicable models probe at startup.
    emit("verifying", None, None, None);
    let verify_imports = if variant == TorchVariant::None_ {
        "import numpy, sherpa_onnx, httpx, huggingface_hub"
    } else {
        "import numpy, torch, transformers, huggingface_hub"
    };
    run_setup_command(
        emit,
        "verifying",
        &venv_python,
        &["-c".to_string(), verify_imports.to_string()],
    )?;

    let python_version = pip_probe(&venv_python, &["--version"])
        .and_then(|line| line.split_whitespace().nth(1).map(str::to_string))
        .unwrap_or_else(|| PBS_PYTHON_VERSION.to_string());
    let receipt = RuntimeReceipt {
        pbs_tag: PBS_TAG.to_string(),
        python_version,
        torch_variant: variant.receipt_name().to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let bytes = serde_json::to_vec_pretty(&receipt)
        .map_err(|e| format!("Could not serialize runtime receipt: {e}"))?;
    std::fs::write(runtime_dir.join(RECEIPT_NAME), bytes)
        .map_err(|e| format!("Could not write runtime receipt: {e}"))?;

    Ok(())
}

/// The venv's deep `Lib/site-packages` tree easily exceeds the classic
/// 260-char MAX_PATH. With a deep app data folder and long path support off,
/// fail before downloading instead of dying mid-install.
#[cfg(windows)]
fn check_long_paths(app_dir: &Path) -> Result<(), String> {
    let len = app_dir.to_string_lossy().chars().count();
    if len <= 80 {
        return Ok(());
    }
    use std::os::windows::process::CommandExt;
    let enabled = Command::new("reg")
        .args([
            "query",
            r"HKLM\SYSTEM\CurrentControlSet\Control\FileSystem",
            "/v",
            "LongPathsEnabled",
        ])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.contains("0x1"))
        .unwrap_or(false);
    if enabled {
        return Ok(());
    }
    Err(format!(
        "The app data folder path is {len} characters and Windows long path support is disabled —          the speech runtime would create paths beyond 260 characters and fail partway. Enable          long paths (as administrator: reg add          HKLM\\SYSTEM\\CurrentControlSet\\Control\\FileSystem /v LongPathsEnabled /t          REG_DWORD /d 1) or use a shorter portable folder."
    ))
}

/// Small stdout capture used for the receipt's python version.
fn pip_probe(program: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    cmd.output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            String::from_utf8(output.stdout)
                .ok()
                .map(|text| text.trim().to_string())
        })
}

/// Download with resume + throttled progress events, following the same
/// retry policy as the llama runtime downloader.
enum DownloadMsg {
    /// Response headers arrived: whether the server honored the Range resume,
    /// how many bytes are already on disk, and how many remain.
    Ready {
        resumed: bool,
        already_have: u64,
        remaining: u64,
    },
    Chunk(Vec<u8>),
    Done,
    Failed(String),
}

fn download_archive(
    emit: SetupEmit,
    pin: &PythonBuildPin,
    partial: &Path,
    archive: &Path,
) -> Result<(), String> {
    crate::network_policy::check_download()?;
    // A verified previous download is kept in the cache for retries.
    if let Ok(meta) = std::fs::metadata(archive) {
        if meta.len() > 0
            && crate::rewrite::runtime_install::verify_sha256(archive, pin.sha256).is_ok()
        {
            emit("downloading", Some(100), None, None);
            return Ok(());
        }
        let _ = std::fs::remove_file(archive);
    }
    let resume_from: u64 = std::fs::metadata(partial).map(|m| m.len()).unwrap_or(0);
    let url = pin.url.to_string();
    let approx_bytes = pin.approx_bytes;

    // Send, header wait, and body read all live on the worker: a stall at
    // any stage must not pin Cancel, which drops this channel and returns.
    // The worker itself still ends on its own — a 15s connect timeout plus a
    // 30s per-read timeout (the async client's read_timeout applies to the
    // blocking wrapper too) means a hung socket errors out shortly after.
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<DownloadMsg>();
    let _worker = std::thread::spawn(move || {
        let async_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60 * 30))
            .connect_timeout(std::time::Duration::from_secs(15))
            .read_timeout(std::time::Duration::from_secs(30))
            .redirect(crate::network_policy::redirect_policy());
        let client = match reqwest::blocking::ClientBuilder::from(async_client).build() {
            Ok(client) => client,
            Err(error) => {
                let _ = msg_tx.send(DownloadMsg::Failed(format!(
                    "HTTP client init failed: {error}"
                )));
                return;
            }
        };
        let mut response =
            match crate::rewrite::runtime_install::send_with_retry(&client, &url, resume_from) {
                Ok(response) => response,
                Err(error) => {
                    let _ = msg_tx.send(DownloadMsg::Failed(error));
                    return;
                }
            };
        let status = response.status();
        let resumed = status == reqwest::StatusCode::PARTIAL_CONTENT;
        if !resumed && !status.is_success() {
            let _ = msg_tx.send(DownloadMsg::Failed(format!(
                "Download failed: HTTP {status}"
            )));
            return;
        }
        let already_have = if resumed { resume_from } else { 0 };
        let remaining = response
            .content_length()
            .unwrap_or(approx_bytes.saturating_sub(already_have));
        if msg_tx
            .send(DownloadMsg::Ready {
                resumed,
                already_have,
                remaining,
            })
            .is_err()
        {
            return;
        }
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            match std::io::Read::read(&mut response, &mut buffer) {
                Ok(0) => {
                    let _ = msg_tx.send(DownloadMsg::Done);
                    return;
                }
                Ok(n) => {
                    if msg_tx
                        .send(DownloadMsg::Chunk(buffer[..n].to_vec()))
                        .is_err()
                    {
                        return; // receiver dropped: cancelled
                    }
                }
                Err(error) => {
                    let _ = msg_tx.send(DownloadMsg::Failed(format!("Read failed: {error}")));
                    return;
                }
            }
        }
    });

    let mut file = None;
    let mut last_emit = Instant::now();
    loop {
        match msg_rx.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(DownloadMsg::Ready {
                resumed,
                already_have,
                remaining,
            }) => {
                if !resumed && partial.exists() {
                    let _ = std::fs::remove_file(partial);
                }
                let total = already_have + remaining;
                if total == 0 {
                    return Err("Download has zero total size".into());
                }
                let mut opened = std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(partial)
                    .map_err(|e| format!("Could not open download target: {e}"))?;
                opened
                    .seek(SeekFrom::Start(already_have))
                    .map_err(|e| format!("Could not seek in download target: {e}"))?;
                emit(
                    "downloading",
                    Some((already_have * 100 / total) as u32),
                    None,
                    None,
                );
                file = Some((opened, already_have, total));
            }
            Ok(DownloadMsg::Chunk(chunk)) => {
                let Some((file, downloaded, total)) = file.as_mut() else {
                    return Err("Download stream started before headers".into());
                };
                file.write_all(&chunk)
                    .map_err(|e| format!("Write failed: {e}"))?;
                *downloaded += chunk.len() as u64;
                if last_emit.elapsed() >= std::time::Duration::from_millis(250) {
                    emit(
                        "downloading",
                        Some((*downloaded * 100 / *total) as u32),
                        None,
                        None,
                    );
                    last_emit = Instant::now();
                }
            }
            Ok(DownloadMsg::Done) => break,
            Ok(DownloadMsg::Failed(error)) => return Err(error),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => cancelled()?,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Download worker ended unexpectedly".into());
            }
        }
        cancelled()?;
    }
    let Some((mut file, _, _)) = file else {
        return Err("Download worker ended unexpectedly".into());
    };
    let _ = file.flush();
    let _ = file.sync_all();
    if archive.exists() {
        let _ = std::fs::remove_file(archive);
    }
    std::fs::rename(partial, archive).map_err(|e| format!("Could not finalize download: {e}"))?;
    Ok(())
}

/// Unpack the whole `python/` tree. tar's unpack refuses `..` components and
/// preserves the symlinks CPython relies on (bin/python3 → python3.12).
fn extract_archive(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| format!("Open tar.gz: {e}"))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    tar.unpack(dest)
        .map_err(|e| format!("Extract Python archive: {e}"))?;
    let interpreter = dest.join("python").join(if cfg!(windows) {
        "python.exe"
    } else {
        "bin/python3"
    });
    if !interpreter.exists() {
        return Err("Python archive did not contain an interpreter".into());
    }
    Ok(())
}

/// Spawn a child with its output streamed back to the UI as `detail` lines.
/// The child is parked in SETUP_CHILD so Cancel can kill it; stderr is
/// drained on a thread so both pipes merge without deadlock.
fn run_setup_command(
    emit: SetupEmit,
    phase: &str,
    program: &Path,
    args: &[String],
) -> Result<(), String> {
    cancelled()?;
    let mut cmd = Command::new(program);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    #[cfg(target_os = "linux")]
    crate::platform::process::own_child(&mut cmd);
    #[cfg(unix)]
    {
        // Own process group: cancelling or quitting can then kill pip and any
        // children it spawned (console subprocesses) with one group signal.
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Could not start {}: {e}", program.display()))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    {
        // Cancel can land between spawn and registration — re-check the flag
        // while still holding the slot lock and reap the just-spawned child.
        let mut slot = SETUP_CHILD.lock().unwrap();
        *slot = Some(child);
        if SETUP_CANCEL.load(Ordering::SeqCst) {
            if let Some(running) = slot.as_mut() {
                let _ = running.kill();
            }
        }
    }

    // Both pipes drain on reader threads into one channel so the main loop
    // wakes on a timer, not on output — a quiet pip stage (long single file
    // downloads) still answers cancel and airplane mode within ~500ms.
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let mut readers = Vec::new();
    let streams: [Option<Box<dyn std::io::Read + Send>>; 2] = [
        stdout.map(|s| Box::new(s) as _),
        stderr.map(|s| Box::new(s) as _),
    ];
    for stream in streams.into_iter().flatten() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    return;
                }
            }
        }));
    }
    drop(tx);

    let mut last_emit = Instant::now();
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(line) => {
                if last_emit.elapsed() >= std::time::Duration::from_millis(250) {
                    emit(phase, None, Some(line), None);
                    last_emit = Instant::now();
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        // Cancel can land while the child still holds the lock slot.
        if SETUP_CANCEL.load(Ordering::SeqCst) {
            if let Some(child) = SETUP_CHILD.lock().unwrap().as_mut() {
                kill_setup_tree(child);
            }
            return Err(CANCELLED.to_string());
        }
        // Airplane mode mid-install must kill pip promptly — a staged
        // download ignores the cancel flag unless someone checks it.
        if crate::network_policy::check_download().is_err() {
            if let Some(child) = SETUP_CHILD.lock().unwrap().as_mut() {
                kill_setup_tree(child);
            }
            return Err(
                "Downloads were turned off mid-setup (airplane mode). Setup cancelled.".into(),
            );
        }
    }
    for reader in readers {
        let _ = reader.join();
    }

    let child = SETUP_CHILD.lock().unwrap().take();
    let Some(mut child) = child else {
        cancelled()?;
        return Err("Setup process was unexpectedly taken over".into());
    };
    let status = child
        .wait()
        .map_err(|e| format!("Could not wait for {}: {e}", program.display()))?;
    cancelled()?;
    if !status.success() {
        return Err(format!("{} exited with {}", program.display(), status));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_target_has_a_verified_pin() {
        for target in [
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu",
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
            "x86_64-pc-windows-msvc",
        ] {
            let pin = pin_for(target).unwrap_or_else(|| panic!("missing pin for {target}"));
            assert_eq!(pin.target, target);
            assert!(pin.asset.starts_with("cpython-3.12.15+20261003-"));
            assert!(pin.asset.ends_with("-install_only.tar.gz"));
            assert!(!pin.asset.contains("stripped"));
            assert_eq!(pin.sha256.len(), 64);
            assert!(pin.url.contains(&*pin.asset.replace('+', "%2B")));
            assert!(pin.approx_bytes > 10_000_000);
        }
        assert!(pin_for("riscv64-unknown-linux-gnu").is_none());
        assert!(pin_for(target_triple()).is_some());
    }

    #[test]
    fn torch_variant_picks_index_per_target() {
        assert_eq!(
            torch_variant("x86_64-unknown-linux-gnu", true),
            TorchVariant::Cuda
        );
        assert_eq!(
            torch_variant("x86_64-pc-windows-msvc", true),
            TorchVariant::Cuda
        );
        assert_eq!(
            torch_variant("x86_64-unknown-linux-gnu", false),
            TorchVariant::Cpu
        );
        assert_eq!(
            torch_variant("x86_64-pc-windows-msvc", false),
            TorchVariant::Cpu
        );
        assert_eq!(
            torch_variant("aarch64-apple-darwin", false),
            TorchVariant::Pypi
        );
        // Torch publishes no Intel-mac wheels — the variant is none so the
        // worker skips stage 4 and installs the sherpa-onnx set instead.
        assert_eq!(
            torch_variant("x86_64-apple-darwin", false),
            TorchVariant::None_
        );
        assert_eq!(TorchVariant::None_.receipt_name(), "none");
        assert_eq!(
            TorchVariant::Cuda.index_url(),
            Some("https://download.pytorch.org/whl/cu130")
        );
        assert_eq!(
            TorchVariant::Cpu.index_url(),
            Some("https://download.pytorch.org/whl/cpu")
        );
        assert_eq!(TorchVariant::Pypi.index_url(), None);
    }

    #[test]
    fn torch_version_reads_the_pin_and_ignores_torchao() {
        let requirements =
            "# speech\ntorch==2.13.0\ntransformers==5.15.1\ntorchao==0.18.0\nnumpy==2.4.4\n";
        assert_eq!(torch_version(requirements).as_deref(), Some("2.13.0"));
        assert_eq!(torch_version("numpy==2.4.4\ntorchao==0.18.0\n"), None);
        assert_eq!(torch_version("torch == 2.13.0\n"), None);
    }

    #[test]
    fn managed_child_dir_only_allows_direct_children() {
        let root = std::env::temp_dir().join(format!("reflow-guard-{}", std::process::id()));
        let app_dir = root.join("app");
        let outside = root.join("outside");
        std::fs::create_dir_all(app_dir.join("python-runtime")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        assert!(managed_child_dir(&app_dir, &app_dir.join("python-runtime")).is_ok());
        assert!(managed_child_dir(&app_dir, &outside).is_err());
        assert!(managed_child_dir(
            &app_dir,
            &app_dir.join("..").join("outside").canonicalize().unwrap()
        )
        .is_err());

        // A symlink inside app_dir pointing outside must not be deletable.
        #[cfg(unix)]
        {
            let link = app_dir.join("sneaky");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert!(managed_child_dir(&app_dir, &link).is_err());
        }

        // The subtle case: a link resolving to a *sibling inside* app_dir
        // passes a canonical-parent check — it must still be refused.
        #[cfg(unix)]
        {
            let models = app_dir.join("models");
            std::fs::create_dir_all(&models).unwrap();
            let link = app_dir.join("python-runtime");
            std::fs::remove_dir_all(&link).unwrap();
            std::os::unix::fs::symlink(&models, &link).unwrap();
            assert!(managed_child_dir(&app_dir, &link).is_err());
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(all(test, target_os = "linux"))]
    mod e2e {
        //! Opt-in real-network checks. `cargo test -- --ignored` runs them;
        //! the env override redirects app data into a temp dir so the user's
        //! real `~/.local/share/reflow` is untouched.
        use super::*;

        fn scratch_app_dir() -> PathBuf {
            // Stable name: the download cache is preserved across runs, so a
            // dropped connection resumes instead of restarting 67 MB.
            let dir = std::env::temp_dir().join("reflow-python-e2e");
            std::env::set_var("REFLOW_APP_DIR", &dir);
            crate::network_policy::configure(false, &dir).expect("go online");
            dir
        }

        #[test]
        #[ignore = "downloads ~67 MB and creates a venv; run with -- --ignored"]
        fn download_extract_venv_and_pip_smoke() {
            let app_dir = scratch_app_dir();
            let emits = std::sync::Mutex::new(Vec::<(String, Option<u32>)>::new());
            let emit =
                |phase: &str, pct: Option<u32>, _detail: Option<String>, _err: Option<String>| {
                    emits.lock().unwrap().push((phase.to_string(), pct));
                };
            let pin = pin_for(target_triple()).expect("linux pin");
            let cache = PlatformSys::get_cache_dir();
            std::fs::create_dir_all(&cache).unwrap();
            let partial = cache.join(format!("{}.partial", pin.asset));
            let archive = cache.join(pin.asset);

            download_archive(&emit, pin, &partial, &archive).expect("download");
            crate::rewrite::runtime_install::verify_sha256(&archive, pin.sha256)
                .expect("sha256 matches the published SHA256SUMS");

            let staging = app_dir.join("python-base.staging");
            std::fs::create_dir_all(&staging).unwrap();
            extract_archive(&archive, &staging).expect("extract");
            let base_python = staging.join("python").join("bin/python3");
            assert!(base_python.exists(), "extracted interpreter");

            let venv = app_dir.join(RUNTIME_DIR_NAME);
            run_setup_command(
                &emit,
                "creating_env",
                &base_python,
                &[
                    "-m".to_string(),
                    "venv".to_string(),
                    venv.to_string_lossy().into_owned(),
                ],
            )
            .expect("venv creation");
            let venv_python = venv.join("bin/python");
            let version = pip_probe(&venv_python, &["--version"]).expect("python --version");
            assert!(version.contains("3.12"), "unexpected version: {version}");
            let pip = pip_probe(&venv_python, &["-m", "pip", "--version"]);
            assert!(pip.is_some(), "venv pip missing");

            std::env::remove_var("REFLOW_APP_DIR");
            let _ = std::fs::remove_dir_all(&app_dir);
        }

        #[test]
        #[ignore = "downloads ~2 GB of speech packages; run with -- --ignored"]
        fn managed_setup_end_to_end() {
            let app_dir = scratch_app_dir();
            // Keep the run CPU-sized regardless of the test machine's GPU.
            std::env::set_var("REFLOW_TORCH_VARIANT", "cpu");
            let emits = std::sync::Mutex::new(Vec::<String>::new());
            let emit =
                |phase: &str, _pct: Option<u32>, _detail: Option<String>, _err: Option<String>| {
                    emits.lock().unwrap().push(phase.to_string());
                };
            run_setup_worker(&emit, None).expect("full setup");
            let runtime = app_dir.join(RUNTIME_DIR_NAME);
            assert!(runtime.join("bin/python").exists());
            let receipt = read_receipt(&runtime).expect("receipt written");
            assert_eq!(receipt.pbs_tag, PBS_TAG);
            assert_eq!(receipt.torch_variant, "cpu");
            let phases = emits.lock().unwrap().clone();
            // The terminal "complete" event is emitted by the command wrapper
            // after reload — the worker's last phase is "verifying".
            assert_eq!(phases.last().map(String::as_str), Some("verifying"));
            std::env::remove_var("REFLOW_TORCH_VARIANT");
            std::env::remove_var("REFLOW_APP_DIR");
            let _ = std::fs::remove_dir_all(&app_dir);
        }
    }
}
