//! Execution progress: queued per session, committed in order.
//!
//! A tool's progress sink reports synchronously, from inside the tool's call
//! on the turn's async worker or from a plugin's monitor thread, so `report`
//! only queues the update on the session's cell. One consumer task per
//! session applies and commits the queued updates in order, awaiting each
//! append; it is spawned, on the runtime of the turn that created the sink,
//! when an update finds no consumer running (or by a flush, on its runtime,
//! when the last consumer stopped with another runtime), and ends when the
//! queue is empty. The queue is closed, and its consumer aborted, when the session is
//! removed and when the kernel shuts down its history, so no consumer
//! outlives either.
//!
//! The queue is bounded. When it is full, an output update is coalesced into
//! the queued output of its execution, if nothing of that execution is
//! queued after it: its text is appended, keeping the newest bytes. Every
//! other update (an execution's start, status, disposal and cleanup failure)
//! is queued even then, so the queue holds at most its capacity plus those
//! and one output after each.
//!
//! A turn awaits every update queued before its terminal commit
//! (`flush_execution_progress`), so what its calls reported is committed
//! first. A dropped turn cannot await: updates still queued then are
//! committed after its cancellation, still in order. The application's own
//! synchronous reports (`report_session_execution_progress`) wait for the
//! queued updates before them and commit on the calling thread.

use hanafubuki_agent_foundation::terminal_text::aligned_start;
use parking_lot::{Condvar, Mutex};
use tokio::sync::watch;

use super::*;
use crate::AgentToolProgressUpdate;

/// Queued updates past which an output is coalesced instead of queued.
const EXECUTION_PROGRESS_QUEUE_CAPACITY: usize = 64;
/// The text a coalesced output keeps: its newest bytes.
const MAXIMUM_COALESCED_OUTPUT_BYTES: usize = 16 * 1024;
/// How long a synchronous report waits for the queued progress before it.
/// The consumer runs as long as the turn's runtime, so this bounds the wait
/// only once that runtime is gone.
const SYNCHRONOUS_REPORT_FLUSH: Duration = Duration::from_secs(5);

/// One session's queued execution progress.
pub(in crate::service) struct ExecutionProgressQueue {
    state: Mutex<QueueState>,
    /// The sequence of the last entry applied, for async flushes.
    applied: watch::Sender<u64>,
    /// Signalled with `applied`, for blocking flushes (shutdown).
    applied_blocking: Condvar,
}

#[derive(Default)]
struct QueueState {
    pending: VecDeque<QueuedProgress>,
    /// Entries queued so far; an entry's sequence is its place in that order.
    queued: u64,
    /// The sequence of the last entry the consumer took off the queue.
    taken: u64,
    /// The sequence of the last entry applied (committed, or failed).
    applied: u64,
    /// The running consumer, by id, with its task once it is spawned.
    consumer: Option<(u64, Option<tokio::task::AbortHandle>)>,
    next_consumer: u64,
    closed: bool,
}

struct QueuedProgress {
    sequence: u64,
    /// The call whose sink reported it.
    call_id: Option<String>,
    update: AgentToolProgressUpdate,
}

impl QueueState {
    /// Reserves the consumer slot for a new consumer, unless one runs.
    fn reserve_consumer(&mut self) -> Option<u64> {
        if self.consumer.is_some() {
            return None;
        }
        let id = self.next_consumer;
        self.next_consumer += 1;
        self.consumer = Some((id, None));
        Some(id)
    }
}

impl Default for ExecutionProgressQueue {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            applied: watch::Sender::new(0),
            applied_blocking: Condvar::new(),
        }
    }
}

impl ExecutionProgressQueue {
    /// Queues `update`, coalescing an output when the queue is full, and
    /// starts a consumer with `spawn` when none runs. `false` once closed.
    pub(in crate::service) fn report(
        &self,
        call_id: Option<String>,
        update: AgentToolProgressUpdate,
        spawn: impl FnOnce(u64) -> tokio::task::AbortHandle,
    ) -> bool {
        let start = {
            let mut state = self.state.lock();
            if state.closed {
                return false;
            }
            let update = if state.pending.len() >= EXECUTION_PROGRESS_QUEUE_CAPACITY {
                coalesce_output(&mut state.pending, update)
            } else {
                Some(update)
            };
            if let Some(update) = update {
                state.queued += 1;
                let sequence = state.queued;
                state.pending.push_back(QueuedProgress {
                    sequence,
                    call_id,
                    update,
                });
            }
            state.reserve_consumer()
        };
        self.start_consumer(start, spawn);
        true
    }

    /// Starts a consumer with `spawn` when updates are queued and none runs
    /// (the last one stopped with its runtime).
    fn resume(&self, spawn: impl FnOnce(u64) -> tokio::task::AbortHandle) {
        let start = {
            let mut state = self.state.lock();
            if state.closed || state.pending.is_empty() {
                return;
            }
            state.reserve_consumer()
        };
        self.start_consumer(start, spawn);
    }

    /// Spawns the consumer reserved as `start`, with the lock released: a
    /// runtime that has shut down drops the consumer inside `spawn`, and its
    /// guard takes the lock.
    fn start_consumer(
        &self,
        start: Option<u64>,
        spawn: impl FnOnce(u64) -> tokio::task::AbortHandle,
    ) {
        let Some(id) = start else {
            return;
        };
        let task = spawn(id);
        let mut state = self.state.lock();
        match &mut state.consumer {
            Some((current, slot @ None)) if *current == id => *slot = Some(task),
            // It ended, or the queue was closed, before this.
            _ => {}
        }
    }

    /// The next update for consumer `id`, or `None` once the queue is empty,
    /// which ends the consumer.
    fn take(&self, id: u64) -> Option<QueuedProgress> {
        let mut state = self.state.lock();
        match state.pending.pop_front() {
            Some(queued) => {
                state.taken = queued.sequence;
                Some(queued)
            }
            None => {
                if state
                    .consumer
                    .as_ref()
                    .is_some_and(|(current, _)| *current == id)
                {
                    state.consumer = None;
                }
                None
            }
        }
    }

    /// Entry `sequence` is applied.
    fn applied(&self, sequence: u64) {
        let mut state = self.state.lock();
        state.applied = state.applied.max(sequence);
        self.notify(state.applied);
    }

    /// Consumer `id` stopped before the queue was empty (aborted, or its
    /// runtime shut down): the entry it took settles on its own, and a later
    /// report starts a new consumer.
    fn consumer_stopped(&self, id: u64) {
        let mut state = self.state.lock();
        if state
            .consumer
            .as_ref()
            .is_some_and(|(current, _)| *current == id)
        {
            state.consumer = None;
        }
        state.applied = state.applied.max(state.taken);
        self.notify(state.applied);
    }

    /// Refuses later reports, drops what is queued, aborts the consumer and
    /// releases every flush.
    pub(in crate::service) fn close(&self) {
        let mut state = self.state.lock();
        state.closed = true;
        state.pending.clear();
        if let Some((_, Some(consumer))) = state.consumer.take() {
            consumer.abort();
        }
        state.applied = state.queued;
        self.notify(state.applied);
    }

    /// Waits until every update queued before the call is applied.
    pub(in crate::service) async fn flushed(&self) {
        let mut applied = self.applied.subscribe();
        let target = self.state.lock().queued;
        // The sender lives as long as the queue.
        let _ = applied.wait_for(|applied| *applied >= target).await;
    }

    /// `flushed`, blocking the calling thread until `deadline`; `false` when
    /// it passed first. Off the async runtime only.
    pub(in crate::service) fn flushed_blocking(&self, deadline: Instant) -> bool {
        let mut state = self.state.lock();
        let target = state.queued;
        while state.applied < target {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            self.applied_blocking.wait_for(&mut state, remaining);
        }
        true
    }

    fn notify(&self, applied: u64) {
        self.applied.send_replace(applied);
        self.applied_blocking.notify_all();
    }
}

/// Folds `update`, when it is an output, into the queued output of its
/// execution, if that is the last entry of the execution queued; otherwise
/// hands it back to be queued. One execution reports through one call's
/// sink, so the queued entry's call is the update's.
fn coalesce_output(
    pending: &mut VecDeque<QueuedProgress>,
    update: AgentToolProgressUpdate,
) -> Option<AgentToolProgressUpdate> {
    let AgentToolProgressUpdate::ExecutionOutput {
        execution_id,
        generation,
        text: newer_text,
        next_offset: newer_offset,
        truncated: newer_truncated,
    } = update
    else {
        return Some(update);
    };
    let latest = pending
        .iter_mut()
        .rev()
        .find(|queued| execution_of(&queued.update) == (execution_id, generation));
    let Some(QueuedProgress {
        update:
            AgentToolProgressUpdate::ExecutionOutput {
                text,
                next_offset,
                truncated,
                ..
            },
        ..
    }) = latest
    else {
        return Some(AgentToolProgressUpdate::ExecutionOutput {
            execution_id,
            generation,
            text: newer_text,
            next_offset: newer_offset,
            truncated: newer_truncated,
        });
    };
    if newer_truncated {
        // Output was skipped before the newer text: the queued text falls
        // into that gap.
        *text = newer_text;
        *truncated = true;
    } else {
        text.push_str(&newer_text);
    }
    if text.len() > MAXIMUM_COALESCED_OUTPUT_BYTES {
        let cut = text.len() - MAXIMUM_COALESCED_OUTPUT_BYTES;
        let start = aligned_start(text.as_bytes(), cut);
        text.drain(..start);
        *truncated = true;
    }
    *next_offset = newer_offset;
    None
}

/// The execution an update is about.
fn execution_of(update: &AgentToolProgressUpdate) -> (u64, u64) {
    match update {
        AgentToolProgressUpdate::ExecutionStarted {
            execution_id,
            generation,
            ..
        }
        | AgentToolProgressUpdate::ExecutionOutput {
            execution_id,
            generation,
            ..
        }
        | AgentToolProgressUpdate::ExecutionStatus {
            execution_id,
            generation,
            ..
        }
        | AgentToolProgressUpdate::ExecutionDisposed {
            execution_id,
            generation,
        }
        | AgentToolProgressUpdate::ExecutionCleanupFailed {
            execution_id,
            generation,
        } => (*execution_id, *generation),
    }
}

/// One session's consumer task. Dropped before the queue was empty (aborted,
/// or its runtime shut down, even before it first ran), it frees the
/// session's consumer slot for the next report.
struct ProgressConsumer {
    cell: Arc<SessionCell>,
    id: u64,
    drained: bool,
}

impl Drop for ProgressConsumer {
    fn drop(&mut self) {
        if !self.drained {
            self.cell.progress().consumer_stopped(self.id);
        }
    }
}

/// The progress sink of one tool call: it queues on the call's session,
/// stamped with the call.
pub(super) struct SessionToolProgressSink {
    pub(super) service: AgentService,
    pub(super) session_id: AgentSessionId,
    pub(super) call_id: Option<String>,
    /// The runtime of the call's turn, which runs the session's consumer.
    pub(super) runtime: tokio::runtime::Handle,
}

impl crate::AgentToolProgressSink for SessionToolProgressSink {
    fn report(&self, update: AgentToolProgressUpdate) {
        self.service.queue_execution_progress(
            self.session_id,
            self.call_id.clone(),
            update,
            &self.runtime,
        );
    }
}

impl AgentService {
    /// Queues `update` on the session's progress queue; never waits.
    pub(super) fn queue_execution_progress(
        &self,
        session_id: AgentSessionId,
        call_id: Option<String>,
        update: AgentToolProgressUpdate,
        runtime: &tokio::runtime::Handle,
    ) {
        let queued = self.inner.sessions.find(session_id).is_some_and(|cell| {
            cell.progress().report(call_id, update, |id| {
                self.spawn_progress_consumer(&cell, id, runtime)
            })
        });
        if !queued {
            log::debug!(
                "agent_diag stage=execution.progress.queue session_id={session_id} outcome=session_closed"
            );
        }
    }

    fn spawn_progress_consumer(
        &self,
        cell: &Arc<SessionCell>,
        id: u64,
        runtime: &tokio::runtime::Handle,
    ) -> tokio::task::AbortHandle {
        let consumer = ProgressConsumer {
            cell: cell.clone(),
            id,
            drained: false,
        };
        runtime
            .spawn(self.clone().drain_execution_progress(consumer))
            .abort_handle()
    }

    /// Applies and commits the queued updates in order until the queue is
    /// empty.
    async fn drain_execution_progress(self, mut consumer: ProgressConsumer) {
        let cell = consumer.cell.clone();
        while let Some(queued) = cell.progress().take(consumer.id) {
            if let Err(error) = self
                .commit_execution_progress(&cell, queued.call_id, queued.update)
                .await
            {
                log::warn!(
                    "agent_diag stage=execution.progress.persist session_id={} outcome=error error={}",
                    cell.session_id(),
                    bounded_diagnostic(error.to_string())
                );
            }
            cell.progress().applied(queued.sequence);
        }
        consumer.drained = true;
    }

    /// Applies one update to the session's activities and commits it,
    /// awaiting the append.
    async fn commit_execution_progress(
        &self,
        cell: &Arc<SessionCell>,
        call_id: Option<String>,
        update: AgentToolProgressUpdate,
    ) -> Result<(), AgentError> {
        let staged = {
            let mut record = cell.lock_after_persistence().await?;
            self.stage_execution_progress(cell, &mut record, call_id, update)
        };
        let Some((staged, event)) = staged else {
            return Ok(());
        };
        self.persist_staged(staged).await?;
        self.emit_session_event(cell.session_id(), event);
        Ok(())
    }

    /// Commits `update` on the calling thread, for the application's own
    /// reports off the async runtime (a background terminal's stop, startup
    /// recovery). It first waits, for `SYNCHRONOUS_REPORT_FLUSH` at most, for
    /// the progress the session's sinks queued before it, so an execution's
    /// last output and status precede the application's disposal of it.
    pub fn report_session_execution_progress(
        &self,
        session_id: AgentSessionId,
        update: AgentToolProgressUpdate,
    ) -> Result<(), AgentError> {
        let cell = self.session(session_id)?;
        if !cell
            .progress()
            .flushed_blocking(Instant::now() + SYNCHRONOUS_REPORT_FLUSH)
        {
            log::warn!(
                "agent_diag stage=execution.progress.flush session_id={session_id} outcome=deadline_elapsed"
            );
        }
        let staged = {
            let mut record = cell.lock_after_persistence_blocking()?;
            self.stage_execution_progress(&cell, &mut record, None, update)
        };
        let Some((staged, event)) = staged else {
            return Ok(());
        };
        self.persist_staged_blocking(staged)?;
        self.emit_session_event(session_id, event);
        Ok(())
    }

    /// Stages `update` on the record: a checkpoint when it starts or ends an
    /// execution, a delta otherwise; nothing while the session is disposed.
    fn stage_execution_progress(
        &self,
        cell: &Arc<SessionCell>,
        record: &mut SessionRecord,
        call_id: Option<String>,
        update: AgentToolProgressUpdate,
    ) -> Option<(StagedPersistence, AgentEvent)> {
        if record.status.disposing {
            return None;
        }
        let checkpoint = matches!(
            &update,
            AgentToolProgressUpdate::ExecutionStarted { .. }
                | AgentToolProgressUpdate::ExecutionDisposed { .. }
                | AgentToolProgressUpdate::ExecutionCleanupFailed { .. }
        ) || matches!(
            &update,
            AgentToolProgressUpdate::ExecutionStatus { status, .. } if status.is_terminal()
        );
        let previous = record.rollback_state();
        let event = record_session_event(
            record,
            AgentEventKind::ExecutionProgress { call_id, update },
            self.inner.limits,
        );
        let staged = if checkpoint {
            StagedPersistence::commit(cell, record, previous)
        } else {
            StagedPersistence::delta(cell, record, previous)
        };
        Some((staged, event))
    }

    /// Emits a session-level event to the sink of the session's managed turn,
    /// or nowhere if it never had one.
    pub(in crate::service) fn emit_session_event(
        &self,
        session_id: AgentSessionId,
        event: AgentEvent,
    ) {
        let sink = self.inner.event_sinks.lock().get(&session_id).cloned();
        if let Some(sink) = sink {
            sink.emit(event);
        }
    }

    /// Waits until every update queued for the session so far is applied,
    /// resuming the consumer on this runtime if it stopped with another.
    pub(super) async fn flush_execution_progress(&self, cell: &Arc<SessionCell>) {
        let runtime = tokio::runtime::Handle::current();
        cell.progress()
            .resume(|id| self.spawn_progress_consumer(cell, id, &runtime));
        cell.progress().flushed().await;
    }
}

#[cfg(any(test, feature = "test-support"))]
impl AgentService {
    /// The progress sink the tool call `call_id` of `session_id` reports
    /// into, on the current runtime (test support).
    pub fn execution_progress_sink(
        &self,
        session_id: AgentSessionId,
        call_id: Option<String>,
    ) -> Arc<dyn crate::AgentToolProgressSink> {
        Arc::new(SessionToolProgressSink {
            service: self.clone(),
            session_id,
            call_id,
            runtime: tokio::runtime::Handle::current(),
        })
    }

    /// Waits until every update reported for `session_id` so far is applied
    /// (test support).
    pub async fn flush_session_execution_progress(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentError> {
        let cell = self.session(session_id)?;
        self.flush_execution_progress(&cell).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
