//! Persistent Linux launcher paths and Desktop Entry Exec escaping.

use std::path::{Path, PathBuf};

pub(crate) fn launcher_executable(
    appimage: Option<&Path>,
    current_exe: &Path,
) -> Result<PathBuf, String> {
    // AppImage executables live under a temporary FUSE mount. Only the original
    // image remains available after logout or an application restart.
    let executable = appimage
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(current_exe);
    if !executable.is_absolute() {
        return Err("The Linux launcher requires an absolute executable path".into());
    }
    Ok(executable.to_path_buf())
}

pub(crate) fn current_launcher_executable() -> Result<PathBuf, String> {
    let current = std::env::current_exe().map_err(|error| error.to_string())?;
    let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
    launcher_executable(appimage.as_deref(), &current)
}

pub(crate) fn desktop_exec(executable: &Path) -> Result<String, String> {
    let executable = executable
        .to_str()
        .ok_or_else(|| "The executable path is not valid UTF-8".to_owned())?;
    if executable.is_empty() || executable.contains(['\n', '\r', '\t', '\0', '=']) {
        return Err("The executable path cannot be represented by a Linux desktop launcher".into());
    }
    // Desktop files unescape string values before unquoting Exec arguments.
    // Escape both layers, then escape literal percent field codes.
    let escaped = executable
        .replace('\\', "\\\\\\\\")
        .replace('"', "\\\\\"")
        .replace('`', "\\\\`")
        .replace('$', "\\\\$")
        .replace('%', "%%");
    // GLib checks that the executable exists before expanding field codes.
    // A literal '%' in its path would be checked as '%%'. Let env resolve
    // that path as an argument after field-code expansion instead.
    let command = if executable.contains('%') {
        "/usr/bin/env -- "
    } else {
        ""
    };
    Ok(format!("{command}\"{escaped}\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appimage_launcher_survives_the_temporary_mount_disappearing() {
        assert_eq!(
            launcher_executable(
                Some(Path::new("/home/user/My Apps/Reflow.AppImage")),
                Path::new("/tmp/.mount_Reflow123/usr/bin/reflow"),
            )
            .unwrap(),
            Path::new("/home/user/My Apps/Reflow.AppImage")
        );
        for appimage in [None, Some(Path::new(""))] {
            assert_eq!(
                launcher_executable(appimage, Path::new("/usr/bin/reflow")).unwrap(),
                Path::new("/usr/bin/reflow")
            );
        }
        assert!(launcher_executable(
            Some(Path::new("Reflow.AppImage")),
            Path::new("/usr/bin/reflow")
        )
        .is_err());
    }

    #[test]
    fn exec_paths_escape_both_desktop_string_and_argument_layers() {
        assert_eq!(
            desktop_exec(Path::new("/home/user/My Apps/Reflow.AppImage")).unwrap(),
            "\"/home/user/My Apps/Reflow.AppImage\""
        );
        assert_eq!(
            desktop_exec(Path::new(r#"/home/user/a\b"c$d`e%f/Reflow"#)).unwrap(),
            r#"/usr/bin/env -- "/home/user/a\\\\b\\"c\\$d\\`e%%f/Reflow""#
        );
        for path in [
            "",
            "/tmp/new\nline",
            "/tmp/carriage\rreturn",
            "/tmp/a\tb",
            "/tmp/a\0b",
            "/tmp/a=b",
        ] {
            assert!(desktop_exec(Path::new(path)).is_err(), "accepted {path:?}");
        }
    }
}
