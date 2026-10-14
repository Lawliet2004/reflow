//! Propagate a cancelled async session into its already-running delivery worker.
use std::cell::RefCell;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

thread_local! {
    static ACTIVE: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

pub(crate) struct CancelOnDrop(pub(crate) Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

struct Scope(Option<Arc<AtomicBool>>);
impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.with(|active| active.replace(self.0.take()));
    }
}

pub(crate) fn with<T>(cancelled: Arc<AtomicBool>, action: impl FnOnce() -> T) -> T {
    let _scope = Scope(ACTIVE.with(|active| active.replace(Some(cancelled))));
    action()
}

pub(crate) fn active() -> Option<Arc<AtomicBool>> {
    ACTIVE.with(|active| active.borrow().clone())
}
pub(crate) fn check() -> Result<(), String> {
    if active().is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err("Text delivery was cancelled".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dropped_session_invalidates_delivery_and_worker_scope_does_not_leak() {
        let cancelled = Arc::new(AtomicBool::new(false));
        with(Arc::clone(&cancelled), || {
            assert!(check().is_ok());
            drop(CancelOnDrop(cancelled));
            assert!(check().unwrap_err().contains("cancelled"));
        });
        assert!(active().is_none());
    }
}
