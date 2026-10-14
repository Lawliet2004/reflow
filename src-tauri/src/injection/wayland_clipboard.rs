//! Exact MIME payload snapshots for Wayland, with a bounded byte budget.
//! A private offer marker distinguishes our temporary transcript from a new
//! user copy containing identical text. No clipboard contents are logged.

use std::collections::HashSet;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use wl_clipboard_rs::{copy, paste};

const MARKER: &str = "application/x-reflow-temporary-selection-";
const PAYLOAD_TIMEOUT: Duration = Duration::from_secs(2);
static SEQUENCE: AtomicU32 = AtomicU32::new(0);

#[derive(Clone)]
pub(crate) struct Snapshot(Vec<(String, Vec<u8>)>);

fn offered() -> Result<HashSet<String>, paste::Error> {
    paste::get_mime_types(paste::ClipboardType::Regular, paste::Seat::Unspecified)
}

fn preservable(mimes: &HashSet<String>) -> bool {
    mimes.len() <= 64
        && !mimes.iter().any(|mime| {
            mime.to_ascii_lowercase().contains("passwordmanagerhint")
                || mime == "application/x-kde-cutselection"
        })
}

fn read_bounded(
    mut reader: impl Read + AsRawFd,
    budget: usize,
    deadline: Instant,
) -> Result<Vec<u8>, String> {
    let fd = reader.as_raw_fd();
    // The offer may stop responding. Nonblocking reads and poll bound that
    // wait without leaving a detached reader thread behind.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err("Could not bound the clipboard read. Clipboard unchanged.".into());
        }
    }
    let mut bytes = Vec::new();
    let mut chunk = [0; 16_384];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("Clipboard payload timed out. Clipboard unchanged.".into());
        }
        let mut poll = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe {
            libc::poll(
                &mut poll,
                1,
                remaining.as_millis().min(i32::MAX as u128) as i32,
            )
        };
        if ready < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err("Could not read clipboard payload. Clipboard unchanged.".into());
        }
        if ready == 0 {
            continue;
        }
        match reader.read(&mut chunk) {
            Ok(0) => return Ok(bytes),
            Ok(count) if count <= budget.saturating_sub(bytes.len()) => {
                bytes.extend_from_slice(&chunk[..count])
            }
            Ok(_) => {
                return Err(
                    "Clipboard exceeds the 64 MiB restoration limit. Clipboard unchanged.".into(),
                )
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => return Err("Could not read clipboard payload. Clipboard unchanged.".into()),
        }
    }
}

/// One-shot capability probe: WAYLAND_DISPLAY alone does not mean the
/// compositor speaks wlr-data-control — wl-clipboard-rs needs that protocol,
/// while arboard may be using its X11 backend. Every caller must agree on
/// the same backend, so the answer is cached for the process lifetime.
pub(super) fn supported() -> bool {
    static SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return false;
        }
        match offered() {
            // The compositor answered: empty/unknown selections still prove
            // the data-control protocol is present.
            Ok(_)
            | Err(paste::Error::ClipboardEmpty)
            | Err(paste::Error::NoMimeType) => true,
            // MissingProtocol/NoSeats/Io: native Wayland clipboard is not
            // usable — arboard's backend (possibly X11) takes over.
            Err(_) => false,
        }
    })
}

pub(super) fn snapshot() -> Result<Snapshot, String> {
    let mimes = match offered() {
        Ok(mimes) => mimes,
        Err(paste::Error::ClipboardEmpty) => return Ok(Snapshot(Vec::new())),
        Err(error) => {
            return Err(format!(
                "Could not verify clipboard formats: {error}. Clipboard unchanged."
            ))
        }
    };
    if !preservable(&mimes) {
        return Err("Clipboard contains private or cut-file data that cannot safely be restored. Clipboard unchanged. Copy the transcript from Reflow after pasting or clearing that selection.".into());
    }
    let mut formats: Vec<_> = mimes
        .iter()
        .filter(|mime| !mime.starts_with(MARKER))
        .cloned()
        .collect();
    formats.sort();
    let deadline = Instant::now() + PAYLOAD_TIMEOUT;
    let mut remaining = super::clipboard::MAX_SNAPSHOT_BYTES;
    let mut payloads = Vec::new();
    for mime in formats {
        let (reader, _) = paste::get_contents(
            paste::ClipboardType::Regular,
            paste::Seat::Unspecified,
            paste::MimeType::Specific(&mime),
        )
        .map_err(|error| {
            format!("Could not preserve clipboard format {mime}: {error}. Clipboard unchanged.")
        })?;
        let bytes = read_bounded(reader, remaining, deadline)?;
        remaining -= bytes.len();
        payloads.push((mime, bytes));
    }
    if offered().ok().as_ref() != Some(&mimes) {
        return Err("Clipboard changed while being preserved. Try insertion again.".into());
    }
    Ok(Snapshot(payloads))
}

fn write(payloads: &[(String, Vec<u8>)]) -> Result<(), String> {
    let mut options = copy::Options::new();
    options.omit_additional_text_mime_types(true);
    options
        .copy_multi(
            payloads
                .iter()
                .map(|(mime, bytes)| copy::MimeSource {
                    source: copy::Source::Bytes(bytes.clone().into_boxed_slice()),
                    mime_type: copy::MimeType::Specific(mime.clone()),
                })
                .collect(),
        )
        .map_err(|error| format!("Could not write Wayland clipboard: {error}"))
}

pub(super) fn set_text(text: &str) -> Result<(), String> {
    if text.len() > super::clipboard::MAX_SNAPSHOT_BYTES {
        return Err("Transcript exceeds the 64 MiB clipboard limit. Clipboard unchanged.".into());
    }
    let sequence = SEQUENCE.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    write(&[
        ("text/plain;charset=utf-8".into(), text.as_bytes().to_vec()),
        ("text/plain".into(), text.as_bytes().to_vec()),
        (format!("{MARKER}{sequence}"), Vec::new()),
    ])
}

fn offer_sequence(mimes: &HashSet<String>) -> u32 {
    // A richer offer with the same text must also cancel restoration.
    if mimes.iter().any(|mime| {
        !mime.starts_with(MARKER)
            && !matches!(mime.as_str(), "text/plain" | "text/plain;charset=utf-8")
    }) {
        return 0;
    }
    mimes
        .iter()
        .find_map(|mime| mime.strip_prefix(MARKER)?.parse().ok())
        .unwrap_or(0)
}

pub(super) fn sequence() -> Option<u32> {
    match offered() {
        Ok(mimes) => Some(offer_sequence(&mimes)),
        Err(paste::Error::ClipboardEmpty) => Some(0),
        Err(_) => None,
    }
}

pub(super) fn restore(snapshot: &Snapshot) -> Result<(), String> {
    if snapshot.0.is_empty() {
        copy::clear(copy::ClipboardType::Regular, copy::Seat::All)
            .map_err(|error| error.to_string())
    } else {
        write(&snapshot.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "reads current Wayland MIME payloads without writing or logging content"]
    fn live_offer_can_be_preserved_without_changing_the_clipboard() {
        let before = offered().expect("requires a Wayland selection");
        let saved = snapshot().expect("all MIME payloads fit the bounded preservation policy");
        let expected: HashSet<_> = before
            .iter()
            .filter(|mime| !mime.starts_with(MARKER))
            .cloned()
            .collect();
        assert_eq!(
            saved
                .0
                .iter()
                .map(|(mime, _)| mime.clone())
                .collect::<HashSet<_>>(),
            expected
        );
        assert_eq!(offered().unwrap(), before);
        eprintln!(
            "Preserved {} MIME payloads, {} total bytes; selection left unchanged",
            saved.0.len(),
            saved.0.iter().map(|(_, bytes)| bytes.len()).sum::<usize>()
        );
    }

    #[test]
    #[ignore = "requires live KDE and REFLOW_CLIPBOARD_TESTS=isolated; preserves existing clipboard"]
    fn live_mime_roundtrip_and_same_text_recopy_cancel_restoration() {
        assert_eq!(
            std::env::var("REFLOW_CLIPBOARD_TESTS").as_deref(),
            Ok("isolated")
        );
        // Only begin when the persistent manager can restore the user's text
        // after this short-lived test process exits. Do not reduce rich data.
        eprintln!(
            "Clipboard verification prerequisite (MIME metadata only): {:?}",
            offered()
        );
        let previous = super::super::clipboard::live_test_snapshot()
            .expect("requires a plain-text clipboard and KDE Klipper; no data changed");
        let test_marker = format!("application/x-reflow-test-{}", uuid::Uuid::new_v4());
        let mut last_sequence = None;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let expected = vec![
                (
                    "text/plain".into(),
                    "Reflow नमस्ते clipboard".as_bytes().to_vec(),
                ),
                ("text/html".into(), b"<b>Reflow</b>".to_vec()),
                (
                    "text/uri-list".into(),
                    b"file:///tmp/reflow-isolated-test.txt\r\n".to_vec(),
                ),
                ("image/png".into(), b"opaque-image-fixture".to_vec()),
                (test_marker.clone(), Vec::new()),
            ];
            write(&expected).unwrap();
            let saved = snapshot().unwrap();
            assert_eq!(saved.0, {
                let mut sorted = expected.clone();
                sorted.sort_by(|a, b| a.0.cmp(&b.0));
                sorted
            });
            for _ in 0..30 {
                set_text("Reflow same text").unwrap();
                last_sequence = sequence();
                assert!(last_sequence.is_some_and(|sequence| sequence != 0));
                restore(&saved).unwrap();
                assert_eq!(snapshot().unwrap().0, saved.0);
            }
            set_text("Reflow same text").unwrap();
            last_sequence = sequence();
            // Model a new application copy with the exact same plain text.
            write(&[
                ("text/plain".into(), b"Reflow same text".to_vec()),
                (test_marker.clone(), Vec::new()),
            ])
            .unwrap();
            assert_ne!(sequence(), last_sequence);
        }));
        let ours = offered().is_ok_and(|mimes| mimes.contains(&test_marker))
            || last_sequence.is_some_and(|last| last != 0 && sequence() == Some(last));
        if ours {
            super::super::clipboard::restore_live_test_snapshot(&previous);
        } else {
            panic!(
                "Clipboard changed during verification; newer user selection was left untouched"
            );
        }
        if let Err(error) = result {
            std::panic::resume_unwind(error);
        }
    }

    #[test]
    fn rich_images_unicode_and_file_copies_are_preservable_but_secrets_and_cuts_are_not() {
        let mimes = [
            "text/plain",
            "text/html",
            "text/uri-list",
            "image/png",
            "application/octet-stream",
        ]
        .map(str::to_owned)
        .into_iter()
        .collect();
        assert!(preservable(&mimes));
        for hint in [
            "x-kde-passwordManagerHint",
            "application/x-kde-passwordManagerHint",
            "application/x-kde-cutselection",
        ] {
            assert!(!preservable(&[hint.to_owned()].into_iter().collect()));
        }
    }

    #[test]
    fn same_text_without_our_offer_marker_or_with_new_formats_cannot_restore() {
        let mut offer = HashSet::from(["text/plain".into(), format!("{MARKER}42")]);
        assert_eq!(offer_sequence(&offer), 42);
        offer.insert("text/html".into());
        assert_eq!(offer_sequence(&offer), 0);
        offer.remove("text/html");
        offer.remove(&format!("{MARKER}42"));
        assert_eq!(offer_sequence(&offer), 0);
    }

    #[test]
    fn stalled_or_oversized_payloads_fail_without_an_unbounded_reader() {
        use std::io::Write;
        use std::os::unix::net::UnixStream;
        let (reader, mut writer) = UnixStream::pair().unwrap();
        writer.write_all(b"12345").unwrap();
        assert!(read_bounded(reader, 4, Instant::now() + PAYLOAD_TIMEOUT)
            .unwrap_err()
            .contains("limit"));
        let (reader, _writer) = UnixStream::pair().unwrap();
        assert!(
            read_bounded(reader, 10, Instant::now() + Duration::from_millis(20))
                .unwrap_err()
                .contains("timed out")
        );
    }
}
