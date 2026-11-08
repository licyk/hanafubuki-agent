use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use kardia_core::IdTable;
use parking_lot::Mutex;
use tokio::sync::Notify;

/// How long a retiring entry waits for running calls before cancelling them.
pub const DRAIN_GRACE: Duration = Duration::from_secs(2);
/// How long a retiring entry waits for cancelled calls before it lets the
/// plugin's teardown continue and logs the calls it left running.
pub const RETIRE_CANCEL_BOUND: Duration = Duration::from_secs(3);

#[derive(Default)]
pub(super) struct EntryState {
    retired: AtomicBool,
    calls: Mutex<IdTable<CancellationToken>>,
    drained: Notify,
}

impl EntryState {
    pub(super) fn is_retired(&self) -> bool {
        self.retired.load(Ordering::Acquire)
    }

    pub(super) async fn retire_and_drain(&self, owner: &str) {
        self.retired.store(true, Ordering::Release);
        if self.wait_drained(DRAIN_GRACE).await {
            return;
        }
        for call in self.calls.lock().values() {
            call.cancel();
        }
        if !self.wait_drained(RETIRE_CANCEL_BOUND).await {
            log::warn!(
                "{} call(s) of plugin {owner} were still running when it stopped",
                self.calls.lock().len()
            );
        }
    }

    async fn wait_drained(&self, bound: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + bound;
        loop {
            let drained = self.drained.notified();
            if self.calls.lock().is_empty() {
                return true;
            }
            if tokio::time::timeout_at(deadline, drained).await.is_err() {
                return self.calls.lock().is_empty();
            }
        }
    }
}

/// Keeps a registry entry's plugin from being torn down while one call runs
/// against it. The plugin stopping cancels the call's token after
/// [`DRAIN_GRACE`]; the call must observe that token.
pub struct EntryLease {
    state: Arc<EntryState>,
    id: u64,
}

impl EntryLease {
    pub(super) fn acquire(state: &Arc<EntryState>, call: &CancellationToken) -> Option<Self> {
        if state.is_retired() {
            return None;
        }
        let id = state.calls.lock().insert(call.clone());
        let lease = Self {
            state: state.clone(),
            id,
        };
        // A retirement that started between the check and the insert may
        // already be waiting; dropping the lease wakes it.
        if state.is_retired() {
            return None;
        }
        Some(lease)
    }
}

impl Drop for EntryLease {
    fn drop(&mut self) {
        let mut calls = self.state.calls.lock();
        calls.remove(self.id);
        if calls.is_empty() {
            self.state.drained.notify_waiters();
        }
    }
}
