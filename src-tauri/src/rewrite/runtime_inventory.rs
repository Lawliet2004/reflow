//! Immutable runtime generations and an atomic current/previous pointer.
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

const POINTER: &str = "runtime-active.json";
const MANIFEST: &str = "runtime-manifest.json";
static PENDING: AtomicBool = AtomicBool::new(false);
static GATE: OnceLock<RwLock<()>> = OnceLock::new();

pub fn launch_guard() -> Result<RwLockReadGuard<'static, ()>, String> {
    if PENDING.load(Ordering::Acquire) {
        return Err("Runtime update is in progress; retry shortly.".into());
    }
    let guard = GATE.get_or_init(|| RwLock::new(())).read();
    if PENDING.load(Ordering::Acquire) {
        return Err("Runtime update is in progress; retry shortly.".into());
    }
    Ok(guard)
}

/// Set pending before stopping the ASR actor: new launch requests must fail
/// promptly, rather than park behind a write lock that an actor unload needs.
pub struct Maintenance;
impl Maintenance {
    pub fn begin() -> Result<Self, String> {
        PENDING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "Another runtime update is in progress.".to_string())?;
        Ok(Self)
    }
    pub fn exclusive(&self) -> RwLockWriteGuard<'static, ()> {
        GATE.get_or_init(|| RwLock::new(())).write()
    }
}
impl Drop for Maintenance {
    fn drop(&mut self) {
        PENDING.store(false, Ordering::Release);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeFile {
    pub filename: String,
    pub sha256: String,
    pub bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeManifest {
    pub id: String,
    pub version: String,
    pub kind: String,
    pub asset: String,
    pub archive_sha256: String,
    pub binary: String,
    pub files: Vec<RuntimeFile>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimePointer {
    pub current: String,
    pub previous: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct RuntimeEntry {
    pub id: String,
    pub version: String,
    pub kind: String,
    pub binary_path: String,
    pub active: bool,
    pub rollback_available: bool,
    pub healthy: bool,
    pub error: Option<String>,
}

pub(crate) fn safe_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err("Invalid runtime inventory identifier or filename".into());
    }
    Ok(())
}
pub fn generation_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    safe_name(id)?;
    Ok(root.join("runtimes").join(id))
}
pub fn binary_name() -> &'static str {
    if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}
pub fn read_pointer(root: &Path) -> Result<Option<RuntimePointer>, String> {
    let path = root.join(POINTER);
    if !path.exists() {
        return Ok(None);
    }
    let pointer: RuntimePointer =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("Invalid runtime pointer: {e}"))?;
    safe_name(&pointer.current)?;
    if let Some(previous) = &pointer.previous {
        safe_name(previous)?;
    }
    Ok(Some(pointer))
}
pub fn read_manifest(root: &Path, id: &str) -> Result<RuntimeManifest, String> {
    if id == "legacy" {
        return Ok(RuntimeManifest {
            id: id.into(),
            version: "legacy".into(),
            kind: std::fs::read_to_string(root.join("llama-server.kind"))
                .unwrap_or_else(|_| "Unknown".into())
                .trim()
                .into(),
            asset: String::new(),
            archive_sha256: String::new(),
            binary: binary_name().into(),
            files: vec![],
        });
    }
    let manifest: RuntimeManifest = serde_json::from_slice(
        &std::fs::read(generation_dir(root, id)?.join(MANIFEST)).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("Invalid runtime manifest: {e}"))?;
    if manifest.id != id || manifest.files.is_empty() {
        return Err("Runtime manifest identity/files are invalid".into());
    }
    safe_name(&manifest.binary)?;
    for file in &manifest.files {
        safe_name(&file.filename)?;
    }
    if !manifest.files.iter().any(|f| f.filename == manifest.binary) {
        return Err("Runtime manifest has no launcher".into());
    }
    Ok(manifest)
}
pub fn entry_binary(root: &Path, manifest: &RuntimeManifest) -> Result<PathBuf, String> {
    if manifest.id == "legacy" {
        Ok(root.join(binary_name()))
    } else {
        Ok(generation_dir(root, &manifest.id)?.join(&manifest.binary))
    }
}
pub fn active_manifest(root: &Path) -> Option<RuntimeManifest> {
    match read_pointer(root) {
        Ok(Some(pointer)) => read_manifest(root, &pointer.current).ok(),
        Ok(None) => None,
        Err(error) => {
            log::warn!("{error}");
            None
        }
    }
}
pub fn active_binary(root: &Path) -> Option<PathBuf> {
    active_manifest(root).and_then(|m| entry_binary(root, &m).ok())
}
pub fn verify_generation(root: &Path, id: &str) -> Result<RuntimeManifest, String> {
    let manifest = read_manifest(root, id)?;
    let binary = entry_binary(root, &manifest)?;
    if !binary.is_file() {
        return Err("Runtime launcher is missing".into());
    }
    if id != "legacy" {
        for file in &manifest.files {
            let path = generation_dir(root, id)?.join(&file.filename);
            if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() != file.bytes {
                return Err(format!("Runtime file {} changed size", file.filename));
            }
            super::runtime_install::verify_sha256(&path, &file.sha256)?;
        }
    }
    Ok(manifest)
}
pub fn write_manifest(root: &Path, manifest: &RuntimeManifest) -> Result<(), String> {
    let dir = generation_dir(root, &manifest.id)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join(MANIFEST),
        serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

pub fn write_pointer(root: &Path, pointer: &RuntimePointer) -> Result<(), String> {
    use std::io::Write;
    safe_name(&pointer.current)?;
    if let Some(previous) = &pointer.previous {
        safe_name(previous)?;
    }
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let temporary = root.join(format!(".runtime-pointer-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec_pretty(pointer).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        atomic_replace(&temporary, &root.join(POINTER))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

#[cfg(windows)]
fn atomic_replace(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    // Both names are NUL-terminated and valid for the duration of the call.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 1 | 8) } != 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        // Antivirus and transient file readers can deny replacement. Retrying
        // preserves the previous pointer; deleting it would break atomicity.
        if matches!(error.raw_os_error(), Some(5 | 32 | 33)) && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
            continue;
        }
        return Err(format!("Could not atomically activate runtime: {error}"));
    }
}
#[cfg(not(windows))]
fn atomic_replace(source: &Path, target: &Path) -> Result<(), String> {
    std::fs::rename(source, target)
        .map_err(|e| format!("Could not atomically activate runtime: {e}"))
}

pub fn promote(root: &Path, id: &str) -> Result<(), String> {
    verify_generation(root, id)?;
    let current = read_pointer(root)?
        .map(|p| p.current)
        .or_else(|| root.join(binary_name()).is_file().then(|| "legacy".into()));
    write_pointer(
        root,
        &RuntimePointer {
            current: id.into(),
            previous: current.filter(|previous| previous != id),
        },
    )
}
pub fn rollback_target(root: &Path) -> Result<RuntimeManifest, String> {
    let previous = read_pointer(root)?
        .and_then(|p| p.previous)
        .ok_or("No previous runtime is available for rollback")?;
    verify_generation(root, &previous)
}
pub fn rollback(root: &Path) -> Result<(), String> {
    let previous = rollback_target(root)?;
    let current = read_pointer(root)?
        .ok_or("No active runtime pointer")?
        .current;
    write_pointer(
        root,
        &RuntimePointer {
            current: previous.id,
            previous: Some(current),
        },
    )
}
pub fn inventory(root: &Path) -> Vec<RuntimeEntry> {
    let pointer = read_pointer(root).ok().flatten();
    let mut ids = vec![];
    if root.join(binary_name()).is_file() {
        ids.push("legacy".to_string());
    }
    if let Ok(dirs) = std::fs::read_dir(root.join("runtimes")) {
        ids.extend(
            dirs.filter_map(Result::ok)
                .filter(|d| d.path().is_dir())
                .filter_map(|d| d.file_name().into_string().ok())
                .filter(|name| !name.starts_with('.')),
        );
    }
    ids.sort();
    ids.into_iter()
        .map(|id| {
            let manifest = read_manifest(root, &id);
            let error = verify_generation(root, &id).err();
            RuntimeEntry {
                active: pointer
                    .as_ref()
                    .map(|p| p.current == id)
                    .unwrap_or(id == "legacy"),
                rollback_available: pointer
                    .as_ref()
                    .is_some_and(|p| p.previous.as_ref() == Some(&id)),
                version: manifest
                    .as_ref()
                    .map(|m| m.version.clone())
                    .unwrap_or_default(),
                kind: manifest
                    .as_ref()
                    .map(|m| m.kind.clone())
                    .unwrap_or_else(|_| "Unknown".into()),
                binary_path: manifest
                    .as_ref()
                    .ok()
                    .and_then(|m| entry_binary(root, m).ok())
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                healthy: error.is_none(),
                error,
                id,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    struct TestRoot(PathBuf);
    impl TestRoot {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("reflow-runtime-inventory-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn generation(&self, id: &str, content: &[u8]) {
            let dir = generation_dir(&self.0, id).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(binary_name()), content).unwrap();
            write_manifest(
                &self.0,
                &RuntimeManifest {
                    id: id.into(),
                    version: "test".into(),
                    kind: "CPU".into(),
                    asset: "fixture.zip".into(),
                    archive_sha256: "0".repeat(64),
                    binary: binary_name().into(),
                    files: vec![RuntimeFile {
                        filename: binary_name().into(),
                        bytes: content.len() as u64,
                        sha256: format!("{:x}", Sha256::digest(content)),
                    }],
                },
            )
            .unwrap();
        }
    }
    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn corrupt_staged_runtime_preserves_the_active_pointer_and_files() {
        let root = TestRoot::new();
        root.generation("old", b"healthy-old");
        root.generation("new", b"healthy-new");
        promote(&root.0, "old").unwrap();
        let before = std::fs::read(root.0.join(POINTER)).unwrap();
        std::fs::write(
            generation_dir(&root.0, "new").unwrap().join(binary_name()),
            b"corrupt-new",
        )
        .unwrap();
        assert!(promote(&root.0, "new").is_err());
        assert_eq!(std::fs::read(root.0.join(POINTER)).unwrap(), before);
        assert_eq!(
            std::fs::read(active_binary(&root.0).unwrap()).unwrap(),
            b"healthy-old"
        );
    }
    #[test]
    fn promotion_and_rollback_retain_both_immutable_generations() {
        let root = TestRoot::new();
        root.generation("old", b"old");
        root.generation("new", b"new");
        promote(&root.0, "old").unwrap();
        promote(&root.0, "new").unwrap();
        assert_eq!(
            read_pointer(&root.0).unwrap().unwrap(),
            RuntimePointer {
                current: "new".into(),
                previous: Some("old".into())
            }
        );
        rollback(&root.0).unwrap();
        assert_eq!(
            read_pointer(&root.0).unwrap().unwrap(),
            RuntimePointer {
                current: "old".into(),
                previous: Some("new".into())
            }
        );
        assert!(verify_generation(&root.0, "new").is_ok());
        let entries = inventory(&root.0);
        assert!(entries
            .iter()
            .any(|entry| entry.id == "old" && entry.active && entry.healthy));
        assert!(entries
            .iter()
            .any(|entry| entry.id == "new" && entry.rollback_available && entry.healthy));
    }
    #[test]
    fn first_managed_promotion_can_roll_back_to_the_legacy_launcher() {
        let root = TestRoot::new();
        std::fs::write(root.0.join(binary_name()), b"legacy").unwrap();
        std::fs::write(root.0.join("llama-server.kind"), "Vulkan\n").unwrap();
        root.generation("new", b"new");
        promote(&root.0, "new").unwrap();
        rollback(&root.0).unwrap();
        assert_eq!(active_binary(&root.0).unwrap(), root.0.join(binary_name()));
        assert_eq!(active_manifest(&root.0).unwrap().kind, "Vulkan");
    }
    #[test]
    fn pointer_names_cannot_escape_the_inventory_root() {
        let root = TestRoot::new();
        for name in ["../escape", "..", "C:\\escape", "/escape"] {
            assert!(generation_dir(&root.0, name).is_err());
            assert!(write_pointer(
                &root.0,
                &RuntimePointer {
                    current: name.into(),
                    previous: None
                }
            )
            .is_err());
        }
        assert!(!root.0.join(POINTER).exists());
    }
    #[test]
    fn replacing_a_pointer_never_exposes_partial_json_to_readers() {
        let root = TestRoot::new();
        write_pointer(
            &root.0,
            &RuntimePointer {
                current: "old".into(),
                previous: None,
            },
        )
        .unwrap();
        let path = root.0.clone();
        let reader = std::thread::spawn(move || {
            for _ in 0..200 {
                let pointer = read_pointer(&path).unwrap().unwrap();
                assert!(pointer.current == "old" || pointer.current == "new");
            }
        });
        for index in 0..20 {
            write_pointer(
                &root.0,
                &RuntimePointer {
                    current: if index % 2 == 0 { "new" } else { "old" }.into(),
                    previous: None,
                },
            )
            .unwrap();
        }
        reader.join().unwrap();
    }
}
