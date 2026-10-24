//! One-click managed speech-runtime setup.
//!
//! Downloads a pinned python-build-standalone interpreter, creates a venv in
//! the app data directory (`<app_dir>/python-runtime`, which
//! [`super::sidecar::Qwen3AsrSidecar::find_python`] already probes first), and
//! installs the speech dependencies into it. All heavy work happens on a
//! background thread; progress is streamed to the UI via
//! `python-runtime:setup-progress` events.

use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::context::AppContext;
use crate::platform::PlatformSys;

const RUNTIME_DIR_NAME: &str = "python-runtime";
const BASE_DIR_NAME: &str = "python-base";
const RECEIPT_NAME: &str = "reflow-runtime.json";
const PROGRESS_EVENT: &str = "python-runtime:setup-progress";
/// Internal sentinel — a cancelled run reports `cancelled`, not `error`.
const CANCELLED: &str = "python runtime setup cancelled";

#[derive(Deserialize)]
pub struct PythonBuildPin {
    pub target: &'static str,
    pub asset: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub approx_bytes: u64,
}

#[derive(Deserialize)]
struct PythonBuilds {
    pbs_tag: &'static str,
    python_version: &'static str,
    pip_version: &'static str,
    pins: Vec<PythonBuildPin>,
}

fn python_builds() -> &'static PythonBuilds {
    static BUILDS: std::sync::OnceLock<PythonBuilds> = std::sync::OnceLock::new();
    BUILDS.get_or_init(|| {
        serde_json::from_str(include_str!("../../../model-runtime/python-builds.json"))
            .expect("checked-in Python build pins must be valid")
    })
}

pub(crate) fn pin_for(target: &str) -> Option<&'static PythonBuildPin> {
    python_builds().pins.iter().find(|pin| pin.target == target)
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
    /// CUDA 12.6 for older NVIDIA GPUs and drivers.
    CudaLegacy,
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
            Self::CudaLegacy => Some("https://download.pytorch.org/whl/cu126"),
            Self::Cpu => Some("https://download.pytorch.org/whl/cpu"),
            Self::Pypi | Self::None_ => None,
        }
    }
    pub(crate) fn receipt_name(&self) -> &'static str {
        match self {
            Self::Cuda => "cu130",
            Self::CudaLegacy => "cu126",
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
#[cfg(test)]
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

#[cfg(test)]
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
    #[serde(default)]
    pub requirements_digest: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_pack_sha256: Option<String>,
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
    pub progress: Option<SetupProgressEvent>,
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
    pub attempt_id: String,
    pub revision: u64,
    pub model: String,
    pub device: String,
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_pct: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn update_progress(
    attempt_id: &str,
    phase: &str,
    progress_pct: Option<u32>,
    detail: Option<String>,
    error: Option<String>,
) -> Option<SetupProgressEvent> {
    let mut snapshot = SETUP_PROGRESS.lock().unwrap();
    let progress = snapshot
        .as_mut()
        .filter(|progress| progress.attempt_id == attempt_id)?;
    if matches!(progress.phase.as_str(), "complete" | "error" | "cancelled") {
        return None;
    }
    progress.revision = progress.revision.saturating_add(1);
    progress.phase = phase.into();
    progress.progress_pct = progress_pct;
    progress.detail = detail;
    progress.error = error;
    if matches!(phase, "complete" | "error" | "cancelled") {
        SETUP_ACTIVE.store(false, Ordering::SeqCst);
    }
    Some(progress.clone())
}

fn emit_progress(
    app: &AppHandle,
    attempt_id: &str,
    phase: &str,
    progress_pct: Option<u32>,
    detail: Option<String>,
    error: Option<String>,
) {
    if let Some(progress) = update_progress(attempt_id, phase, progress_pct, detail, error) {
        let _ = app.emit(PROGRESS_EVENT, progress);
    }
}

fn claim_setup(model: &str, device: &str) -> Result<String, String> {
    let mut snapshot = SETUP_PROGRESS.lock().unwrap();
    if SETUP_ACTIVE.swap(true, Ordering::SeqCst) {
        return Err("Speech runtime setup is already in progress.".into());
    }
    SETUP_CANCEL.store(false, Ordering::SeqCst);
    let attempt_id = uuid::Uuid::new_v4().to_string();
    let revision = snapshot
        .as_ref()
        .map_or(1, |previous| previous.revision.saturating_add(1));
    *snapshot = Some(SetupProgressEvent {
        attempt_id: attempt_id.clone(),
        revision,
        model: model.into(),
        device: device.into(),
        phase: "downloading".into(),
        progress_pct: None,
        detail: None,
        error: None,
    });
    Ok(attempt_id)
}

static SETUP_PROGRESS: Mutex<Option<SetupProgressEvent>> = Mutex::new(None);
static SETUP_ACTIVE: AtomicBool = AtomicBool::new(false);
static SETUP_CANCEL: AtomicBool = AtomicBool::new(false);
/// The currently spawned child (venv/pip) so Cancel can kill it mid-run.
static SETUP_CHILD: Mutex<Option<Child>> = Mutex::new(None);

fn cancelled() -> Result<(), String> {
    if SETUP_CANCEL.load(Ordering::SeqCst) {
        Err(CANCELLED.to_string())
    } else {
        Ok(())
    }
}

fn begin_activation(emit: SetupEmit) -> Result<(), String> {
    {
        // Cancellation and activation share the progress lock: whichever wins
        // first determines whether the verified pointer may be committed.
        let mut snapshot = SETUP_PROGRESS.lock().unwrap();
        cancelled()?;
        if SETUP_ACTIVE.load(Ordering::SeqCst) {
            if let Some(progress) = snapshot.as_mut() {
                progress.phase = "activating".into();
                progress.revision = progress.revision.saturating_add(1);
            }
        }
    }
    emit("activating", Some(100), None, None);
    Ok(())
}

/// A single atomic pointer selects an immutable base/venv pair. Never move a
/// venv: its interpreter links, pyvenv.cfg and entrypoints contain absolute paths.
pub(crate) fn managed_runtime_dir(app_dir: &Path) -> PathBuf {
    std::fs::read_to_string(app_dir.join("python-active"))
        .ok()
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
        .map(|id| app_dir.join("python-generations").join(id).join("runtime"))
        .unwrap_or_else(|| app_dir.join(RUNTIME_DIR_NAME))
}

fn promote_generation(app_dir: &Path, id: &str) -> Result<(), String> {
    crate::platform::persistence::write_atomic(&app_dir.join("python-active"), id.as_bytes())
        .map_err(|e| format!("Could not activate verified speech runtime: {e}"))
}

pub(crate) fn dependency_profile(model: &str) -> &'static str {
    match model {
        "zipformer-20m" | "omnilingual-300m-int8" => "zipformer",
        "phonon-2" => "phonon",
        "parakeet-ultra" => "phonon",
        "indic-transcribe-8bit" => "indic",
        _ => "qwen",
    }
}

fn requirements_digest(profile: &str, variant: TorchVariant) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(include_bytes!("../../../model-runtime/requirements.txt"));
    digest.update(include_bytes!(
        "../../../model-runtime/requirements-phonon.txt"
    ));
    digest.update(include_bytes!(
        "../../../model-runtime/requirements-zipformer.txt"
    ));
    digest.update(include_bytes!("../../../model-runtime/managed_pip.py"));
    if profile == "indic" {
        digest.update(include_bytes!(
            "../../../model-runtime/requirements-indic.txt"
        ));
    }
    digest.update(
        super::dependency_locks::locked_requirements(
            target_triple(),
            profile,
            variant.receipt_name(),
        )
        .unwrap_or("unsupported"),
    );
    digest.update(include_bytes!("../../../model-runtime/locks/pip.txt"));
    digest.update(format!(
        "{profile}:{}:{}:v2",
        variant.receipt_name(),
        python_builds().pip_version
    ));
    format!("{:x}", digest.finalize())
}

pub(crate) fn variant_for(
    model: &str,
    device: &str,
    caps: &crate::capability::Capabilities,
) -> TorchVariant {
    if matches!(model, "zipformer-20m" | "omnilingual-300m-int8") {
        TorchVariant::None_
    } else if device == "cpu" {
        torch_variant(target_triple(), false)
    } else {
        resolved_torch_variant_for(caps)
    }
}

fn read_receipt(runtime_dir: &Path) -> Option<RuntimeReceipt> {
    let bytes = std::fs::read(runtime_dir.join(RECEIPT_NAME)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn runtime_overridden(ctx: &AppContext) -> bool {
    std::env::var("REFLOW_PYTHON")
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
        || ctx
            .settings_store
            .get()
            .asr
            .python_path
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

fn verify_installed_versions(interpreter: &Path, profile: &str, variant: TorchVariant) -> bool {
    let Some(locked) = super::dependency_locks::locked_requirements(
        target_triple(),
        profile,
        variant.receipt_name(),
    ) else {
        return false;
    };
    let pins: Vec<_> = locked
        .lines()
        .filter_map(|line| {
            let (name, version) = line.split_once("==")?;
            Some((name.trim(), version.trim().trim_end_matches('\\').trim()))
        })
        .collect();
    let Ok(pins) = serde_json::to_string(&pins) else {
        return false;
    };
    let prefix = serde_json::to_string(
        &interpreter
            .parent()
            .and_then(|p| p.parent())
            .unwrap_or(interpreter)
            .to_string_lossy(),
    )
    .unwrap_or_default();
    let code = format!("import importlib.metadata as m, json, sys, os; assert os.path.realpath(sys.prefix) == os.path.realpath({prefix}); pins=json.loads({pins:?}); assert all(m.version(name) == version for name,version in pins); print('ready')");
    super::sidecar::Qwen3AsrSidecar::bounded_python_probe(&interpreter.to_string_lossy(), &code)
        .is_some()
}

fn receipt_matches(receipt: &RuntimeReceipt, profile: &str, variant: TorchVariant) -> bool {
    receipt.target == target_triple()
        && receipt.profile == profile
        && receipt.requirements_digest == requirements_digest(profile, variant)
        && receipt.pbs_tag == python_builds().pbs_tag
        && receipt.torch_variant == variant.receipt_name()
}

/// Reuse a verified environment for the selected profile, including offline.
/// Selection is explicit; this command never downloads or alters installed weights.
#[tauri::command]
pub async fn activate_cached_speech_runtime(ctx: State<'_, AppContext>) -> Result<bool, String> {
    let ctx = ctx.inner().clone();
    let operation = speech_setup_admission(&ctx).await?;
    tokio::task::spawn_blocking(move || {
        let _session = operation;
        let _settings = ctx.settings_operation.lock();
        if SETUP_ACTIVE.load(Ordering::SeqCst) || ctx.meetings.busy() {
            return Err(
                "Speech resources are busy. Wait for setup or meeting processing to finish.".into(),
            );
        }
        if runtime_overridden(&ctx) {
            return Ok(false);
        }
        let selection = ctx.settings_store.get().asr;
        if selection.runtime != "python" {
            return Ok(false);
        }
        let profile = dependency_profile(&selection.model);
        let variant = variant_for(
            &selection.model,
            &selection.device,
            &crate::capability::capabilities(),
        );
        let app_dir = PlatformSys::get_app_dir();
        let current = managed_runtime_dir(&app_dir);
        let candidate = find_cached_generation(&app_dir, profile, variant, |runtime| {
            let python = runtime.join(if cfg!(windows) {
                "Scripts/python.exe"
            } else {
                "bin/python"
            });
            if !verify_installed_versions(&python, profile, variant) {
                return false;
            }
            let imports = verification_code(
                profile,
                variant,
                matches!(selection.precision.as_str(), "auto" | "int4" | "int8")
                    && matches!(variant, TorchVariant::Cuda | TorchVariant::CudaLegacy),
            );
            super::sidecar::Qwen3AsrSidecar::bounded_python_probe_for(
                &python.to_string_lossy(),
                &format!("{imports}; print('ready')"),
                Duration::from_secs(30),
            )
            .is_some()
        });
        let Some((id, runtime)) = candidate else {
            return Ok(false);
        };
        if runtime != current {
            ctx.asr_handle.shutdown_blocking(Duration::from_secs(10))?;
            promote_generation(&app_dir, &id)?;
        }
        Ok(true)
    })
    .await
    .map_err(|e| format!("Could not prepare the installed speech runtime: {e}"))?
}

fn find_cached_generation(
    app_dir: &Path,
    profile: &str,
    variant: TorchVariant,
    mut verify: impl FnMut(&Path) -> bool,
) -> Option<(String, PathBuf)> {
    let mut entries = std::fs::read_dir(app_dir.join("python-generations"))
        .ok()?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let id = entry.file_name().to_string_lossy().into_owned();
        if uuid::Uuid::parse_str(&id).is_err() || !entry.file_type().is_ok_and(|kind| kind.is_dir())
        {
            continue;
        }
        let runtime = entry.path().join("runtime");
        if read_receipt(&runtime).is_some_and(|r| receipt_matches(&r, profile, variant))
            && verify(&runtime)
        {
            return Some((id, runtime));
        }
    }
    None
}

pub fn runtime_status(ctx: &AppContext) -> PythonRuntimeStatus {
    runtime_status_for(ctx, None)
}

/// Progress polling must not probe Python or walk an environment being installed.
#[tauri::command]
pub fn get_python_setup_progress() -> Option<SetupProgressEvent> {
    SETUP_PROGRESS.lock().unwrap().clone()
}

fn runtime_status_for(ctx: &AppContext, model: Option<&str>) -> PythonRuntimeStatus {
    let app_dir = PlatformSys::get_app_dir();
    let runtime_dir = managed_runtime_dir(&app_dir);
    let interpreter = runtime_dir.join(if cfg!(windows) {
        "Scripts\\python.exe"
    } else {
        "bin/python"
    });
    let receipt = read_receipt(&runtime_dir);
    let overridden = runtime_overridden(ctx);
    let mut settings = ctx.settings_store.get();
    if let Some(model) = model {
        settings.asr.model = model.into();
    }
    let profile = dependency_profile(&settings.asr.model);
    let variant = variant_for(
        &settings.asr.model,
        &settings.asr.device,
        &crate::capability::capabilities(),
    );
    let ready = receipt
        .as_ref()
        .is_some_and(|r| receipt_matches(r, profile, variant));
    let managed_installed = !overridden
        && interpreter.exists()
        && ready
        && verify_installed_versions(&interpreter, profile, variant);
    let python_available = crate::asr::sidecar::Qwen3AsrSidecar::find_python().is_some();
    let size_bytes = dir_size(&app_dir.join(RUNTIME_DIR_NAME))
        + dir_size(&app_dir.join(BASE_DIR_NAME))
        + dir_size(&app_dir.join("python-generations"));
    // Python probes and directory scans can outlast an entire setup transition.
    // Capture the job state after them, so the reply cannot retain a pre-probe phase.
    let (setting_up, progress) = {
        let snapshot = SETUP_PROGRESS.lock().unwrap();
        (SETUP_ACTIVE.load(Ordering::SeqCst), snapshot.clone())
    };
    PythonRuntimeStatus {
        managed_installed,
        receipt,
        overridden,
        setting_up,
        progress,
        python_available,
        size_bytes,
        torch_variant: variant.receipt_name().to_string(),
    }
}

#[tauri::command]
pub async fn get_python_runtime_status(
    ctx: State<'_, AppContext>,
) -> Result<PythonRuntimeStatus, String> {
    let context = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || runtime_status(&context))
        .await
        .map_err(|error| format!("Could not check speech dependencies: {error}"))
}

/// Exercise libraries without loading or downloading any model weights. The
/// caller owns session admission so removal/setup cannot replace the interpreter.
pub(crate) fn verify_speech_runtime_inner(
    ctx: &AppContext,
    model: Option<&str>,
) -> Result<(), String> {
    if !matches!(
        *ctx.state_enum.read(),
        crate::state::AppStateEnum::Ready
            | crate::state::AppStateEnum::Idle
            | crate::state::AppStateEnum::Error
    ) {
        return Err("Finish dictation before checking speech dependencies.".into());
    }
    if SETUP_ACTIVE.load(Ordering::SeqCst) {
        return Err("Speech dependency setup is still running. Wait for it to finish.".into());
    }
    let settings = ctx.settings_store.get().asr;
    if settings.runtime != "python" {
        return Err("The selected speech engine does not use Python.".into());
    }
    let status = runtime_status_for(ctx, model);
    if !status.overridden && !status.managed_installed {
        return Err(
            "Install or repair speech dependencies before downloading a speech model.".into(),
        );
    }
    let python = super::sidecar::Qwen3AsrSidecar::find_python()
        .ok_or("The selected Python interpreter could not start. Check it in Model settings.")?;
    let model = model.unwrap_or(&settings.model);
    let variant = variant_for(model, &settings.device, &crate::capability::capabilities());
    let quantized = matches!(settings.precision.as_str(), "auto" | "int4" | "int8")
        && matches!(variant, TorchVariant::Cuda | TorchVariant::CudaLegacy);
    let imports = verification_code(dependency_profile(model), variant, quantized);
    // Report bounded diagnostics on stdout, including import/kernel failures.
    // The probe deadline covers a hung native import as well as interpreter startup.
    let code = format!(
        "import json\ntry:\n exec({imports:?})\n print('REFLOW_HEALTH:'+json.dumps({{'ok':True}}))\nexcept Exception as e:\n print('REFLOW_HEALTH:'+json.dumps({{'error':type(e).__name__+': '+str(e)[:1000]}}))"
    );
    let output = super::sidecar::Qwen3AsrSidecar::bounded_python_probe_for(
        &python,
        &code,
        std::time::Duration::from_secs(30),
    )
    .ok_or("The speech dependency check failed or exceeded 30 seconds. Repair dependencies or select another Python interpreter.")?;
    let health = String::from_utf8_lossy(&output)
        .lines()
        .filter_map(|line| line.strip_prefix("REFLOW_HEALTH:"))
        .next_back()
        .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .ok_or("The speech dependency check did not return a valid result.")?;
    if health.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err(format!(
            "Speech dependencies could not run: {}. Repair the environment, or select CPU in Model settings if a graphics check failed.",
            health.get("error").and_then(|v| v.as_str()).unwrap_or("unknown error")
        ));
    }
    if ctx.settings_store.get().asr != settings {
        return Err("Speech settings changed during the dependency check. Check again.".into());
    }
    Ok(())
}

async fn speech_setup_admission(
    ctx: &AppContext,
) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
    if ctx.meetings.busy()
        || !matches!(
            *ctx.state_enum.read(),
            crate::state::AppStateEnum::Ready
                | crate::state::AppStateEnum::Idle
                | crate::state::AppStateEnum::Error
        )
    {
        return Err("Finish the active recording before preparing speech dependencies.".into());
    }
    // A settings write may already have queued a model reload. Join behind it
    // instead of making an ordinary model switch fail with a transient busy error.
    tokio::time::timeout(
        Duration::from_secs(60),
        ctx.session_operation.clone().lock_owned(),
    )
    .await
    .map_err(|_| {
        "Speech resources are still busy. Wait for the model to finish loading and try again."
            .into()
    })
}

#[tauri::command]
pub async fn verify_speech_runtime(ctx: State<'_, AppContext>) -> Result<(), String> {
    let context = ctx.inner().clone();
    let operation = speech_setup_admission(&context).await?;
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = operation;
        verify_speech_runtime_inner(&context, None)
    })
    .await
    .map_err(|error| format!("Could not check speech dependencies: {error}"))?
}

fn verification_code(profile: &str, variant: TorchVariant, quantized: bool) -> String {
    let imports = match profile {
        "zipformer" => "import numpy, sherpa_onnx, httpx, huggingface_hub; assert numpy.ones(1).sum() == 1",
        "phonon" => "import numpy, torch, fermion, scipy, soundfile, safetensors, zstandard, httpx, huggingface_hub; from fermion._speech import engine_phonon2_cpu; from transformers import ParakeetForTDT, ParakeetTDTConfig; assert torch.ones(1).sum().item() == 1",
        "indic" => "import numpy, torch, bitsandbytes, sentencepiece, soundfile, accelerate, safetensors, httpx, huggingface_hub; from transformers import PreTrainedModel, GenerationMixin; assert torch.ones(1).sum().item() == 1",
        _ => "import numpy, torch, transformers, huggingface_hub, httpx; from transformers import AutoProcessor, AutoModelForMultimodalLM, Qwen3ASRProcessor, Qwen3ASRForConditionalGeneration; assert torch.ones(1).sum().item() == 1",
    };
    let quantization = if profile == "qwen" && quantized {
        "; import torchao, accelerate; from torchao.quantization import Int4WeightOnlyConfig, Int8WeightOnlyConfig; from transformers import TorchAoConfig"
    } else {
        ""
    };
    let gpu = if matches!(variant, TorchVariant::Cuda | TorchVariant::CudaLegacy) {
        "; assert torch.cuda.is_available(), 'GPU unavailable; update your graphics driver'; x = torch.ones(1, device='cuda:0'); assert x.sum().item() == 1; torch.cuda.synchronize()"
    } else {
        ""
    };
    let indic_kernel = if profile == "indic"
        && matches!(variant, TorchVariant::Cuda | TorchVariant::CudaLegacy)
    {
        "; layer = bitsandbytes.nn.Linear8bitLt(16, 16, has_fp16_weights=False).to('cuda:0'); assert torch.isfinite(layer(torch.ones(1, 16, device='cuda:0', dtype=torch.float16))).all().item(); torch.cuda.synchronize()"
    } else {
        ""
    };
    format!("{imports}{quantization}{gpu}{indic_kernel}")
}

#[tauri::command]
pub fn setup_python_runtime(app: AppHandle, ctx: State<'_, AppContext>) -> Result<(), String> {
    if runtime_overridden(ctx.inner()) {
        return Err(
            "Clear your selected Python interpreter in Model settings to use automatic setup."
                .into(),
        );
    }
    let selection = ctx.settings_store.get().asr;
    let attempt_id = claim_setup(&selection.model, &selection.device)?;
    let app_clone = app.clone();
    let ctx_clone = ctx.inner().clone();
    std::thread::spawn(move || {
        let resource_dir = app_clone.path().resource_dir().ok();
        let emit = |phase: &str,
                    progress_pct: Option<u32>,
                    detail: Option<String>,
                    error: Option<String>| {
            emit_progress(&app_clone, &attempt_id, phase, progress_pct, detail, error);
        };
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _operation = ctx_clone.session_operation.try_lock().map_err(|_| {
                "Finish the current dictation before setting up speech dependencies.".to_string()
            })?;
            if !matches!(
                *ctx_clone.state_enum.read(),
                crate::state::AppStateEnum::Ready
                    | crate::state::AppStateEnum::Idle
                    | crate::state::AppStateEnum::Error
            ) {
                return Err(
                    "Finish the current dictation before setting up speech dependencies.".into(),
                );
            }
            // Windows locks DLLs in use by the sidecar. Stop it before replacing
            // its interpreter, while preventing a dictation from starting.
            ctx_clone
                .asr_handle
                .shutdown_blocking(std::time::Duration::from_secs(10))?;
            run_setup_worker_for(
                &emit,
                resource_dir.as_deref(),
                &selection.model,
                &selection.device,
            )
        }))
        .unwrap_or_else(|panic| Err(setup_panic_error(panic)));
        // Order matters: clean up anything this attempt built, release the
        // setting_up flag, THEN publish the terminal event — a UI status
        // refetch triggered by the event must never observe setting_up=true.
        if let Some(mut child) = SETUP_CHILD.lock().unwrap().take() {
            kill_setup_tree(&mut child);
        }
        match outcome {
            Ok(()) => {
                // Restart the sidecar on the new interpreter. A running
                // dictation blocks the reload — the user can reload from
                // Model settings in that case.
                if ctx_clone
                    .model_manager
                    .is_installed(&ctx_clone.settings_store.get().asr.model)
                {
                    if let Err(error) =
                        crate::commands::reload_model_inner(app_clone.clone(), &ctx_clone)
                    {
                        log::warn!("Runtime installed; speech model reload deferred: {error}");
                    }
                }
                emit_progress(&app_clone, &attempt_id, "complete", Some(100), None, None);
            }
            Err(err) if err == CANCELLED => {
                emit_progress(&app_clone, &attempt_id, "cancelled", None, None, None);
            }
            Err(err) => {
                emit_progress(&app_clone, &attempt_id, "error", None, None, Some(err));
            }
        }
    });
    Ok(())
}

/// Kill the in-flight setup child *and* its children. pip spawns console
/// helpers; killing only the direct child leaves them downloading.
pub(super) fn kill_setup_tree(child: &mut Child) {
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
        // process_group(0) gives this child its own group on Linux and macOS.
        // Signal it directly; external kill utilities have differing options.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[tauri::command]
pub fn cancel_python_runtime_setup(attempt_id: Option<String>) -> Result<(), String> {
    let snapshot = SETUP_PROGRESS.lock().unwrap();
    if attempt_id.as_ref().is_some_and(|id| {
        !snapshot
            .as_ref()
            .is_some_and(|progress| &progress.attempt_id == id)
    }) {
        return Err("That speech setup attempt has already ended; refresh its status.".into());
    }
    if !SETUP_ACTIVE.load(Ordering::SeqCst) {
        return Ok(());
    }
    if snapshot
        .as_ref()
        .is_some_and(|progress| progress.phase == "activating")
    {
        return Err(
            "The verified speech runtime is being activated. Wait for setup to finish.".into(),
        );
    }
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
    let _operation = ctx
        .session_operation
        .try_lock()
        .map_err(|_| "Finish the current operation before removing the speech runtime.")?;
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
    let freed = dir_size(&app_dir.join(RUNTIME_DIR_NAME))
        + dir_size(&app_dir.join(BASE_DIR_NAME))
        + dir_size(&app_dir.join("python-generations"));
    // Stop the sidecar first: unload kills the child process and waits for
    // it to exit — on Windows the interpreter's exes/DLLs are locked while
    // the process lives, so deleting first would fail or half-delete.
    ctx.asr_handle
        .shutdown_blocking(std::time::Duration::from_secs(10))
        .map_err(|error| format!("Could not stop the speech runtime: {error}"))?;
    remove_managed_dir(&app_dir.join(RUNTIME_DIR_NAME))?;
    remove_managed_dir(&app_dir.join(BASE_DIR_NAME))?;
    remove_managed_dir(&app_dir.join("python-generations"))?;
    if app_dir.join("python-active").exists() {
        std::fs::remove_file(app_dir.join("python-active")).map_err(|e| e.to_string())?;
    }
    Ok(freed)
}

/// The variant this machine would install — the worker's actual decision,
/// shared with `runtime_status` so the UI can describe the right package set.
/// `REFLOW_TORCH_VARIANT` forces the wheel flavor (testing, debugging).
pub(crate) fn resolved_torch_variant_for(caps: &crate::capability::Capabilities) -> TorchVariant {
    match std::env::var("REFLOW_TORCH_VARIANT").as_deref() {
        Ok("cu130") => TorchVariant::Cuda,
        Ok("cu126") => TorchVariant::CudaLegacy,
        Ok("cpu") => TorchVariant::Cpu,
        Ok("default") | Ok("pypi") => TorchVariant::Pypi,
        _ => super::setup_plan::compatible_variant(target_triple(), caps),
    }
}

/// Large CUDA wheels need more patience than pip's 15-second socket timeout.
/// Keep completed downloads outside the disposable venv so retrying setup can
/// reuse them. Only enable download resume when the venv's pip supports it.
fn pip_install_args(cache_dir: &Path, supports_resume: bool) -> Vec<String> {
    let mut args: Vec<String> = [
        "-m",
        "pip",
        "install",
        "--disable-pip-version-check",
        "--no-input",
        "--only-binary=:all:",
        "--timeout",
        "120",
        "--retries",
        "8",
        "--cache-dir",
    ]
    .into_iter()
    .map(str::to_string)
    .chain(std::iter::once(cache_dir.to_string_lossy().into_owned()))
    .collect();
    if supports_resume {
        args.extend([
            "--resume-retries".to_string(),
            "8".to_string(),
            "--progress-bar".into(),
            "raw".into(),
        ]);
    }
    args
}

fn package_progress_detail(line: &str) -> Option<String> {
    let (downloaded, total) = line.strip_prefix("Progress ")?.split_once(" of ")?;
    let downloaded: u64 = downloaded.parse().ok()?;
    let total: u64 = total.parse().ok()?;
    if total == 0 {
        Some(format!(
            "Downloading a speech package: {:.1} MB received",
            downloaded as f64 / 1e6
        ))
    } else {
        Some(format!(
            "Downloading a speech package: {:.1} of {:.1} MB",
            downloaded.min(total) as f64 / 1e6,
            total as f64 / 1e6
        ))
    }
}

fn setup_command_error(phase: &str, status: std::process::ExitStatus, detail: &str) -> String {
    log::error!("Speech setup ({phase}) failed with {status}:\n{detail}");
    if detail.contains("ReadTimeoutError") || detail.contains("Read timed out") {
        return "The speech dependency download timed out. Check your internet connection, VPN, or proxy, then retry setup. Completed downloads are cached for the next attempt.".into();
    }
    if detail.contains("No space left") || detail.contains("not enough space") {
        return "There is not enough free storage to finish speech setup. Free some space and retry.".into();
    }
    if detail.contains("No matching distribution") || detail.contains("Could not find a version") {
        return "Compatible speech packages could not be found for this system. Check for a Reflow update, then retry setup. Technical details are in Open logs.".into();
    }
    if phase == "verifying" {
        return "The installed speech libraries did not pass their check. Retry dependency setup. Technical details are in Open logs.".into();
    }
    "Speech dependencies could not be installed. Check your internet connection and retry setup. Technical details are in Open logs.".into()
}

/// The actual install. `emit` receives (phase, progress_pct, detail, error)
/// event fields; keeping it a closure means the worker is testable without a
/// running Tauri app.
type SetupEmit<'a> = &'a dyn Fn(&str, Option<u32>, Option<String>, Option<String>);

#[cfg(all(test, target_os = "linux"))]
pub(crate) fn run_setup_worker(emit: SetupEmit, resource_dir: Option<&Path>) -> Result<(), String> {
    run_setup_worker_for(emit, resource_dir, "0.6b", "auto")
}

fn run_setup_worker_for(
    emit: SetupEmit,
    _resource_dir: Option<&Path>,
    model: &str,
    device: &str,
) -> Result<(), String> {
    run_setup_worker_from_pack(emit, model, device, None)
}

fn run_setup_worker_from_pack(
    emit: SetupEmit,
    model: &str,
    device: &str,
    pack_override: Option<super::runtime_pack::RuntimePackPin>,
) -> Result<(), String> {
    cancelled()?;

    let app_dir = PlatformSys::get_app_dir();
    std::fs::create_dir_all(&app_dir)
        .map_err(|e| format!("Could not create app data directory: {e}"))?;
    #[cfg(windows)]
    check_long_paths(&app_dir)?;

    let variant = variant_for(model, device, &crate::capability::capabilities_uncached());
    super::setup_plan::platform_compatibility(variant)?;
    if device == "cuda" && !matches!(variant, TorchVariant::Cuda | TorchVariant::CudaLegacy) {
        return Err("GPU only requires a supported NVIDIA GPU and driver. Choose Auto or CPU, or update your driver.".into());
    }
    let locked = super::dependency_locks::locked_requirements(target_triple(), dependency_profile(model), variant.receipt_name()).ok_or("No verified binary dependency set for this platform and device. Select Zipformer 20M or CPU.")?;
    let pack = match pack_override {
        Some(pack) => Some(pack),
        None => super::runtime_pack::selected(
            target_triple(),
            dependency_profile(model),
            variant.receipt_name(),
        )?,
    };
    if pack.as_ref().is_some_and(|pack| {
        pack.target != target_triple()
            || pack.profile != dependency_profile(model)
            || pack.variant != variant.receipt_name()
    }) {
        return Err("The runtime pack does not match the selected speech configuration.".into());
    }
    if pack.is_none() {
        crate::network_policy::check_download()?;
    }
    let estimate = super::setup_plan::peak_disk_estimate(variant, model).saturating_add(
        pack.as_ref()
            .map_or(0, |pack| pack.unpacked_bytes.saturating_add(pack.bytes)),
    );
    crate::network_policy::check_disk_space(&app_dir, estimate)?;

    let pin = pin_for(target_triple()).ok_or_else(|| {
        format!(
            "This platform ({}) has no pinned Python build.",
            target_triple()
        )
    })?;

    // Release builds fetch one complete reviewed pack. The index-based path is
    // retained for development builds with the intentionally empty catalog.
    let cache_dir = PlatformSys::get_cache_dir();
    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("Could not create download cache dir: {e}"))?;
    crate::network_policy::check_disk_space(&cache_dir, estimate)?;
    let archive = if let Some(pack) = &pack {
        let archive = cache_dir.join(&pack.asset);
        let downloads: Vec<_> = if pack.parts.is_empty() {
            vec![(&pack.asset, &pack.url, &pack.sha256, pack.bytes)]
        } else {
            pack.parts
                .iter()
                .map(|part| (&part.asset, &part.url, &part.sha256, part.bytes))
                .collect()
        };
        let mut completed = 0u64;
        for (asset, url, sha256, bytes) in downloads {
            let destination = cache_dir.join(asset);
            let partial = cache_dir.join(format!("{asset}.partial"));
            crate::network_policy::fetch_verified_blocking(
                crate::network_policy::ArtifactDownload {
                    url,
                    sha256,
                    exact_bytes: Some(bytes),
                    partial: &partial,
                    destination: &destination,
                },
                &SETUP_CANCEL,
                |progress| {
                    emit(
                        if progress.phase == "downloading" {
                            "downloading"
                        } else {
                            "verifying"
                        },
                        Some(
                            ((completed + progress.bytes.min(bytes)) as u128 * 100
                                / pack.bytes.max(1) as u128)
                                .min(100) as u32,
                        ),
                        Some("Preparing the speech runtime pack".into()),
                        None,
                    )
                },
            )
            .map_err(|error| {
                if SETUP_CANCEL.load(Ordering::Acquire) {
                    CANCELLED.into()
                } else {
                    error
                }
            })?;
            completed += bytes;
        }
        archive
    } else {
        let partial = cache_dir.join(format!("{}.partial", pin.asset));
        let archive = cache_dir.join(pin.asset);
        download_archive(emit, pin, &partial, &archive)?;
        archive
    };
    cancelled()?;

    // New generation paths are final from birth; only the tiny pointer changes
    // after verification. Failed/interrupted generations never shadow the old one.
    let generation = uuid::Uuid::new_v4().to_string();
    let generation_dir = app_dir.join("python-generations").join(&generation);
    let base_dir = generation_dir.join("base");
    let runtime_dir = generation_dir.join("runtime");
    let payload_dir = generation_dir.join("payload");
    std::fs::create_dir_all(&base_dir).map_err(|e| e.to_string())?;
    let _cleanup = GenerationCleanup(generation_dir.clone(), app_dir.clone());
    emit("extracting", None, None, None);
    if let Some(pack) = &pack {
        super::runtime_pack::unpack(pack, &archive, &payload_dir, &SETUP_CANCEL)?;
        extract_archive(&payload_dir.join("bootstrap").join(pin.asset), &base_dir)?;
    } else {
        extract_archive(&archive, &base_dir)?;
    }
    cancelled()?;
    emit("creating_env", None, None, None);
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
        false,
    )?;
    cancelled()?;

    let venv_python = runtime_dir.join(if cfg!(windows) {
        "Scripts\\python.exe"
    } else {
        "bin/python"
    });
    // Install the pinned pip before the selected speech packages.
    emit(
        "creating_env",
        None,
        Some("Preparing speech libraries".into()),
        None,
    );
    let mut bootstrap_args = pip_install_args(&cache_dir.join("pip"), false);
    let bootstrap_path = generation_dir.join("pip-requirements.txt");
    std::fs::write(
        &bootstrap_path,
        include_bytes!("../../../model-runtime/locks/pip.txt"),
    )
    .map_err(|e| e.to_string())?;
    bootstrap_args.extend([
        "--upgrade".into(),
        "--require-hashes".into(),
        "-r".into(),
        bootstrap_path.to_string_lossy().into_owned(),
    ]);
    if pack.is_some() {
        bootstrap_args.extend(local_wheel_args(&payload_dir));
    }
    run_setup_command(
        emit,
        "creating_env",
        &venv_python,
        &bootstrap_args,
        pack.is_none(),
    )?;
    let mut install_args = pip_install_args(&cache_dir.join("pip"), true);
    if pack.is_some() {
        install_args.extend(local_wheel_args(&payload_dir));
    }
    cancelled()?;

    // Resolve nothing at installation time: exact versions and artifact hashes
    // for the selected target/profile/device are part of the application build.
    let selected_path = generation_dir.join("requirements.txt");
    std::fs::write(&selected_path, locked).map_err(|e| e.to_string())?;
    let mut args = install_args;
    args.extend([
        "--require-hashes".into(),
        "--report".into(),
        generation_dir
            .join("package-report.json")
            .to_string_lossy()
            .into_owned(),
        "-r".into(),
        selected_path.to_string_lossy().into_owned(),
    ]);
    if pack.is_none() {
        args.extend(["--index-url".into(), "https://pypi.org/simple".into()]);
        if let Some(index) = variant.index_url() {
            args.extend(["--extra-index-url".into(), index.into()]);
        }
    }
    emit(
        "installing_packages",
        None,
        Some(format!(
            "Installing verified {} packages ({})",
            dependency_profile(model),
            variant.receipt_name()
        )),
        None,
    );
    run_setup_command(
        emit,
        "installing_packages",
        &venv_python,
        &args,
        pack.is_none(),
    )?;
    cancelled()?;

    // Stage 6: verify the modules the applicable models probe at startup.
    emit("verifying", None, None, None);
    let verify_imports = verification_code(
        dependency_profile(model),
        variant,
        matches!(variant, TorchVariant::Cuda | TorchVariant::CudaLegacy),
    );
    run_setup_command(
        emit,
        "verifying",
        &venv_python,
        &["-c".to_string(), verify_imports],
        false,
    )?;
    cancelled()?;
    run_setup_command(
        emit,
        "verifying",
        &venv_python,
        &["-m".into(), "pip".into(), "check".into()],
        false,
    )?;
    let python_version = super::sidecar::Qwen3AsrSidecar::bounded_python_probe_for(
        &venv_python.to_string_lossy(),
        "import platform; print(platform.python_version())",
        Duration::from_secs(3),
    )
    .and_then(|bytes| String::from_utf8(bytes).ok())
    .map(|version| version.trim().to_string())
    .ok_or("The speech interpreter version check failed. Retry dependency setup.")?;
    if python_version != python_builds().python_version {
        return Err(
            "The speech interpreter does not match its reviewed version. Retry dependency setup."
                .into(),
        );
    }
    let receipt = RuntimeReceipt {
        pbs_tag: python_builds().pbs_tag.to_string(),
        python_version,
        torch_variant: variant.receipt_name().to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        target: target_triple().into(),
        profile: dependency_profile(model).into(),
        requirements_digest: requirements_digest(dependency_profile(model), variant),
        runtime_pack_sha256: pack.as_ref().map(|pack| pack.sha256.clone()),
    };
    let bytes = serde_json::to_vec_pretty(&receipt)
        .map_err(|e| format!("Could not serialize runtime receipt: {e}"))?;
    let receipt_path = runtime_dir.join(RECEIPT_NAME);
    let mut receipt_file = std::fs::File::create(&receipt_path).map_err(|e| e.to_string())?;
    receipt_file.write_all(&bytes).map_err(|e| e.to_string())?;
    receipt_file.sync_all().map_err(|e| e.to_string())?;
    if pack.is_some() {
        std::fs::copy(
            payload_dir.join("licenses.json"),
            runtime_dir.join("reflow-licenses.json"),
        )
        .map_err(|e| e.to_string())?;
        std::fs::remove_dir_all(&payload_dir).map_err(|e| e.to_string())?;
    }

    begin_activation(emit)?;
    promote_generation(&app_dir, &generation)?;
    Ok(())
}

fn local_wheel_args(payload: &Path) -> [String; 3] {
    [
        "--no-index".into(),
        "--find-links".into(),
        payload.join("wheelhouse").to_string_lossy().into_owned(),
    ]
}

struct GenerationCleanup(PathBuf, PathBuf);
impl Drop for GenerationCleanup {
    fn drop(&mut self) {
        if managed_runtime_dir(&self.1) != self.0.join("runtime") {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
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

/// Small stdout capture used for pip capabilities and the receipt's Python version.
#[cfg(all(test, target_os = "linux"))]
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

fn setup_panic_error(panic: Box<dyn std::any::Any + Send>) -> String {
    let reason = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("unknown internal error");
    log::error!("Speech runtime setup worker panicked: {reason}");
    format!("Speech runtime setup stopped unexpectedly: {reason}. Try setup again.")
}

fn download_archive(
    emit: SetupEmit,
    pin: &PythonBuildPin,
    partial: &Path,
    archive: &Path,
) -> Result<(), String> {
    cancelled()?;
    crate::network_policy::fetch_verified_blocking(
        crate::network_policy::ArtifactDownload {
            url: pin.url,
            sha256: pin.sha256,
            exact_bytes: Some(pin.approx_bytes),
            partial,
            destination: archive,
        },
        &SETUP_CANCEL,
        |progress| {
            let phase = if progress.phase == "downloading" {
                "downloading"
            } else {
                "verifying"
            };
            emit(
                phase,
                progress.total.map(|total| {
                    (progress.bytes as u128 * 100 / total.max(1) as u128).min(100) as u32
                }),
                None,
                None,
            );
        },
    )
    .map_err(|error| {
        if SETUP_CANCEL.load(Ordering::Acquire) {
            CANCELLED.into()
        } else {
            error
        }
    })
}

/// Unpack the whole `python/` tree. tar's unpack refuses `..` components and
/// preserves the symlinks CPython relies on (bin/python3 → python3.12).
fn extract_archive(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| format!("Open tar.gz: {e}"))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    for entry in tar.entries().map_err(|e| e.to_string())? {
        cancelled()?;
        entry
            .map_err(|e| e.to_string())?
            .unpack_in(dest)
            .map_err(|e| format!("Extract Python archive: {e}"))?;
    }
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

/// Poll output independently of process lifetime. A child can close both output
/// streams and still hang, or exit while a descendant keeps a pipe open. A
/// bounded spool avoids either case leaving blocked reader threads behind.
pub(super) struct SetupOutput(pub(super) PathBuf);
impl Drop for SetupOutput {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub(super) struct SetupChild<'a>(pub(super) &'a Mutex<Option<Child>>);
impl Drop for SetupChild<'_> {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.lock().unwrap().take() {
            kill_setup_tree(&mut child);
        }
    }
}

fn wait_setup_command(
    emit: SetupEmit,
    phase: &str,
    output: &mut std::fs::File,
    slot: &Mutex<Option<Child>>,
    timeout: Duration,
    interrupt: &dyn Fn() -> Result<(), String>,
) -> Result<(), String> {
    let _child = SetupChild(slot);
    let started = Instant::now();
    let mut last_emit = Instant::now();
    let mut output_tail = std::collections::VecDeque::new();
    let mut buffer = [0u8; 32768];
    loop {
        interrupt()?;
        if started.elapsed() >= timeout {
            return Err("Speech dependency setup timed out. Retry setup; the working runtime has been preserved.".into());
        }
        let status = slot
            .lock()
            .unwrap()
            .as_mut()
            .ok_or("Setup process was unexpectedly taken over")?
            .try_wait()
            .map_err(|e| format!("Could not check setup process: {e}"))?;
        let size = output.metadata().map_err(|e| e.to_string())?.len();
        if size > 32 * 1024 * 1024 {
            return Err("Speech setup produced too much diagnostic output. Retry setup; technical details are in Open logs.".into());
        }
        // Keep the final diagnostics even when a short-lived child writes more
        // than one polling buffer. Never collect an unbounded line or queue.
        if status.is_some() {
            let offset = output.stream_position().map_err(|e| e.to_string())?;
            output
                .seek(std::io::SeekFrom::Start(
                    offset.max(size.saturating_sub(buffer.len() as u64)),
                ))
                .map_err(|e| e.to_string())?;
        }
        let count = output.read(&mut buffer).map_err(|e| e.to_string())?;
        for line in String::from_utf8_lossy(&buffer[..count]).lines() {
            if output_tail.len() == 12 {
                output_tail.pop_front();
            }
            output_tail.push_back(line.chars().take(4096).collect::<String>());
        }
        if count > 0 && last_emit.elapsed() >= Duration::from_millis(250) {
            if let Some(line) = output_tail.back() {
                emit(
                    phase,
                    None,
                    Some(package_progress_detail(line).unwrap_or_else(|| line.clone())),
                    None,
                );
            }
            last_emit = Instant::now();
        }
        if let Some(status) = status {
            interrupt()?;
            if !status.success() {
                let detail = output_tail.into_iter().collect::<Vec<_>>().join("\n");
                #[cfg(test)]
                eprintln!("Setup child failure ({phase}): {detail}");
                return Err(setup_command_error(phase, status, &detail));
            }
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Spawn an owned child and stream bounded diagnostic output to the UI.
fn run_setup_command(
    emit: SetupEmit,
    phase: &str,
    program: &Path,
    args: &[String],
    network_required: bool,
) -> Result<(), String> {
    cancelled()?;
    let mut cmd = Command::new(program);
    // Restore the host environment before clearing Python overrides: AppImage
    // restoration must not reintroduce a caller's PYTHONHOME into managed Python.
    #[cfg(target_os = "linux")]
    crate::platform::process::own_child(&mut cmd);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("PIP_") {
            cmd.env_remove(key);
        }
    }
    // Restoration can add keys absent from the app's current environment.
    cmd.env_remove("PYTHONPATH")
        .env_remove("PYTHONHOME")
        .env_remove("VIRTUAL_ENV");
    let temporary = PlatformSys::get_cache_dir().join("pip-temp");
    std::fs::create_dir_all(&temporary).map_err(|e| e.to_string())?;
    cmd.env(
        "PIP_CONFIG_FILE",
        if cfg!(windows) { "NUL" } else { "/dev/null" },
    )
    .env("PIP_INDEX_URL", "https://pypi.org/simple")
    .env("TMPDIR", &temporary)
    .env("TMP", &temporary)
    .env("TEMP", &temporary)
    .env("PYTHONNOUSERSITE", "1")
    .env("HF_HUB_OFFLINE", "1")
    .env("TRANSFORMERS_OFFLINE", "1");
    if args.starts_with(&["-m".to_string(), "pip".to_string()]) {
        cmd.arg("-c")
            .arg(include_str!("../../../model-runtime/managed_pip.py"))
            .args(&args[2..]);
    } else {
        cmd.args(args);
    }
    cmd.env(
        "REFLOW_NETWORK_POLICY",
        PlatformSys::get_app_dir().join("network-policy.json"),
    );
    let output_path = temporary.join(format!("setup-{}.log", uuid::Uuid::new_v4()));
    let _output_cleanup = SetupOutput(output_path.clone());
    let mut options = std::fs::OpenOptions::new();
    options.append(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let output = options.open(&output_path).map_err(|e| e.to_string())?;
    let mut reader = std::fs::File::open(&output_path).map_err(|e| e.to_string())?;
    cmd.stdin(Stdio::null())
        .stdout(output.try_clone().map_err(|e| e.to_string())?)
        .stderr(output);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    #[cfg(unix)]
    {
        // Own process group: cancelling or quitting can then kill pip and any
        // children it spawned (console subprocesses) with one group signal.
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let child = cmd
        .spawn()
        .map_err(|e| format!("Could not start {}: {e}", program.display()))?;
    // Release the parent's output handles before removing the spool on Windows.
    drop(cmd);
    {
        // Cancel can land between spawn and registration — re-check the flag
        // while still holding the slot lock and reap the just-spawned child.
        let mut slot = SETUP_CHILD.lock().unwrap();
        *slot = Some(child);
        if SETUP_CANCEL.load(Ordering::SeqCst) {
            if let Some(running) = slot.as_mut() {
                kill_setup_tree(running);
            }
        }
    }

    wait_setup_command(
        emit,
        phase,
        &mut reader,
        &SETUP_CHILD,
        Duration::from_secs(
            if phase == "installing_packages" || phase == "installing_torch" {
                1800
            } else {
                120
            },
        ),
        &|| {
            cancelled()?;
            if network_required {
                crate::network_policy::check_download().map_err(|reason| {
                    format!("Downloads stopped during setup: {reason} Setup cancelled.")
                })
            } else {
                Ok(())
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn setup_process_fixture(
        script: &str,
        timeout: Duration,
        stop_when_ready: Option<&str>,
    ) -> Result<(), String> {
        use std::os::unix::process::CommandExt;
        let root =
            std::env::temp_dir().join(format!("reflow-setup-process-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let output = std::fs::OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(root.join("output"))
            .unwrap();
        let mut reader = std::fs::File::open(root.join("output")).unwrap();
        let mut command = Command::new("sh");
        command
            .args(["-c", script])
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(output)
            .process_group(0);
        let child = command.spawn().unwrap();
        drop(command);
        let slot = Mutex::new(Some(child));
        let result = wait_setup_command(
            &|_, _, _, _| {},
            "installing_packages",
            &mut reader,
            &slot,
            timeout,
            &|| {
                if root.join("ready").exists() {
                    if let Some(reason) = stop_when_ready {
                        return Err(reason.into());
                    }
                }
                Ok(())
            },
        );
        assert!(
            slot.lock().unwrap().is_none(),
            "the child must be reaped on every return path"
        );
        #[cfg(target_os = "linux")]
        if let Ok(pid) = std::fs::read_to_string(root.join("background.pid")) {
            let stat = PathBuf::from("/proc").join(pid.trim()).join("stat");
            let deadline = Instant::now() + Duration::from_secs(1);
            loop {
                let running = std::fs::read_to_string(&stat).is_ok_and(|s| {
                    !s.rsplit_once(") ")
                        .is_some_and(|(_, state)| state.starts_with('Z'))
                });
                if !running {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "owned descendant is still running"
                );
                std::thread::yield_now();
            }
        }
        drop(reader);
        std::fs::remove_dir_all(root).unwrap();
        result
    }

    #[cfg(unix)]
    #[test]
    fn closed_output_cannot_bypass_setup_deadline_or_interrupts() {
        for reason in [None, Some(CANCELLED), Some("Offline mode is on")] {
            let started = Instant::now();
            let error = setup_process_fixture(
                "touch ready; exec 1>&- 2>&-; exec sleep 30",
                Duration::from_millis(300),
                reason,
            )
            .unwrap_err();
            assert!(error.contains(reason.unwrap_or("timed out")), "{error}");
            assert!(started.elapsed() < Duration::from_secs(2));
        }
    }

    #[cfg(unix)]
    #[test]
    fn inherited_output_does_not_hold_up_a_finished_setup_process() {
        let started = Instant::now();
        setup_process_fixture(
            "sleep 30 & echo $! > background.pid; exit 0",
            Duration::from_secs(2),
            None,
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[cfg(unix)]
    #[test]
    fn setup_output_is_bounded_and_failure_keeps_recovery_diagnostics() {
        let error = setup_process_fixture(
            "truncate -s 33554433 output; exec sleep 30",
            Duration::from_secs(2),
            None,
        )
        .unwrap_err();
        assert!(error.contains("too much diagnostic output"));
        let error = setup_process_fixture(
            "printf 'Read timed out\\n' >&2; exit 7",
            Duration::from_secs(2),
            None,
        )
        .unwrap_err();
        assert!(error.contains("download timed out"));
    }

    #[test]
    fn health_checks_require_quantization_packages_only_for_quantized_qwen() {
        for (profile, variant, quantized) in [
            ("qwen", TorchVariant::Cpu, false),
            ("qwen", TorchVariant::Pypi, false),
            ("qwen", TorchVariant::Cuda, false),
            ("phonon", TorchVariant::Cuda, true),
            ("zipformer", TorchVariant::None_, false),
        ] {
            let check = verification_code(profile, variant, quantized);
            assert!(!check.contains("torchao"));
            assert!(!check.contains("accelerate"));
        }
        let quantized = verification_code("qwen", TorchVariant::Cuda, true);
        assert!(quantized.contains("import torchao, accelerate"));
        assert!(quantized.contains("torch.cuda.synchronize()"));
        assert!(verification_code("phonon", TorchVariant::Cpu, false).contains("torch.ones(1)"));
    }

    #[test]
    fn setup_claims_capture_configuration_and_stale_work_cannot_cancel_or_finish_a_retry() {
        let first = claim_setup("zipformer-20m", "cpu").unwrap();
        assert!(claim_setup("1.7b", "cuda").is_err());
        let initial = get_python_setup_progress().unwrap();
        assert_eq!(initial.model, "zipformer-20m");
        assert_eq!(initial.device, "cpu");
        let first_terminal = update_progress(&first, "complete", Some(100), None, None).unwrap();
        assert!(first_terminal.revision > initial.revision);
        let next = claim_setup("0.6b", "cpu").unwrap();
        assert_ne!(first, next);
        assert!(get_python_setup_progress().unwrap().revision > first_terminal.revision);
        assert!(cancel_python_runtime_setup(Some(first.clone())).is_err());
        assert!(!SETUP_CANCEL.load(Ordering::SeqCst));
        assert!(
            update_progress(&first, "error", None, None, Some("late failure".into())).is_none()
        );
        assert!(SETUP_ACTIVE.load(Ordering::SeqCst));
        let current = update_progress(&next, "complete", Some(100), None, None).unwrap();
        assert_eq!(current.attempt_id, next);
        assert!(!SETUP_ACTIVE.load(Ordering::SeqCst));

        let cancelled_attempt = claim_setup("zipformer-20m", "cpu").unwrap();
        cancel_python_runtime_setup(Some(cancelled_attempt.clone())).unwrap();
        assert_eq!(begin_activation(&|_, _, _, _| {}).unwrap_err(), CANCELLED);
        update_progress(&cancelled_attempt, "cancelled", None, None, None).unwrap();
        let committing = claim_setup("zipformer-20m", "cpu").unwrap();
        begin_activation(&|_, _, _, _| {}).unwrap();
        assert_eq!(
            SETUP_PROGRESS.lock().unwrap().as_ref().unwrap().phase,
            "activating"
        );
        assert!(cancel_python_runtime_setup(Some(committing.clone()))
            .unwrap_err()
            .contains("being activated"));
        assert!(!SETUP_CANCEL.load(Ordering::SeqCst));
        let terminal = update_progress(&committing, "complete", Some(100), None, None).unwrap();
        assert!(update_progress(&committing, "verifying", None, None, None).is_none());
        assert!(update_progress(
            &committing,
            "error",
            None,
            None,
            Some("late failure".into())
        )
        .is_none());
        let retained = get_python_setup_progress().unwrap();
        assert_eq!(retained.phase, "complete");
        assert_eq!(retained.revision, terminal.revision);
        cancel_python_runtime_setup(Some(committing)).unwrap();
        assert!(!SETUP_CANCEL.load(Ordering::SeqCst));
    }

    #[test]
    fn cached_generation_requires_matching_receipt_and_live_verification() {
        let root = std::env::temp_dir().join(format!("reflow-cached-{}", uuid::Uuid::new_v4()));
        let id = uuid::Uuid::new_v4().to_string();
        let runtime = root.join("python-generations").join(&id).join("runtime");
        std::fs::create_dir_all(&runtime).unwrap();
        let receipt = serde_json::json!({
            "pbs_tag": python_builds().pbs_tag, "python_version": python_builds().python_version,
            "torch_variant": "none", "created_at": "fixture", "target": target_triple(),
            "profile": "zipformer", "requirements_digest": requirements_digest("zipformer", TorchVariant::None_)
        });
        std::fs::write(
            runtime.join(RECEIPT_NAME),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        assert!(find_cached_generation(&root, "qwen", TorchVariant::Cpu, |_| true).is_none());
        assert!(
            find_cached_generation(&root, "zipformer", TorchVariant::None_, |_| false).is_none()
        );
        assert_eq!(
            find_cached_generation(&root, "zipformer", TorchVariant::None_, |_| true),
            Some((id, runtime))
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_and_mismatched_receipts_require_repair() {
        let legacy = serde_json::json!({"pbs_tag": python_builds().pbs_tag, "python_version": python_builds().python_version,
            "torch_variant": "none", "created_at": "fixture"});
        let mut receipt: RuntimeReceipt = serde_json::from_value(legacy).unwrap();
        assert!(!receipt_matches(&receipt, "zipformer", TorchVariant::None_));
        receipt.target = target_triple().into();
        receipt.profile = "zipformer".into();
        receipt.requirements_digest = requirements_digest("zipformer", TorchVariant::None_);
        assert!(receipt_matches(&receipt, "zipformer", TorchVariant::None_));
        assert!(!receipt_matches(&receipt, "qwen", TorchVariant::Cpu));
        receipt.target = "different-target".into();
        assert!(!receipt_matches(&receipt, "zipformer", TorchVariant::None_));
        receipt.target = target_triple().into();
        receipt.requirements_digest = "previous-requirements".into();
        assert!(!receipt_matches(&receipt, "zipformer", TorchVariant::None_));
    }

    #[test]
    fn failed_or_interrupted_generation_preserves_effective_base_and_runtime() {
        let root =
            std::env::temp_dir().join(format!("reflow-generations-{}", uuid::Uuid::new_v4()));
        let old = uuid::Uuid::new_v4().to_string();
        let old_root = root.join("python-generations").join(&old);
        std::fs::create_dir_all(old_root.join("runtime")).unwrap();
        std::fs::write(old_root.join("base"), b"original interpreter").unwrap();
        promote_generation(&root, &old).unwrap();
        let failed = root
            .join("python-generations")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(failed.join("runtime")).unwrap();
        {
            let _cleanup = GenerationCleanup(failed.clone(), root.clone());
        }
        assert!(!failed.exists());
        assert_eq!(managed_runtime_dir(&root), old_root.join("runtime"));
        assert_eq!(
            std::fs::read(old_root.join("base")).unwrap(),
            b"original interpreter"
        );
        // An interrupted pointer write is ignored on restart.
        std::fs::write(root.join("python-active-interrupted.tmp"), b"partial").unwrap();
        assert_eq!(managed_runtime_dir(&root), old_root.join("runtime"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn lightweight_and_cpu_preferences_select_dependency_profiles() {
        let caps = crate::capability::Capabilities::default();
        assert_eq!(dependency_profile("zipformer-20m"), "zipformer");
        assert_eq!(
            variant_for("zipformer-20m", "auto", &caps),
            TorchVariant::None_
        );
        assert_eq!(
            variant_for("0.6b", "cpu", &caps),
            torch_variant(target_triple(), false)
        );
        assert_ne!(
            requirements_digest("qwen", TorchVariant::Cpu),
            requirements_digest("zipformer", TorchVariant::None_)
        );
    }

    #[cfg(unix)]
    #[test]
    fn pip_timeout_reports_recovery_without_a_traceback() {
        use std::os::unix::process::ExitStatusExt;
        let detail = "Traceback (most recent call last):\npip._vendor.urllib3.exceptions.ReadTimeoutError: HTTPSConnectionPool(host='pypi.nvidia.com', port=443): Read timed out.";
        let error = setup_command_error(
            "installing_torch",
            std::process::ExitStatus::from_raw(2 << 8),
            detail,
        );
        assert!(error.contains("download timed out"));
        assert!(error.contains("retry setup"));
        assert!(!error.contains("Traceback"));
        assert!(!error.contains("pip._vendor"));
        // Verification diagnostics are logged; the user sees a recovery action.
        let error = setup_command_error(
            "verifying",
            std::process::ExitStatus::from_raw(1 << 8),
            "ImportError: missing library",
        );
        assert!(error.contains("Retry dependency setup"));
        assert!(error.contains("Open logs"));
        assert!(!error.contains("ImportError"));
    }

    #[test]
    fn pip_progress_is_readable_and_scoped_to_each_package() {
        assert_eq!(
            package_progress_detail("Progress 85000000 of 366000000").as_deref(),
            Some("Downloading a speech package: 85.0 of 366.0 MB")
        );
        assert!(package_progress_detail("Progress unknown of 366000000").is_none());
        assert!(package_progress_detail("Collecting torch").is_none());
        assert_eq!(
            package_progress_detail("Progress 2000000 of 0").as_deref(),
            Some("Downloading a speech package: 2.0 MB received")
        );
    }

    #[test]
    fn pip_download_policy_allows_slow_downloads_and_caches_outside_venv() {
        let cache = Path::new("app-cache").join("pip");
        for supports_resume in [false, true] {
            let args = pip_install_args(&cache, supports_resume);
            for (option, value) in [("--timeout", "120"), ("--retries", "8")] {
                assert!(args.windows(2).any(|pair| pair == [option, value]));
            }
            assert!(args
                .windows(2)
                .any(|pair| pair[0] == "--cache-dir" && pair[1] == cache.to_string_lossy()));
            assert!(!args.iter().any(|arg| arg == "--no-cache-dir"));
            assert_eq!(
                args.windows(2)
                    .any(|pair| pair == ["--resume-retries", "8"]),
                supports_resume
            );
        }
    }

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

        #[test]
        #[ignore = "local model-free pack acceptance; set REFLOW_TEST_RUNTIME_PACK_CATALOG and run this test alone"]
        fn cached_runtime_pack_installs_offline_and_failed_repairs_preserve_it() {
            let catalog_path = PathBuf::from(
                std::env::var("REFLOW_TEST_RUNTIME_PACK_CATALOG").expect("provide a local catalog"),
            );
            let profile = std::env::var("REFLOW_TEST_RUNTIME_PACK_PROFILE")
                .unwrap_or_else(|_| "zipformer".into());
            let (model, variant) = match profile.as_str() {
                "zipformer" => ("zipformer-20m", TorchVariant::None_),
                "qwen" => ("0.6b", TorchVariant::Cpu),
                "phonon" => ("phonon-2", TorchVariant::Cpu),
                _ => panic!("provide a supported local CPU runtime-pack profile"),
            };
            let catalog = std::fs::read_to_string(&catalog_path).unwrap();
            let pin = crate::asr::runtime_pack::selected_from(
                &catalog,
                target_triple(),
                &profile,
                variant.receipt_name(),
            )
            .unwrap()
            .unwrap();
            let assets: Vec<_> = if pin.parts.is_empty() {
                vec![pin.asset.as_str()]
            } else {
                pin.parts.iter().map(|part| part.asset.as_str()).collect()
            };
            let source = catalog_path.parent().unwrap().join(assets[0]);
            let app_dir = scratch_app_dir();
            crate::network_policy::configure(true, &app_dir).unwrap();
            let cache = PlatformSys::get_cache_dir();
            std::fs::create_dir_all(&cache).unwrap();
            for asset in &assets {
                std::fs::copy(
                    catalog_path.parent().unwrap().join(asset),
                    cache.join(asset),
                )
                .unwrap();
            }
            let cached = cache.join(assets[0]);
            let started = Instant::now();
            run_setup_worker_from_pack(&|_, _, _, _| {}, model, "cpu", Some(pin.clone())).unwrap();
            let install_ms = started.elapsed().as_millis();
            let incumbent = managed_runtime_dir(&app_dir);
            let receipt = read_receipt(&incumbent).unwrap();
            assert_eq!(
                receipt.runtime_pack_sha256.as_deref(),
                Some(pin.sha256.as_str())
            );
            assert!(receipt_matches(&receipt, &profile, variant));
            assert!(incumbent.join("reflow-licenses.json").is_file());
            assert!(!incumbent.parent().unwrap().join("payload").exists());
            assert!(!app_dir.join("models").exists());
            let python = incumbent.join("bin/python");
            let health = crate::asr::sidecar::Qwen3AsrSidecar::bounded_python_probe_for(
                &python.to_string_lossy(),
                &format!(
                    "{}; print('ready')",
                    verification_code(&profile, variant, false)
                ),
                Duration::from_secs(30),
            )
            .expect("installed dependency pack must pass the production Python probe");
            assert!(String::from_utf8_lossy(&health).contains("ready"));

            let error = run_setup_worker_from_pack(
                &|phase, _, _, _| {
                    if phase == "extracting" {
                        SETUP_CANCEL.store(true, Ordering::Release);
                    }
                },
                model,
                "cpu",
                Some(pin.clone()),
            )
            .unwrap_err();
            assert_eq!(error, CANCELLED);
            SETUP_CANCEL.store(false, Ordering::Release);
            assert_eq!(managed_runtime_dir(&app_dir), incumbent);
            // A bad or missing cache must not attempt an online fallback in
            // airplane mode, promote a generation or remove the working one.
            std::fs::write(&cached, b"corrupt").unwrap();
            for present in [true, false] {
                if !present {
                    std::fs::remove_file(&cached).unwrap();
                }
                let error =
                    run_setup_worker_from_pack(&|_, _, _, _| {}, model, "cpu", Some(pin.clone()))
                        .unwrap_err();
                assert!(error.contains("Offline mode"), "{error}");
                assert_eq!(managed_runtime_dir(&app_dir), incumbent);
            }
            std::fs::copy(&source, &cached).unwrap();
            let lazy_module = match profile.as_str() {
                "qwen" => Some("transformers.models.qwen3_asr.processing_qwen3_asr"),
                "phonon" => Some("transformers.models.parakeet.modeling_parakeet"),
                _ => None,
            };
            if let Some(lazy_module) = lazy_module {
                // Break only this disposable runtime. Top-level imports still
                // work, but the concrete processor needed by ASR must fail.
                let module = crate::asr::sidecar::Qwen3AsrSidecar::bounded_python_probe_for(
                    &python.to_string_lossy(),
                    &format!("import importlib.util; print('REFLOW_ASR_MODULE:'+importlib.util.find_spec({lazy_module:?}).origin)"),
                    Duration::from_secs(30),
                ).unwrap();
                let module = String::from_utf8_lossy(&module);
                let module = PathBuf::from(
                    module
                        .lines()
                        .find_map(|line| line.strip_prefix("REFLOW_ASR_MODULE:"))
                        .unwrap(),
                );
                assert!(module
                    .canonicalize()
                    .unwrap()
                    .starts_with(incumbent.canonicalize().unwrap()));
                let original = std::fs::read_to_string(&module).unwrap();
                // Keep definitions for Transformers' lazy export discovery.
                std::fs::write(
                    module,
                    format!("{original}\nraise ImportError('Reflow fixture: lazy ASR API is unavailable')\n"),
                )
                .unwrap();
                assert!(
                    crate::asr::sidecar::Qwen3AsrSidecar::bounded_python_probe_for(
                        &python.to_string_lossy(),
                        "import torch, transformers; assert torch.ones(1).sum().item() == 1",
                        Duration::from_secs(30),
                    )
                    .is_some()
                );
                let check = verification_code(&profile, variant, false);
                let diagnostic = format!("try:\n exec({check:?})\n print('unexpected-ready')\nexcept ImportError:\n import sys, traceback\n traceback.print_exc(file=sys.stdout)");
                let rejected = crate::asr::sidecar::Qwen3AsrSidecar::bounded_python_probe_for(
                    &python.to_string_lossy(),
                    &diagnostic,
                    Duration::from_secs(30),
                )
                .unwrap();
                assert!(String::from_utf8_lossy(&rejected)
                    .contains("Reflow fixture: lazy ASR API is unavailable"));
            }
            run_setup_worker_from_pack(&|_, _, _, _| {}, model, "cpu", Some(pin.clone())).unwrap();
            assert_ne!(managed_runtime_dir(&app_dir), incumbent);
            assert!(
                incumbent.join(RECEIPT_NAME).is_file(),
                "repair retains the incumbent generation"
            );
            let journal = app_dir.join("network-journal.jsonl");
            assert!(!journal.exists() || std::fs::metadata(journal).unwrap().len() == 0);
            let keep = std::env::var("REFLOW_TEST_RUNTIME_PACK_KEEP").as_deref() == Ok("1");
            println!(
                "REFLOW_PACK_ACCEPTANCE:{}",
                serde_json::json!({"target":target_triple(),"profile":format!("{profile}-{}",variant.receipt_name()),
                "broken_lazy_api_rejected":lazy_module.is_some(),
                "retained_app_dir":if keep {Some(&app_dir)} else {None},
                "retained_python":if keep {Some(managed_runtime_dir(&app_dir).join("bin/python"))} else {None},
                "pack_sha256":pin.sha256,"bytes":pin.bytes,"offline_install_ms":install_ms,
                "cancelled_repair":"pass","corrupt_cache":"pass","missing_cache":"pass","offline_repair":"pass",
                "network_journal_entries":0,"model_downloads":0,"native_desktop_or_asr_acceptance":false})
            );
            std::env::remove_var("REFLOW_APP_DIR");
            if !keep {
                let _ = std::fs::remove_dir_all(app_dir);
            }
        }

        fn scratch_app_dir() -> PathBuf {
            // Every acceptance run owns its application data. A previously verified
            // archive may be copied in to avoid repeating the bootstrap transfer.
            let dir =
                std::env::temp_dir().join(format!("reflow-python-e2e-{}", uuid::Uuid::new_v4()));
            std::env::set_var("REFLOW_APP_DIR", &dir);
            crate::network_policy::configure(false, &dir).expect("go online");
            if let Ok(archive) = std::env::var("REFLOW_TEST_PYTHON_ARCHIVE") {
                let pin = pin_for(target_triple()).expect("supported target");
                let cache = PlatformSys::get_cache_dir();
                std::fs::create_dir_all(&cache).unwrap();
                std::fs::copy(archive, cache.join(pin.asset)).unwrap();
            }
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
                false,
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
        #[ignore = "real network: installs a complete fresh lightweight profile in an isolated directory"]
        fn managed_zipformer_clean_install_and_cancelled_repair() {
            let app_dir = scratch_app_dir();
            SETUP_CANCEL.store(false, Ordering::SeqCst);
            let emit =
                |phase: &str, _pct: Option<u32>, detail: Option<String>, _err: Option<String>| {
                    eprintln!("{phase}: {}", detail.unwrap_or_default());
                };
            run_setup_worker_for(&emit, None, "zipformer-20m", "cpu")
                .expect("fresh lightweight setup");
            let old = managed_runtime_dir(&app_dir);
            let python = old.join("bin/python");
            assert!(verify_installed_versions(
                &python,
                "zipformer",
                TorchVariant::None_
            ));
            let output = pip_probe(&python, &["-c", "import importlib.util; assert importlib.util.find_spec('torch') is None; assert importlib.util.find_spec('transformers') is None; import numpy, sherpa_onnx; print('lightweight ready')"]);
            assert_eq!(output.as_deref(), Some("lightweight ready"));
            let cancel_at_verification =
                |phase: &str, _pct: Option<u32>, _detail: Option<String>, _err: Option<String>| {
                    if phase == "verifying" {
                        SETUP_CANCEL.store(true, Ordering::SeqCst);
                    }
                };
            let error = run_setup_worker_for(&cancel_at_verification, None, "zipformer-20m", "cpu")
                .unwrap_err();
            assert_eq!(error, CANCELLED);
            assert_eq!(managed_runtime_dir(&app_dir), old);
            assert!(verify_installed_versions(
                &python,
                "zipformer",
                TorchVariant::None_
            ));
            SETUP_CANCEL.store(false, Ordering::SeqCst);
            std::env::remove_var("REFLOW_APP_DIR");
            eprintln!("Isolated acceptance directory: {}", app_dir.display());
        }

        #[test]
        #[ignore = "downloads ~2 GB of speech packages; run with -- --ignored"]
        fn managed_setup_end_to_end() {
            let app_dir = scratch_app_dir();
            // Keep the run CPU-sized regardless of the test machine's GPU.
            let test_variant = std::env::var("REFLOW_TEST_TORCH_VARIANT").unwrap_or("cpu".into());
            assert!(["cpu", "cu130", "cu126"].contains(&test_variant.as_str()));
            std::env::set_var("REFLOW_TORCH_VARIANT", &test_variant);
            let emits = std::sync::Mutex::new(Vec::<String>::new());
            let emit =
                |phase: &str, _pct: Option<u32>, detail: Option<String>, _err: Option<String>| {
                    eprintln!("{phase}: {}", detail.unwrap_or_default());
                    emits.lock().unwrap().push(phase.to_string());
                };
            run_setup_worker(&emit, None).expect("full setup");
            let runtime = managed_runtime_dir(&app_dir);
            assert!(runtime.join("bin/python").exists());
            let receipt = read_receipt(&runtime).expect("receipt written");
            assert_eq!(receipt.pbs_tag, python_builds().pbs_tag);
            assert_eq!(receipt.torch_variant, test_variant);
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
