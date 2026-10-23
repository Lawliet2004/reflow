//! A short-lived lease keeps shortcut selection separate from dictation.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

static NEXT: AtomicU64 = AtomicU64::new(1);
static EPOCH: AtomicU64 = AtomicU64::new(0);
static CAPTURE: Mutex<Option<(u64, Instant)>> = Mutex::new(None);

pub fn begin() -> u64 {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    *CAPTURE.lock() = Some((id, Instant::now() + Duration::from_secs(120)));
    EPOCH.fetch_add(1, Ordering::SeqCst);
    id
}

pub fn end(id: u64) {
    let mut lease = CAPTURE.lock();
    if lease.is_some_and(|(current, _)| current == id) {
        *lease = None;
        EPOCH.fetch_add(1, Ordering::SeqCst);
    }
}

/// Queued presses from before a capture must never become dictation afterward.
pub fn epoch() -> u64 {
    EPOCH.load(Ordering::SeqCst)
}

pub fn active() -> Option<u64> {
    live(*CAPTURE.lock(), Instant::now())
}

fn live(lease: Option<(u64, Instant)>, now: Instant) -> Option<u64> {
    lease
        .filter(|(_, expires)| *expires > now)
        .map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_expires_if_the_window_disappears() {
        let now = Instant::now();
        assert_eq!(live(Some((7, now + Duration::from_secs(1))), now), Some(7));
        assert_eq!(live(Some((7, now)), now), None);
        assert_eq!(live(None, now), None);
    }

    #[test]
    fn old_capture_cannot_end_a_newer_one() {
        let old = begin();
        let new = begin();
        end(old);
        assert_eq!(active(), Some(new));
        end(new);
        assert_eq!(active(), None);
    }
}
