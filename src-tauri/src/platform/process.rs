//! Bounded, reaped desktop probes. A broken server must not stall dictation.
use std::io::{ErrorKind, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Linux kills these owned runtime children even after a crash/forced exit.
pub fn own_child(command: &mut Command) {
    host_environment(command);
    let parent = unsafe { libc::getpid() };
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The parent may have exited between fork and prctl.
            if libc::getppid() != parent {
                libc::_exit(1);
            }
            Ok(())
        });
    }
}

fn appimage_environment(
    appdir: &std::path::Path,
    get: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Vec<(&'static str, Option<std::ffi::OsString>)> {
    let mut changes = Vec::new();
    for name in [
        "PYTHONHOME",
        "PYTHONPATH",
        "LD_LIBRARY_PATH",
        "GDK_BACKEND",
        "GTK_PATH",
        "GTK_DATA_PREFIX",
        "GTK_EXE_PREFIX",
        "GIO_EXTRA_MODULES",
        "GI_TYPELIB_PATH",
        "GSETTINGS_SCHEMA_DIR",
        "GTK_IM_MODULE_FILE",
        "GDK_PIXBUF_MODULE_FILE",
        "GST_PLUGIN_SYSTEM_PATH",
        "GST_PLUGIN_SYSTEM_PATH_1_0",
    ] {
        if let Some(original) = get(&format!("REFLOW_APPIMAGE_ORIGINAL_{name}")) {
            changes.push((name, (original != "__REFLOW_UNSET__").then_some(original)));
            continue;
        }
        let Some(value) = get(name) else {
            continue;
        };
        let kept: Vec<_> = std::env::split_paths(&value)
            .filter(|path| !path.starts_with(appdir))
            .collect();
        if kept.len() == std::env::split_paths(&value).count() {
            continue;
        }
        changes.push((
            name,
            if kept.is_empty() {
                None
            } else {
                std::env::join_paths(kept).ok()
            },
        ));
    }
    changes
}

/// The GUI needs bundled GTK libraries; host Python/PipeWire tools need their
/// own standard library and dependencies. Strip only AppImage-added paths and
/// restore explicit caller overrides captured by our launch hooks.
pub(crate) fn host_environment(command: &mut Command) {
    let Some(appdir) = std::env::var_os("APPDIR") else {
        return;
    };
    for (name, value) in
        appimage_environment(std::path::Path::new(&appdir), |name| std::env::var_os(name))
    {
        if let Some(value) = value {
            command.env(name, value);
        } else {
            command.env_remove(name);
        }
    }
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        // process_group(0) gives this command its own group. Also close any
        // descendants retaining its output pipe; never signal the app's group.
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn output(
    program: &str,
    args: &[&str],
    timeout: Duration,
    limit: usize,
) -> Result<String, String> {
    output_env(program, args, timeout, limit, &[])
}

pub fn output_env(
    program: &str,
    args: &[&str],
    timeout: Duration,
    limit: usize,
    env: &[(&str, &str)],
) -> Result<String, String> {
    let mut command = Command::new(program);
    host_environment(&mut command);
    command.envs(env.iter().copied());
    let mut child = OwnedChild(
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|error| format!("Cannot run {program}: {error}"))?,
    );
    let mut stdout = child.0.stdout.take().ok_or("Missing command output")?;
    // Nonblocking reads allow the same deadline to cover both the command and
    // its output. No detached reader threads survive a wedged desktop probe.
    let fd = stdout.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err("Cannot configure command output".into());
    }
    let deadline = Instant::now() + timeout;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut eof = false;
    loop {
        if Instant::now() >= deadline {
            return Err(format!("{program} timed out"));
        }
        if !eof {
            match stdout.read(&mut buffer) {
                Ok(0) => eof = true,
                Ok(count) => {
                    if bytes.len() + count > limit {
                        return Err(format!("{program} output exceeded its limit"));
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                    continue;
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        if let Some(status) = child.0.try_wait().map_err(|error| error.to_string())? {
            if !status.success() {
                return Err(format!("{program} exited with {status}"));
            }
            if eof {
                return String::from_utf8(bytes).map_err(|error| error.to_string());
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appimage_host_workers_keep_user_paths_and_remove_bundled_python_home() {
        let environment = std::collections::HashMap::from([
            ("PYTHONHOME", "/image/usr"),
            (
                "PYTHONPATH",
                "/image/usr/share/pyshared:/chosen/python-modules",
            ),
            ("LD_LIBRARY_PATH", "/image/usr/lib:/chosen/cuda"),
            ("GSETTINGS_SCHEMA_DIR", "/image/usr/share/glib-2.0/schemas"),
        ]);
        let changes = appimage_environment(std::path::Path::new("/image"), |name| {
            environment.get(name).map(Into::into)
        });
        assert!(changes.contains(&("PYTHONHOME", None)));
        assert!(changes.contains(&("GSETTINGS_SCHEMA_DIR", None)));
        assert!(changes.contains(&("PYTHONPATH", Some("/chosen/python-modules".into()))));
        assert!(changes.contains(&("LD_LIBRARY_PATH", Some("/chosen/cuda".into()))));
        let originals = std::collections::HashMap::from([
            ("REFLOW_APPIMAGE_ORIGINAL_PYTHONHOME", "/chosen/python"),
            ("REFLOW_APPIMAGE_ORIGINAL_GDK_BACKEND", "x11"),
            (
                "REFLOW_APPIMAGE_ORIGINAL_LD_LIBRARY_PATH",
                "__REFLOW_UNSET__",
            ),
        ]);
        let changes = appimage_environment(std::path::Path::new("/image"), |name| {
            originals.get(name).map(Into::into)
        });
        assert!(changes.contains(&("PYTHONHOME", Some("/chosen/python".into()))));
        assert!(changes.contains(&("GDK_BACKEND", Some("x11".into()))));
        assert!(changes.contains(&("LD_LIBRARY_PATH", None)));
    }
    #[test]
    fn hung_commands_and_inherited_output_pipes_are_bounded() {
        for script in ["sleep 10", "sleep 10 & exit 0"] {
            let started = Instant::now();
            assert!(
                output("sh", &["-c", script], Duration::from_millis(60), 1024)
                    .unwrap_err()
                    .contains("timed out")
            );
            assert!(started.elapsed() < Duration::from_secs(1));
        }
    }
    #[test]
    fn output_and_exit_status_are_checked() {
        assert_eq!(
            output("printf", &["hello"], Duration::from_secs(1), 5).unwrap(),
            "hello"
        );
        assert!(output("printf", &["overflow"], Duration::from_secs(1), 3).is_err());
        assert!(output("sh", &["-c", "exit 7"], Duration::from_secs(1), 10).is_err());
    }
}
