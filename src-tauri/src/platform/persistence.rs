//! Atomic visibility for small policy and active-generation documents.
//! This does not claim power-loss durability of an entire installed runtime.
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with(path, |file| {
        file.write_all(bytes)?;
        file.sync_all()
    })
}

fn write_atomic_with(
    path: &Path,
    write_and_flush: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    struct Pending(PathBuf);
    impl Drop for Pending {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let temporary =
        Pending(path.with_file_name(format!(".reflow-write-{}.tmp", uuid::Uuid::new_v4())));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary.0)?;
    write_and_flush(&mut file)?;
    drop(file);
    replace(&temporary.0, path)
}

#[cfg(windows)]
fn replace(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        // Replace + write-through. Never delete the incumbent as a fallback.
        if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 1 | 8) } != 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(5 | 32 | 33)) && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        } else {
            return Err(error);
        }
    }
}

#[cfg(not(windows))]
fn replace(source: &Path, target: &Path) -> io::Result<()> {
    std::fs::rename(source, target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_writes_flushes_and_replacement_preserve_the_incumbent() {
        let root = std::env::temp_dir().join(format!("reflow-atomic-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("active");
        write_atomic(&path, b"incumbent").unwrap();
        for partial in [true, false] {
            let failure = write_atomic_with(&path, |file| {
                file.write_all(if partial { b"par" } else { b"replacement" })?;
                Err(io::Error::other(if partial {
                    "injected storage-full write failure"
                } else {
                    "injected flush failure"
                }))
            });
            assert!(failure.is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"incumbent");
            assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        }
        // Real filesystem failures: a destination directory and a parent file.
        let blocked = root.join("blocked");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(blocked.join("keep"), b"untouched").unwrap();
        assert!(write_atomic(&blocked, b"replacement").is_err());
        assert_eq!(std::fs::read(blocked.join("keep")).unwrap(), b"untouched");
        assert!(write_atomic(&path.join("child"), b"replacement").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"incumbent");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        write_atomic(&path, b"replacement").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_locked_windows_destination_is_retained_and_retry_recovers() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = std::env::temp_dir().join(format!("reflow-atomic-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("active");
        write_atomic(&path, b"incumbent").unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let started = std::time::Instant::now();
        assert!(write_atomic(&path, b"replacement").is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(4));
        drop(lock);
        assert_eq!(std::fs::read(&path).unwrap(), b"incumbent");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        write_atomic(&path, b"replacement").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        std::fs::remove_dir_all(root).unwrap();
    }
}
