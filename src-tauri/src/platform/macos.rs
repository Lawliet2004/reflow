use std::path::Path;

use super::adapter::{DisplaySession, PlatformAdapter};

pub struct MacOsAdapter;

impl PlatformAdapter for MacOsAdapter {
    fn session() -> DisplaySession {
        DisplaySession::Macos
    }

    fn os_display_name() -> String {
        "macOS".into()
    }

    fn simulate_paste(_process: &str) -> Result<(), String> {
        Err("macOS text injection is not implemented in this build".into())
    }

    fn open_path(path: &Path) -> Result<(), String> {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Failed to open folder: {e}"))
    }
}
