//! Revocation must interrupt a stop pipeline, not wait behind it in its queue.
use crate::context::AppContext;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub(crate) struct DispatchGuard {
    generation: AtomicU64,
    owned: RwLock<Option<u64>>,
}

impl DispatchGuard {
    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
    pub(crate) fn current(&self, generation: u64) -> bool {
        self.generation() == generation
    }
    pub(crate) fn own(&self, session: Option<u64>) {
        *self.owned.write() = session;
    }
    pub(crate) fn revoke(&self, ctx: &AppContext) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        if let Some(id) = *self.owned.read() {
            // Even when session_operation is busy, cancel_owned sends the
            // cancellation watch immediately. The queued cleanup can wait.
            let _ = crate::session::cancel_owned(ctx, Some(id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn revocation_interrupts_processing_and_invalidates_queued_release() {
        let dir =
            std::env::temp_dir().join(format!("reflow_dispatch_guard_{}", uuid::Uuid::new_v4()));
        let ctx = AppContext::bootstrap_test(dir.clone());
        *ctx.current_session_id.write() = Some(7);
        let (sender, receiver) = tokio::sync::watch::channel(false);
        *ctx.session_cancel.write() = Some(sender);
        let guard = DispatchGuard::default();
        guard.own(Some(7));
        let queued_release = guard.generation();
        let _processing = ctx.session_operation.lock().await;
        guard.revoke(&ctx);
        assert!(
            *receiver.borrow(),
            "revoke must signal before acquiring the busy operation lock"
        );
        assert!(!guard.current(queued_release));
        assert!(guard.current(guard.generation()));
        drop(_processing);
        drop(ctx);
        let _ = std::fs::remove_dir_all(dir);
    }
}
