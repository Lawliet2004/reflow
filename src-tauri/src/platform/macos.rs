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
        if !accessibility_trusted() {
            return Err(
                "Reflow needs Accessibility access to paste into other apps. Enable Reflow in System Settings → Privacy & Security → Accessibility, then try again."
                    .into(),
            );
        }
        use enigo::{
            Direction::{Click, Press, Release},
            Enigo, Key, Keyboard,
        };
        let mut enigo =
            Enigo::new(&enigo::Settings::default()).map_err(|e| e.to_string())?;
        enigo
            .key(Key::Meta, Press)
            .map_err(|e| e.to_string())?;
        let paste = enigo
            .key(Key::Unicode('v'), Click)
            .map_err(|e| e.to_string());
        let release = enigo.key(Key::Meta, Release).map_err(|e| e.to_string());
        paste.and(release)
    }

    fn open_path(path: &Path) -> Result<(), String> {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Failed to open folder: {e}"))
    }
}

/// Accessibility-trust check for synthetic keystrokes. The prompting variant
/// (`kAXTrustedCheckOptionPrompt: true`) is used only once per process —
/// otherwise every failed paste would re-raise the system dialog mid-dictation.
#[cfg(target_os = "macos")]
fn accessibility_trusted() -> bool {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        static kAXTrustedCheckOptionPrompt: *const c_void;
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFBooleanTrue: *const c_void;
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            num_values: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
    }

    static PROMPTED: AtomicBool = AtomicBool::new(false);
    unsafe {
        if !PROMPTED.swap(true, Ordering::SeqCst) {
            let keys = [kAXTrustedCheckOptionPrompt];
            let values = [kCFBooleanTrue];
            let options = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                std::ptr::null(),
                std::ptr::null(),
            );
            let trusted = AXIsProcessTrustedWithOptions(options);
            if !options.is_null() {
                CFRelease(options);
            }
            return trusted;
        }
        AXIsProcessTrusted()
    }
}
