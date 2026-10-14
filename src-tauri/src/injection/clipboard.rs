//! Clipboard access used by injection. Windows preserves complete supported
//! HGLOBAL payloads, rather than reducing images or rich text to plain text.
pub(super) const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
#[cfg(any(windows, test))]
pub(super) struct FormatData {
    format: u32,
    bytes: Vec<u8>,
}

/// Non-memory clipboard formats cannot be cloned by copying their handles.
/// Windows can synthesize CF_DIB/DIBV5 from CF_BITMAP, so the DIB is sufficient.
#[cfg(any(windows, test))]
fn copyable_format(format: u32, formats: &[u32]) -> Result<bool, String> {
    match format {
        2 | 9 if formats.contains(&8) || formats.contains(&17) => Ok(false),
        1 | 4..=8 | 10..=13 | 15..=17 | 0x300..=0x3ff | 0xc000..=0xffff => Ok(true),
        _ => Err(format!(
            "Clipboard format {format} cannot be safely preserved. Your clipboard was left unchanged. Disable clipboard restoration to replace it."
        )),
    }
}

#[cfg(any(windows, test))]
fn copy_formats(
    formats: &[u32],
    byte_limit: usize,
    mut read: impl FnMut(u32, usize) -> Result<Vec<u8>, String>,
) -> Result<Vec<FormatData>, String> {
    let mut snapshot = Vec::new();
    let mut remaining = byte_limit;
    for &format in formats {
        if !copyable_format(format, formats)? {
            continue;
        }
        let bytes = read(format, remaining)?;
        if bytes.is_empty() || bytes.len() > remaining {
            return Err(
                "Clipboard payload exceeds the restoration limit. Clipboard unchanged.".into(),
            );
        }
        remaining -= bytes.len();
        snapshot.push(FormatData { format, bytes });
    }
    Ok(snapshot)
}

#[cfg(windows)]
pub(super) use native::{ClipboardAccess, ClipboardSnapshot};

#[cfg(windows)]
mod native {
    use super::*;
    use windows::core::w;
    use windows::Win32::Foundation::{
        GetLastError, GlobalFree, SetLastError, ERROR_SUCCESS, HANDLE, HGLOBAL, HWND,
    };
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
        GetClipboardSequenceNumber, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
    };

    #[derive(Clone, Default)]
    pub(crate) struct ClipboardSnapshot(std::sync::Arc<Vec<FormatData>>);

    /// All comparisons and writes happen while the OS clipboard is locked.
    pub(crate) struct ClipboardAccess {
        owner: HWND,
    }

    impl ClipboardAccess {
        pub(crate) fn open() -> Result<Self, String> {
            unsafe {
                // An invisible, message-only window makes this thread the
                // clipboard owner without changing foreground focus.
                let owner = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    w!("Reflow clipboard"),
                    WINDOW_STYLE::default(),
                    0,
                    0,
                    0,
                    0,
                    HWND_MESSAGE,
                    None,
                    None,
                    None,
                )
                .map_err(|e| format!("Could not create clipboard owner: {e}"))?;
                let mut error = None;
                for attempt in 0..5 {
                    match OpenClipboard(owner) {
                        Ok(()) => return Ok(Self { owner }),
                        Err(err) => error = Some(err),
                    }
                    if attempt < 4 {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                }
                let _ = DestroyWindow(owner);
                Err(format!("Clipboard is busy: {}", error.unwrap()))
            }
        }

        pub(crate) fn sequence(&self) -> Option<u32> {
            Some(unsafe { GetClipboardSequenceNumber() })
        }

        pub(crate) fn text(&self) -> Option<String> {
            unsafe {
                let handle = GetClipboardData(13).ok()?;
                let global = HGLOBAL(handle.0);
                let len = GlobalSize(global);
                if !(2..=MAX_SNAPSHOT_BYTES).contains(&len) {
                    return None;
                }
                let ptr = GlobalLock(global) as *const u16;
                if ptr.is_null() {
                    return None;
                }
                let units = std::slice::from_raw_parts(ptr, len / 2);
                let end = units.iter().position(|v| *v == 0).unwrap_or(units.len());
                let text = String::from_utf16_lossy(&units[..end]);
                let _ = GlobalUnlock(global);
                Some(text)
            }
        }

        pub(crate) fn snapshot(&self) -> Result<ClipboardSnapshot, String> {
            unsafe {
                let mut formats = Vec::new();
                let mut previous = 0;
                loop {
                    SetLastError(ERROR_SUCCESS);
                    let format = EnumClipboardFormats(previous);
                    if format == 0 {
                        if GetLastError() != ERROR_SUCCESS {
                            return Err(
                                "Could not enumerate clipboard; it was left unchanged".into()
                            );
                        }
                        break;
                    }
                    if formats.len() >= 256 {
                        return Err("Clipboard has too many formats to preserve safely; it was left unchanged".into());
                    }
                    formats.push(format);
                    previous = format;
                }
                let snapshot = copy_formats(&formats, MAX_SNAPSHOT_BYTES, |format, remaining| {
                    let handle = GetClipboardData(format).map_err(|e| {
                        format!(
                            "Cannot preserve clipboard format {format}: {e}. Clipboard unchanged."
                        )
                    })?;
                    let global = HGLOBAL(handle.0);
                    let size = GlobalSize(global);
                    if size == 0 || size > remaining {
                        return Err(format!("Clipboard format {format} cannot be copied within the 64 MiB limit. Clipboard unchanged."));
                    }
                    let data = GlobalLock(global) as *const u8;
                    if data.is_null() {
                        return Err(format!("Clipboard format {format} is not readable memory. Clipboard unchanged."));
                    }
                    let bytes = std::slice::from_raw_parts(data, size).to_vec();
                    let _ = GlobalUnlock(global);
                    Ok(bytes)
                })?;
                Ok(ClipboardSnapshot(std::sync::Arc::new(snapshot)))
            }
        }

        pub(crate) fn set_text(&mut self, text: &str) -> Result<(), String> {
            if text.encode_utf16().count() >= MAX_SNAPSHOT_BYTES / 2 {
                return Err(
                    "Transcript exceeds the 64 MiB clipboard limit. Clipboard unchanged.".into(),
                );
            }
            let bytes = text
                .encode_utf16()
                .chain(std::iter::once(0))
                .flat_map(u16::to_ne_bytes)
                .collect();
            self.restore(&ClipboardSnapshot(std::sync::Arc::new(vec![FormatData {
                format: 13,
                bytes,
            }])))
        }

        pub(crate) fn restore(&mut self, snapshot: &ClipboardSnapshot) -> Result<(), String> {
            // Allocate every payload before emptying the clipboard. An
            // allocation failure therefore never destroys the current copy.
            let mut prepared = Vec::with_capacity(snapshot.0.len());
            for data in snapshot.0.iter() {
                prepared.push((data.format, OwnedGlobal::from_bytes(&data.bytes)?));
            }
            unsafe {
                EmptyClipboard().map_err(|e| format!("Could not replace clipboard: {e}"))?;
                for (format, mut memory) in prepared {
                    SetClipboardData(format, HANDLE(memory.0 .0))
                        .map_err(|e| format!("Could not restore clipboard format {format}: {e}"))?;
                    // Ownership transferred to the OS only on success.
                    memory.0 = HGLOBAL::default();
                }
            }
            Ok(())
        }
    }

    impl Drop for ClipboardAccess {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseClipboard();
                let _ = DestroyWindow(self.owner);
            }
        }
    }

    struct OwnedGlobal(HGLOBAL);
    impl OwnedGlobal {
        fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
            unsafe {
                let memory = Self(
                    GlobalAlloc(GMEM_MOVEABLE, bytes.len())
                        .map_err(|e| format!("Could not allocate clipboard memory: {e}"))?,
                );
                let pointer = GlobalLock(memory.0) as *mut u8;
                if pointer.is_null() {
                    return Err("Could not lock clipboard memory".into());
                }
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer, bytes.len());
                let _ = GlobalUnlock(memory.0);
                Ok(memory)
            }
        }
    }
    impl Drop for OwnedGlobal {
        fn drop(&mut self) {
            if !self.0 .0.is_null() {
                unsafe {
                    let _ = GlobalFree(self.0);
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn staged_memory_contains_an_independent_copy_without_touching_clipboard() {
            let bytes = vec![1, 2, 3, 4, 5];
            let memory = OwnedGlobal::from_bytes(&bytes).unwrap();
            unsafe {
                assert!(GlobalSize(memory.0) >= bytes.len());
                let pointer = GlobalLock(memory.0) as *const u8;
                assert!(!pointer.is_null());
                assert_eq!(
                    std::slice::from_raw_parts(pointer, bytes.len()),
                    bytes.as_slice()
                );
                let _ = GlobalUnlock(memory.0);
            }
            // RAII releases untransferred allocations here.
        }
    }
}

// arboard exposes text and decoded images on Linux/macOS. It does not expose
// arbitrary native clipboard formats; those platforms retain that limitation.
#[cfg(not(windows))]
mod portable {
    use arboard::{Clipboard, ImageData};
    use std::borrow::Cow;

    #[derive(Clone)]
    pub(crate) enum ClipboardSnapshot {
        #[cfg(target_os = "linux")]
        Wayland(super::super::wayland_clipboard::Snapshot),
        #[cfg(not(windows))]
        Empty,
        Text(String),
        Image {
            width: usize,
            height: usize,
            bytes: Vec<u8>,
        },
    }
    // Linux serves clipboard requests from the last owning application.
    // Keep an owner alive after a short operation drops its access guard,
    // including after a paste failure leaves the transcript for manual paste.
    #[cfg(target_os = "linux")]
    static CLIPBOARD: std::sync::OnceLock<parking_lot::Mutex<Option<Clipboard>>> =
        std::sync::OnceLock::new();
    #[cfg(target_os = "linux")]
    pub(crate) struct ClipboardAccess(parking_lot::MappedMutexGuard<'static, Clipboard>);
    #[cfg(not(target_os = "linux"))]
    pub(crate) struct ClipboardAccess(Clipboard);

    #[cfg(target_os = "linux")]
    fn wayland_clipboard_is_empty(
        offered: Result<std::collections::HashSet<String>, wl_clipboard_rs::paste::Error>,
    ) -> bool {
        // NoMimeType can mean files, rich text, or another unsupported format.
        // Only an absent selection establishes that nothing will be erased.
        matches!(offered, Err(wl_clipboard_rs::paste::Error::ClipboardEmpty))
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(super) fn requires_richer_snapshot(mimes: &std::collections::HashSet<String>) -> bool {
        let mut text = false;
        let mut image = false;
        for mime in mimes {
            if mime == "text/plain"
                || mime.starts_with("text/plain;")
                || matches!(mime.as_str(), "UTF8_STRING" | "STRING" | "TEXT")
            {
                text = true;
            } else if matches!(
                mime.as_str(),
                "image/png" | "image/jpeg" | "image/bmp" | "image/tiff"
            ) {
                image = true;
            } else if !matches!(
                mime.as_str(),
                "TARGETS" | "MULTIPLE" | "TIMESTAMP" | "SAVE_TARGETS"
            ) {
                // HTML, files, app-specific payloads and privacy hints cannot
                // be reconstructed by arboard's text/image snapshots.
                return true;
            }
        }
        // Restoring only the image would discard an independent text payload.
        text && image
    }

    /// `true` only when the general pasteboard advertises no types at all.
    /// Anything else — files, rich text, app-specific payloads — must be
    /// treated as preservable-or-unsafe, never as empty.
    #[cfg(target_os = "macos")]
    fn pasteboard_is_empty() -> bool {
        use objc::sel;
        unsafe {
            let pb: *mut objc::runtime::Object =
                objc::msg_send![objc::class!(NSPasteboard), generalPasteboard];
            if pb.is_null() {
                return false;
            }
            let types: *mut objc::runtime::Object = objc::msg_send![pb, types];
            if types.is_null() {
                return true;
            }
            let count: usize = objc::msg_send![types, count];
            count == 0
        }
    }

    #[cfg(target_os = "linux")]
    fn clipboard_is_empty() -> bool {
        // Match arboard's backend preference even in a mixed X11/Wayland
        // environment; an empty X11 selection says nothing about Wayland.
        if super::super::wayland_clipboard::supported() {
            use wl_clipboard_rs::paste::{get_mime_types, ClipboardType, Seat};
            return wayland_clipboard_is_empty(get_mime_types(
                ClipboardType::Regular,
                Seat::Unspecified,
            ));
        }
        use x11rb::protocol::xproto::ConnectionExt;
        let selection_owner = || -> Option<u32> {
            let (connection, _) = x11rb::connect(None).ok()?;
            let selection = connection
                .intern_atom(false, b"CLIPBOARD")
                .ok()?
                .reply()
                .ok()?
                .atom;
            let owner = connection
                .get_selection_owner(selection)
                .ok()?
                .reply()
                .ok()?
                .owner;
            Some(owner)
        };
        selection_owner() == Some(x11rb::NONE)
    }

    impl ClipboardAccess {
        pub(crate) fn open() -> Result<Self, String> {
            #[cfg(target_os = "linux")]
            {
                let mut owner = CLIPBOARD
                    .get_or_init(|| parking_lot::Mutex::new(None))
                    .lock();
                if owner.is_none() {
                    *owner = Some(
                        Clipboard::new().map_err(|e| format!("Could not open clipboard: {e}"))?,
                    );
                }
                Ok(Self(parking_lot::MutexGuard::map(owner, |owner| {
                    owner.as_mut().expect("clipboard initialized above")
                })))
            }
            #[cfg(not(target_os = "linux"))]
            {
                Clipboard::new()
                    .map(Self)
                    .map_err(|e| format!("Could not open clipboard: {e}"))
            }
        }
        pub(crate) fn sequence(&self) -> Option<u32> {
            #[cfg(target_os = "linux")]
            if super::super::wayland_clipboard::supported() {
                return super::super::wayland_clipboard::sequence();
            }
            None
        }
        pub(crate) fn text(&mut self) -> Option<String> {
            self.0.get_text().ok()
        }
        pub(crate) fn snapshot(&mut self) -> Result<ClipboardSnapshot, String> {
            #[cfg(target_os = "linux")]
            if super::super::wayland_clipboard::supported() {
                return super::super::wayland_clipboard::snapshot().map(ClipboardSnapshot::Wayland);
            }
            if let Ok(image) = self.0.get_image() {
                if image.bytes.len() > super::MAX_SNAPSHOT_BYTES {
                    return Err("Clipboard image exceeds the 64 MiB restoration limit. Clipboard unchanged.".into());
                }
                return Ok(ClipboardSnapshot::Image {
                    width: image.width,
                    height: image.height,
                    bytes: image.bytes.into_owned(),
                });
            }
            match self.0.get_text() {
                Ok(text) if text.len() <= super::MAX_SNAPSHOT_BYTES => {
                    Ok(ClipboardSnapshot::Text(text))
                }
                Ok(_) => Err(
                    "Clipboard text exceeds the 64 MiB restoration limit. Clipboard unchanged."
                        .into(),
                ),
                // An absent selection on Linux is proved through X11/Wayland
                // directly; on macOS only a truly empty general pasteboard may
                // be cleared on restore — a files/rich-only copy has types but
                // no text, and ContentNotAvailable must not destroy it.
                #[cfg(target_os = "linux")]
                Err(arboard::Error::ContentNotAvailable) if clipboard_is_empty() => Ok(ClipboardSnapshot::Empty),
                #[cfg(target_os = "macos")]
                Err(arboard::Error::ContentNotAvailable) if pasteboard_is_empty() => Ok(ClipboardSnapshot::Empty),
                // Unsupported formats must still be left unchanged. Linux's
                // empty selection is checked separately from a missing MIME.
                Err(arboard::Error::ContentNotAvailable) => Err("Clipboard content cannot be preserved on this platform. Clipboard unchanged. Disable clipboard restoration to replace it.".into()),
                Err(err) => Err(format!("Could not preserve clipboard: {err}")),
            }
        }
        pub(crate) fn set_text(&mut self, text: &str) -> Result<(), String> {
            #[cfg(target_os = "linux")]
            if super::super::wayland_clipboard::supported() {
                return super::super::wayland_clipboard::set_text(text);
            }
            self.0
                .set_text(text)
                .map_err(|e| format!("Could not write clipboard: {e}"))
        }
        pub(crate) fn restore(&mut self, snapshot: &ClipboardSnapshot) -> Result<(), String> {
            match snapshot {
                #[cfg(target_os = "linux")]
                ClipboardSnapshot::Wayland(snapshot) => {
                    return super::super::wayland_clipboard::restore(snapshot)
                }
                #[cfg(not(windows))]
                ClipboardSnapshot::Empty => self.0.clear(),
                ClipboardSnapshot::Text(text) => self.0.set_text(text),
                ClipboardSnapshot::Image {
                    width,
                    height,
                    bytes,
                } => self.0.set_image(ImageData {
                    width: *width,
                    height: *height,
                    bytes: Cow::Borrowed(bytes),
                }),
            }
            .map_err(|e| format!("Could not restore clipboard: {e}"))
        }
    }

    #[cfg(all(test, target_os = "linux"))]
    mod tests {
        use super::*;

        #[test]
        fn plain_text_and_images_can_be_restored_without_rich_payloads() {
            for mimes in [
                vec!["text/plain", "text/plain;charset=utf-8", "UTF8_STRING"],
                vec!["image/png", "image/jpeg"],
            ] {
                assert!(!requires_richer_snapshot(
                    &mimes.into_iter().map(String::from).collect()
                ));
            }
        }

        #[test]
        fn readable_plain_text_does_not_authorize_discarding_other_formats() {
            for extra in [
                "text/html",
                "text/uri-list",
                "application/rtf",
                "application/x-kde-passwordManagerHint",
                "image/png",
            ] {
                assert!(requires_richer_snapshot(&std::collections::HashSet::from(
                    ["text/plain".into(), extra.into()]
                )));
            }
        }

        #[test]
        fn empty_wayland_selection_is_distinct_from_unsupported_or_unreadable_content() {
            use wl_clipboard_rs::paste::Error;
            assert!(wayland_clipboard_is_empty(Err(Error::ClipboardEmpty)));
            assert!(!wayland_clipboard_is_empty(Err(Error::NoMimeType)));
            assert!(!wayland_clipboard_is_empty(Err(Error::NoSeats)));
            assert!(!wayland_clipboard_is_empty(Err(Error::MissingProtocol {
                name: "data-control",
                version: 1
            })));
            assert!(!wayland_clipboard_is_empty(Ok(
                std::collections::HashSet::from(["text/uri-list".into()])
            )));
        }
    }
}
#[cfg(not(windows))]
pub(super) use portable::{ClipboardAccess, ClipboardSnapshot};

// Live tests run in a short-lived process. Restoring with arboard alone would
// leave that process owning the previous content until it exits. Hand the
// restored text to KDE's persistent clipboard manager before the test ends.
#[cfg(all(test, target_os = "linux"))]
pub(super) fn live_test_snapshot() -> Option<ClipboardSnapshot> {
    let manager = std::process::Command::new("qdbus6")
        .args(["org.kde.klipper", "/klipper", "getClipboardContents"])
        .output()
        .ok()?;
    if !manager.status.success() {
        return None;
    }
    let mut access = ClipboardAccess::open().ok()?;
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        let mimes = wl_clipboard_rs::paste::get_mime_types(
            wl_clipboard_rs::paste::ClipboardType::Regular,
            wl_clipboard_rs::paste::Seat::Unspecified,
        )
        .ok()?;
        if portable::requires_richer_snapshot(&mimes) {
            return None;
        }
    }
    access.text().map(ClipboardSnapshot::Text)
}

#[cfg(all(test, target_os = "linux"))]
pub(super) fn restore_live_test_snapshot(snapshot: &ClipboardSnapshot) {
    let ClipboardSnapshot::Text(text) = snapshot else {
        unreachable!("live tests require a text snapshot");
    };
    let result = std::process::Command::new("qdbus6")
        .args(["org.kde.klipper", "/klipper", "setClipboardContents"])
        .arg(text)
        .output()
        .expect("restore previous clipboard with the persistent KDE manager");
    assert!(
        result.status.success(),
        "persistent clipboard restoration failed"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires live KDE with a text clipboard; restores it through the persistent manager"]
    fn linux_clipboard_remains_available_after_access_is_dropped() {
        let Some(previous) = live_test_snapshot() else {
            eprintln!(
                "skipping: a persistent KDE clipboard manager and text clipboard are required"
            );
            return;
        };
        let mut clipboard = ClipboardAccess::open().expect("open desktop clipboard");
        let marker = format!("Reflow clipboard ownership check {}", uuid::Uuid::new_v4());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            clipboard.set_text(&marker).unwrap();
            drop(clipboard);
            // A separate reader models a paste target after injection has
            // released its operation guard, including the manual fallback.
            let mut reader = arboard::Clipboard::new().unwrap();
            assert_eq!(reader.get_text().unwrap(), marker);
        }));
        restore_live_test_snapshot(&previous);
        if let Err(error) = result {
            std::panic::resume_unwind(error);
        }
    }

    #[test]
    fn text_image_html_and_files_are_copyable_memory_formats() {
        let formats = [13, 8, 17, 15, 0xc000, 0xc001];
        for format in formats {
            assert_eq!(copyable_format(format, &formats), Ok(true));
        }
    }
    #[test]
    fn bitmap_is_preserved_as_a_synthesized_dib() {
        assert_eq!(copyable_format(2, &[2, 8]), Ok(false));
        assert_eq!(copyable_format(9, &[8, 9]), Ok(false));
        assert!(copyable_format(2, &[2]).is_err());
    }
    #[test]
    fn noncopyable_formats_refuse_replacement() {
        for format in [3, 9, 14, 0x80, 0x200, 0x2ff] {
            assert!(copyable_format(format, &[13, format]).is_err());
        }
    }

    #[test]
    fn snapshots_preserve_format_order_and_every_supported_payload() {
        let expected = vec![
            FormatData {
                format: 0xc000,
                bytes: b"<b>HTML</b>".to_vec(),
            },
            FormatData {
                format: 8,
                bytes: vec![1, 2, 3],
            },
            FormatData {
                format: 15,
                bytes: b"files".to_vec(),
            },
            FormatData {
                format: 13,
                bytes: b"text".to_vec(),
            },
        ];
        let formats: Vec<_> = expected.iter().map(|data| data.format).collect();
        let snapshot = copy_formats(&formats, MAX_SNAPSHOT_BYTES, |format, _| {
            Ok(expected
                .iter()
                .find(|data| data.format == format)
                .unwrap()
                .bytes
                .clone())
        })
        .unwrap();
        assert_eq!(snapshot, expected);
    }

    #[test]
    fn a_snapshot_exceeding_total_budget_is_rejected() {
        assert!(copy_formats(&[13, 15], 4, |_, _| Ok(vec![1, 2, 3])).is_err());
        assert!(copy_formats(&[13], 4, |_, _| Ok(vec![])).is_err());
    }
}
