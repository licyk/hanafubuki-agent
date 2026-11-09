//! Durable commits: staged on the record, appended, and only then visible.
//!
//! A commit is staged on the session record under its cell's lock (the
//! record's `persisting_revision` holds every other writer back) and
//! appended to history. It settles when the append is durable, publishing
//! the persisted revision, or failed, rolling the record back; only then may
//! the caller make it visible.
//!
//! The turn loop queues its appends (`persist_staged`) and awaits their
//! acknowledgement, so it never parks its async worker on the history
//! writer. The settlement runs inside the acknowledgement, where the store
//! completes the append (its writer), not in the awaiting future: it happens
//! even if the turn is aborted meanwhile, and a thread blocked on the session
//! is released without the turn's runtime making progress. An
//! acknowledgement the store drops unacknowledged rolls the record back.
//! Synchronous callers append on their own thread (`persist_staged_blocking`).
//!
//! A drop guard cannot await. It hands its commit to the store with nothing
//! awaiting it (`StagedPersistence::into_follow_up`): the acknowledgement
//! publishes it and then makes it visible itself, on the store's thread. If a
//! commit was staged when the guard dropped, the guard's commit is deferred
//! on the cell and the settlement of the staged one stages it and hands it
//! back to the store as a follow-up.

use super::*;
use crate::{AgentHistoryAppendAck, AgentHistoryFollowUp};

/// One staged durable mutation: what to append, and the record state to
/// restore if the append fails.
pub(super) struct StagedPersistence {
    cell: Arc<SessionCell>,
    committed_revision: u64,
    previous: SessionRollbackState,
    append: StagedAppend,
}

enum StagedAppend {
    Commit(Box<AgentConversationCommit>),
    Delta(AgentConversationDelta),
}

impl StagedAppend {
    /// Starts the append's trace.
    fn trace(&self) -> AppendTrace {
        match self {
            Self::Commit(commit) => AppendTrace::commit(commit),
            Self::Delta(delta) => AppendTrace::delta(delta),
        }
    }
}

impl StagedPersistence {
    /// Stages a checkpoint of `record`, the record `cell` holds; `previous`
    /// is its state before the mutation.
    pub(super) fn commit(
        cell: &Arc<SessionCell>,
        record: &mut SessionRecord,
        previous: SessionRollbackState,
    ) -> Self {
        Self {
            cell: cell.clone(),
            committed_revision: record.status.revision,
            previous,
            append: StagedAppend::Commit(Box::new(conversation_commit(cell.session_id(), record))),
        }
    }

    /// Stages the history mutations of `record` without a checkpoint.
    pub(super) fn delta(
        cell: &Arc<SessionCell>,
        record: &mut SessionRecord,
        previous: SessionRollbackState,
    ) -> Self {
        Self {
            cell: cell.clone(),
            committed_revision: record.status.revision,
            previous,
            append: StagedAppend::Delta(conversation_delta(cell.session_id(), record)),
        }
    }

    /// This checkpoint as an append nothing awaits: its acknowledgement
    /// publishes the revision, or rolls the record back, and once it is
    /// durable runs `visible` on the thread that acknowledged it (the history
    /// writer). For a drop guard's terminal commit, handed to the store
    /// directly (`AgentService::hand_off`) or as a deferred commit's
    /// follow-up.
    pub(super) fn into_follow_up(
        self,
        visible: impl FnOnce() + Send + 'static,
    ) -> AgentHistoryFollowUp {
        // Both callers (a turn's and a compaction's drop-guard terminal) stage
        // with `StagedPersistence::commit`, never `delta`.
        let StagedAppend::Commit(commit) = self.append else {
            unreachable!("only a checkpoint is handed off without an awaiting caller");
        };
        let settlement = Settlement {
            trace: AppendTrace::commit(&commit),
            cell: self.cell,
            committed_revision: self.committed_revision,
            previous: Some(self.previous),
            outcome: Some(SettlementOutcome::Visible(Box::new(visible))),
        };
        AgentHistoryFollowUp {
            commit: *commit,
            ack: AgentHistoryAppendAck::new(move |appended| settlement.settle(appended)),
        }
    }
}

/// Settles one staged commit with its append's outcome. Dropped unsettled
/// (the store dropped the acknowledgement), it rolls the record back.
struct Settlement {
    cell: Arc<SessionCell>,
    committed_revision: u64,
    previous: Option<SessionRollbackState>,
    trace: AppendTrace,
    outcome: Option<SettlementOutcome>,
}

/// Who learns that a commit settled.
enum SettlementOutcome {
    /// The caller awaiting the commit, which makes it visible.
    Awaited(tokio::sync::oneshot::Sender<Result<(), AgentError>>),
    /// Nobody awaits it: this makes it visible once it is durable.
    Visible(Box<dyn FnOnce() + Send>),
}

impl Settlement {
    /// Settles the commit and returns the deferred commit staged in its
    /// place, for the store to append next.
    fn settle(
        mut self,
        appended: Result<(), AgentHistoryStoreError>,
    ) -> Option<AgentHistoryFollowUp> {
        self.trace.end(appended.is_ok());
        let previous = self.previous.take()?;
        let (result, follow_up) = settle(&self.cell, self.committed_revision, previous, appended);
        match self.outcome.take() {
            Some(SettlementOutcome::Awaited(sender)) => {
                let _ = sender.send(result);
            }
            Some(SettlementOutcome::Visible(visible)) => match result {
                Ok(()) => visible(),
                Err(error) => log::warn!(
                    "agent_diag stage=history.hand_off session_id={} revision={} outcome=error error={}",
                    self.cell.session_id(),
                    self.committed_revision,
                    bounded_diagnostic(error.to_string())
                ),
            },
            None => {}
        }
        follow_up
    }
}

impl Drop for Settlement {
    fn drop(&mut self) {
        let Some(previous) = self.previous.take() else {
            return;
        };
        self.trace.end(false);
        // The store stopped appending, so a deferred commit staged in this
        // one's place fails (and rolls back) the same way.
        if let Some(follow_up) = restore_if_revision(&self.cell, self.committed_revision, previous)
        {
            follow_up.ack.fail(AgentHistoryStoreError::ShuttingDown);
        }
        if let Some(SettlementOutcome::Awaited(sender)) = self.outcome.take() {
            let _ = sender.send(Err(history_store_error(
                AgentHistoryStoreError::ShuttingDown,
            )));
        }
    }
}

impl AgentService {
    /// Queues `staged` and awaits its acknowledgement. On success the commit
    /// is durable and its revision published, and the caller may make it
    /// visible; on failure the record is already rolled back.
    pub(super) async fn persist_staged(&self, staged: StagedPersistence) -> Result<(), AgentError> {
        let StagedPersistence {
            cell,
            committed_revision,
            previous,
            append,
        } = staged;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let settlement = Settlement {
            cell,
            committed_revision,
            previous: Some(previous),
            trace: append.trace(),
            outcome: Some(SettlementOutcome::Awaited(sender)),
        };
        let ack = AgentHistoryAppendAck::new(move |appended| settlement.settle(appended));
        match append {
            StagedAppend::Commit(commit) => self.inner.history_store.enqueue_commit(*commit, ack),
            StagedAppend::Delta(delta) => self.inner.history_store.enqueue_delta(delta, ack),
        }
        receiver
            .await
            .unwrap_or_else(|_| Err(history_store_error(AgentHistoryStoreError::ShuttingDown)))
    }

    /// Appends `staged` on the calling thread, then publishes its revision or
    /// rolls the record back.
    pub(super) fn persist_staged_blocking(
        &self,
        staged: StagedPersistence,
    ) -> Result<(), AgentError> {
        let trace = staged.append.trace();
        let appended = match staged.append {
            StagedAppend::Commit(commit) => self.inner.history_store.append_commit(*commit),
            StagedAppend::Delta(delta) => self.inner.history_store.append_delta(delta),
        };
        trace.end(appended.is_ok());
        let (result, follow_up) = settle(
            &staged.cell,
            staged.committed_revision,
            staged.previous,
            appended,
        );
        self.hand_off(follow_up);
        result
    }
}

/// Publishes the staged commit's revision, or rolls the record back, and
/// returns the deferred commit staged in its place.
fn settle(
    cell: &Arc<SessionCell>,
    committed_revision: u64,
    previous: SessionRollbackState,
    appended: Result<(), AgentHistoryStoreError>,
) -> (Result<(), AgentError>, Option<AgentHistoryFollowUp>) {
    match appended {
        Ok(()) => (Ok(()), publish_persisted_revision(cell, committed_revision)),
        Err(error) => (
            Err(history_store_error(error)),
            restore_if_revision(cell, committed_revision, previous),
        ),
    }
}

/// The diagnostic trace of one append, logged when it starts and when it
/// settles.
enum AppendTrace {
    Commit {
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
        turn_phase: &'static str,
        started: Instant,
    },
    Delta {
        session_id: AgentSessionId,
        revision: AgentRevision,
        started: Instant,
    },
}

impl AppendTrace {
    fn commit(commit: &AgentConversationCommit) -> Self {
        let session_id = commit.runtime.id;
        let generation = commit.runtime.generation;
        let revision = commit.runtime.revision;
        let turn_phase = commit
            .runtime
            .turn
            .as_ref()
            .map_or("none", |turn| agent_turn_phase_name(turn.phase));
        log::debug!(
            "agent_diag stage=history.append.begin session_id={} generation={} revision={} turn_phase={} mutations={}",
            session_id,
            generation,
            revision,
            turn_phase,
            commit.history_mutations.len()
        );
        Self::Commit {
            session_id,
            generation,
            revision,
            turn_phase,
            started: Instant::now(),
        }
    }

    fn delta(delta: &AgentConversationDelta) -> Self {
        log::debug!(
            "agent_diag stage=history.append_delta.begin session_id={} revision={} mutations={}",
            delta.session_id,
            delta.session_revision,
            delta.history_mutations.len()
        );
        Self::Delta {
            session_id: delta.session_id,
            revision: delta.session_revision,
            started: Instant::now(),
        }
    }

    fn end(&self, appended: bool) {
        let outcome = if appended { "ok" } else { "error" };
        match self {
            Self::Commit {
                session_id,
                generation,
                revision,
                turn_phase,
                started,
            } => log::debug!(
                "agent_diag stage=history.append.end session_id={} generation={} revision={} turn_phase={} outcome={} elapsed_ms={}",
                session_id,
                generation,
                revision,
                turn_phase,
                outcome,
                started.elapsed().as_millis()
            ),
            Self::Delta {
                session_id,
                revision,
                started,
            } => log::debug!(
                "agent_diag stage=history.append_delta.end session_id={} revision={} outcome={} elapsed_ms={}",
                session_id,
                revision,
                outcome,
                started.elapsed().as_millis()
            ),
        }
    }
}

fn agent_turn_phase_name(phase: AgentTurnPhase) -> &'static str {
    match phase {
        AgentTurnPhase::Sampling => "sampling",
        AgentTurnPhase::RunningTools => "running_tools",
        AgentTurnPhase::WaitingForUserInput => "waiting_for_user_input",
        AgentTurnPhase::Completed => "completed",
        AgentTurnPhase::Cancelled => "cancelled",
        AgentTurnPhase::Failed => "failed",
        AgentTurnPhase::Interrupted => "interrupted",
    }
}
