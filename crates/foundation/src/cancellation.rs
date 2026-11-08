use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::{Condvar, Mutex};

#[derive(Default)]
struct CancellationState {
    cancelled: AtomicBool,
    changed: Condvar,
    wait_lock: Mutex<()>,
    children: Mutex<Vec<Weak<CancellationState>>>,
}

/// Project-owned cooperative cancellation primitive for blocking Rust work.
#[derive(Clone, Default)]
pub struct CancellationToken {
    state: Arc<CancellationState>,
}

impl CancellationToken {
    pub fn cancel(&self) {
        // Serialize the predicate transition with Condvar::wait_timeout so a
        // cancellation cannot occur between the waiter's final check and its
        // atomic unlock-and-wait operation.
        {
            let _guard = self.state.wait_lock.lock();
            self.state.cancelled.store(true, Ordering::Release);
            self.state.changed.notify_all();
        }
        // The flag is set before the children are taken, so a concurrent
        // `child()` either sees the cancellation or is in the taken list.
        let children = std::mem::take(&mut *self.state.children.lock());
        for child in children.iter().filter_map(Weak::upgrade) {
            CancellationToken { state: child }.cancel();
        }
    }

    /// A token that is cancelled with this one and can also be cancelled on
    /// its own without affecting this one.
    pub fn child(&self) -> CancellationToken {
        let child = CancellationToken::default();
        let mut children = self.state.children.lock();
        if self.is_cancelled() {
            drop(children);
            child.cancel();
            return child;
        }
        children.retain(|weak| weak.strong_count() > 0);
        children.push(Arc::downgrade(&child.state));
        child
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    /// Waits until cancellation or the timeout elapses without polling/sleeping.
    /// Returns `true` only when cancellation was observed.
    pub fn wait_timeout(&self, timeout: Duration) -> bool {
        if self.is_cancelled() {
            return true;
        }
        let mut guard = self.state.wait_lock.lock();
        if self.is_cancelled() {
            return true;
        }
        let _ = self.state.changed.wait_for(&mut guard, timeout);
        self.is_cancelled()
    }
}

#[cfg(test)]
mod tests;
