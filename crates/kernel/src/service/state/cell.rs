//! The live sessions, one cell each.
//!
//! `SessionTable` maps a session to its `SessionCell`; its lock is held only
//! to look a cell up, insert one or remove one, and no other lock is taken
//! under it. A cell owns its `SessionRecord` behind its own lock, held only
//! for in-memory work on that record: never across an await, a history
//! append or any other I/O, and never while another cell's lock is taken. So
//! one session's work never waits for another's.
//!
//! A cell has two signals. `persisted` advances when the record's staged
//! commit settles (its revision is published or the record is rolled back);
//! `terminal` advances when a turn or compaction reaches a terminal state.
//! Both advance when the session is removed. Each signal has a blocking side,
//! a `Condvar` on the cell's lock, for callers off the async runtime (IPC on
//! blocking workers, shutdown), and an async side, a `watch` epoch, that
//! async callers await.
//!
//! A commit that cannot wait for the staged one, because its caller is a
//! drop guard, is deferred on the cell instead: whatever settles the staged
//! commit stages it under the same lock (`stage_or_defer`).

use std::collections::{HashMap, VecDeque};
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::{Condvar, Mutex, MutexGuard};
use tokio::sync::watch;

use super::SessionRecord;
use crate::model::{AgentError, AgentSessionId};
use crate::service::progress::ExecutionProgressQueue;
use crate::AgentHistoryFollowUp;

#[derive(Default)]
pub(in crate::service) struct SessionTable {
    cells: Mutex<HashMap<AgentSessionId, Arc<SessionCell>>>,
}

impl SessionTable {
    pub(in crate::service) fn get(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Arc<SessionCell>, AgentError> {
        self.find(session_id).ok_or(AgentError::SessionNotFound)
    }

    pub(in crate::service) fn find(&self, session_id: AgentSessionId) -> Option<Arc<SessionCell>> {
        self.cells.lock().get(&session_id).cloned()
    }

    pub(in crate::service) fn contains(&self, session_id: AgentSessionId) -> bool {
        self.cells.lock().contains_key(&session_id)
    }

    pub(in crate::service) fn is_empty(&self) -> bool {
        self.cells.lock().is_empty()
    }

    /// Every live cell, by session id.
    pub(in crate::service) fn cells(&self) -> Vec<Arc<SessionCell>> {
        let mut cells = self.cells.lock().values().cloned().collect::<Vec<_>>();
        cells.sort_by_key(|cell| cell.session_id);
        cells
    }

    /// Adds `record` while `accepting` holds. The flag is read under the
    /// table's lock, so a shutdown that clears it before listing the cells
    /// sees every session added before it did.
    pub(in crate::service) fn insert(
        &self,
        record: SessionRecord,
        accepting: &AtomicBool,
    ) -> Result<Arc<SessionCell>, AgentError> {
        let mut cells = self.cells.lock();
        if !accepting.load(Ordering::Acquire) {
            return Err(AgentError::ShuttingDown);
        }
        let cell = Arc::new(SessionCell::new(record));
        cells.insert(cell.session_id, cell.clone());
        Ok(cell)
    }

    /// Adds `record` unless its session already has a cell; `false` when it
    /// had one.
    pub(in crate::service) fn insert_if_absent(&self, record: SessionRecord) -> bool {
        let mut cells = self.cells.lock();
        if cells.contains_key(&record.session_id) {
            return false;
        }
        let cell = Arc::new(SessionCell::new(record));
        cells.insert(cell.session_id, cell);
        true
    }

    /// Removes the session `guard` holds, marking its cell removed first so
    /// every holder of the cell sees `SessionNotFound` from then on.
    pub(in crate::service) fn remove(&self, mut guard: SessionGuard<'_>) {
        guard.slot.removed = true;
        let deferred = std::mem::take(&mut guard.slot.deferred);
        let cell = guard.cell;
        drop(guard);
        drop(deferred);
        cell.progress.close();
        cell.notify_persisted();
        cell.notify_terminal();
        let mut cells = self.cells.lock();
        if cells
            .get(&cell.session_id)
            .is_some_and(|current| std::ptr::eq(Arc::as_ptr(current), cell))
        {
            cells.remove(&cell.session_id);
        }
    }
}

pub(in crate::service) struct SessionCell {
    session_id: AgentSessionId,
    slot: Mutex<SessionSlot>,
    persisted: SessionSignal,
    terminal: SessionSignal,
    /// Execution progress waiting to be committed, behind its own lock.
    progress: ExecutionProgressQueue,
}

struct SessionSlot {
    record: SessionRecord,
    removed: bool,
    /// Commits waiting for the staged one to settle, in order.
    deferred: VecDeque<DeferredCommit>,
}

/// A commit deferred until the staged one settles (`SessionCell::
/// stage_or_defer`). It stages on the record, under the cell's lock, and
/// returns the checkpoint to append, or nothing when it no longer applies.
pub(in crate::service) type DeferredCommit =
    Box<dyn FnOnce(&Arc<SessionCell>, &mut SessionRecord) -> Option<AgentHistoryFollowUp> + Send>;

/// The cell's lock, held over its live record.
pub(in crate::service) struct SessionGuard<'a> {
    cell: &'a SessionCell,
    slot: MutexGuard<'a, SessionSlot>,
}

impl Deref for SessionGuard<'_> {
    type Target = SessionRecord;

    fn deref(&self) -> &SessionRecord {
        &self.slot.record
    }
}

impl DerefMut for SessionGuard<'_> {
    fn deref_mut(&mut self) -> &mut SessionRecord {
        &mut self.slot.record
    }
}

impl SessionGuard<'_> {
    /// Once the staged commit has settled: stages the first deferred commit
    /// that still applies and returns its checkpoint to append.
    pub(in crate::service) fn stage_deferred(
        &mut self,
        cell: &Arc<SessionCell>,
    ) -> Option<AgentHistoryFollowUp> {
        while self.slot.record.status.persisting_revision.is_none() {
            let commit = self.slot.deferred.pop_front()?;
            if let Some(follow_up) = commit(cell, &mut self.slot.record) {
                return Some(follow_up);
            }
        }
        None
    }
}

struct SessionSignal {
    blocking: Condvar,
    epoch: watch::Sender<u64>,
}

impl SessionSignal {
    fn new() -> Self {
        Self {
            blocking: Condvar::new(),
            epoch: watch::Sender::new(0),
        }
    }

    fn notify(&self) {
        self.epoch
            .send_modify(|epoch| *epoch = epoch.wrapping_add(1));
        self.blocking.notify_all();
    }
}

impl SessionCell {
    fn new(record: SessionRecord) -> Self {
        Self {
            session_id: record.session_id,
            slot: Mutex::new(SessionSlot {
                record,
                removed: false,
                deferred: VecDeque::new(),
            }),
            persisted: SessionSignal::new(),
            terminal: SessionSignal::new(),
            progress: ExecutionProgressQueue::default(),
        }
    }

    pub(in crate::service) fn session_id(&self) -> AgentSessionId {
        self.session_id
    }

    /// The session's queued execution progress (`service::progress`).
    pub(in crate::service) fn progress(&self) -> &ExecutionProgressQueue {
        &self.progress
    }

    /// Locks the record as it is, staged commit or not.
    pub(in crate::service) fn lock(&self) -> Result<SessionGuard<'_>, AgentError> {
        let slot = self.slot.lock();
        if slot.removed {
            return Err(AgentError::SessionNotFound);
        }
        Ok(SessionGuard { cell: self, slot })
    }

    /// Locks the record once no commit is staged on it, waiting
    /// asynchronously for the staged one to settle.
    pub(in crate::service) async fn lock_after_persistence(
        &self,
    ) -> Result<SessionGuard<'_>, AgentError> {
        let mut persisted = self.persisted.epoch.subscribe();
        loop {
            {
                let guard = self.lock()?;
                if guard.status.persisting_revision.is_none() {
                    return Ok(guard);
                }
            }
            // The sender lives as long as the cell, so this only returns
            // when the epoch advanced.
            let _ = persisted.changed().await;
        }
    }

    /// Like `lock_after_persistence`, blocking the calling thread. Only for
    /// callers off the async runtime, or a drop guard: a staged commit
    /// settles where the history store completes its append, never on the
    /// waiter's runtime, so the wait is bounded by that append.
    pub(in crate::service) fn lock_after_persistence_blocking(
        &self,
    ) -> Result<SessionGuard<'_>, AgentError> {
        let mut slot = self.slot.lock();
        loop {
            if slot.removed {
                return Err(AgentError::SessionNotFound);
            }
            if slot.record.status.persisting_revision.is_none() {
                return Ok(SessionGuard { cell: self, slot });
            }
            self.persisted.blocking.wait(&mut slot);
        }
    }

    /// Waits asynchronously until `ready` returns a value: for a staged
    /// commit to settle while one is staged, otherwise for a terminal
    /// transition.
    pub(in crate::service) async fn wait_until<T>(
        &self,
        mut ready: impl FnMut(&SessionRecord) -> Result<Option<T>, AgentError>,
    ) -> Result<T, AgentError> {
        let mut persisted = self.persisted.epoch.subscribe();
        let mut terminal = self.terminal.epoch.subscribe();
        loop {
            let staged = {
                let guard = self.lock()?;
                if let Some(value) = ready(&guard)? {
                    return Ok(value);
                }
                guard.status.persisting_revision.is_some()
            };
            let _ = if staged {
                persisted.changed().await
            } else {
                terminal.changed().await
            };
        }
    }

    /// Like `wait_until`, blocking the calling thread until `deadline`
    /// (`Ok(None)` when it passes). Only for callers off the async runtime.
    pub(in crate::service) fn wait_until_blocking<T>(
        &self,
        deadline: Instant,
        mut ready: impl FnMut(&SessionRecord) -> Result<Option<T>, AgentError>,
    ) -> Result<Option<T>, AgentError> {
        let mut slot = self.slot.lock();
        loop {
            if slot.removed {
                return Err(AgentError::SessionNotFound);
            }
            if let Some(value) = ready(&slot.record)? {
                return Ok(Some(value));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            let signal = if slot.record.status.persisting_revision.is_some() {
                &self.persisted
            } else {
                &self.terminal
            };
            signal.blocking.wait_for(&mut slot, remaining);
        }
    }

    /// Stages `commit` now when no commit is staged, returning the
    /// checkpoint to append; otherwise defers it until the staged commit
    /// settles, and whatever settles it stages it. Never waits. `None` when
    /// deferred, when the session is gone or when nothing applies.
    pub(in crate::service) fn stage_or_defer(
        self: &Arc<Self>,
        commit: DeferredCommit,
    ) -> Option<AgentHistoryFollowUp> {
        let mut guard = self.lock().ok()?;
        if guard.status.persisting_revision.is_some() {
            guard.slot.deferred.push_back(commit);
            return None;
        }
        commit(self, &mut guard)
    }

    /// Advances `persisted`: the staged commit settled. The change must
    /// already be made under the cell's lock; the lock may still be held.
    pub(in crate::service) fn notify_persisted(&self) {
        self.persisted.notify();
    }

    /// Advances `terminal`: a turn or compaction reached a terminal state.
    /// The change must already be made under the cell's lock.
    pub(in crate::service) fn notify_terminal(&self) {
        self.terminal.notify();
    }
}

#[cfg(test)]
mod tests;
