//! Compaction in the background, the `background` strategy (Codex's
//! `async_compaction`): between the background threshold and the
//! foreground one, a turn launches the session's own compaction of its
//! completed turns on a task the session owns, and the turn goes on. A
//! later checkpoint installs the result before the work added since
//! (`InstallMode::Rebase`), or waits for it when a compaction is required.
//!
//! The slot is not persisted, and it is not an active operation: nothing
//! waits for it but a required compaction, and a restart drops it.

use super::*;

mod adopt;
mod launch;

pub(in crate::service) use adopt::{AdoptedCompaction, BackgroundAdoption};

/// The session's background compaction: the inputs of the last one
/// launched, which are not launched again, and its run while there is one.
#[derive(Clone, Default)]
pub(in crate::service) struct BackgroundCompactionSlot {
    last_attempt: Option<BackgroundAttempt>,
    run: Option<Arc<BackgroundRun>>,
}

/// What a background compaction compacts and with what. A failed attempt is
/// not retried with the same inputs: the next one waits for another turn to
/// complete, another window, model or implementation.
#[derive(Clone, PartialEq, Eq)]
struct BackgroundAttempt {
    window_id: crate::AgentContextWindowId,
    /// The items before the current turn, which it compacts; within a
    /// window the history only grows, so they are its first items.
    compacted_items: usize,
    model: String,
    implementation: AgentCompactionImplementation,
}

/// One run, shared by `Arc`: a commit rolled back restores the record's
/// runtime, never what the task wrote, which lives here.
struct BackgroundRun {
    attempt: BackgroundAttempt,
    /// The launching turn's generation, recorded with the install.
    turn_generation: AgentGeneration,
    cancellation: CancellationToken,
    outcome: Mutex<BackgroundOutcome>,
}

enum BackgroundOutcome {
    Running,
    Ready(Box<(CompactionPlan, CompactionSample)>),
    /// Taken by a checkpoint that is installing it; another one waiting
    /// for the run waits for the install too.
    Installing,
    /// Installed by the checkpoint that took it, for one that waited.
    Installed(Box<AgentCompactionOutcome>),
    /// Failed, cancelled, or taken and not installed.
    Spent,
}

impl BackgroundOutcome {
    fn state(&self) -> Option<crate::AgentBackgroundCompactionState> {
        match self {
            Self::Running => Some(crate::AgentBackgroundCompactionState::Running),
            Self::Ready(_) | Self::Installing => Some(crate::AgentBackgroundCompactionState::Ready),
            Self::Installed(_) | Self::Spent => None,
        }
    }
}

impl BackgroundCompactionSlot {
    pub(in crate::service) fn state(&self) -> Option<crate::AgentBackgroundCompactionState> {
        self.run.as_ref()?.outcome.lock().state()
    }

    /// Cancels the run and forgets the slot, the last attempt included. A
    /// result the run still produces is discarded; its usage is recorded.
    pub(in crate::service) fn cancel(&mut self) {
        if let Some(run) = self.run.take() {
            run.cancellation.cancel();
        }
        self.last_attempt = None;
    }
}

impl BackgroundAttempt {
    /// Whether the session would still take this attempt's result: the same
    /// window, model and implementation.
    fn matches(&self, record: &SessionRecord) -> bool {
        self.window_id == record.runtime.context_window.window_id && self.same_model(record)
    }

    fn same_model(&self, record: &SessionRecord) -> bool {
        self.model == record.settings.config.model
            && self.implementation
                == AgentCompactionImplementation::from(record.settings.config.compaction_capability)
    }
}

/// Registered from the launch to the end of the task, which shutdown,
/// archive and delete wait for. A task dropped before its end leaves its
/// run spent.
struct BackgroundTaskGuard {
    service: AgentService,
    session_id: AgentSessionId,
    id: AgentCompactionId,
    run: Arc<BackgroundRun>,
}

impl Drop for BackgroundTaskGuard {
    fn drop(&mut self) {
        {
            let mut outcome = self.run.outcome.lock();
            if matches!(*outcome, BackgroundOutcome::Running) {
                *outcome = BackgroundOutcome::Spent;
            }
        }
        if let Some(cell) = self.service.inner.sessions.find(self.session_id) {
            cell.notify_terminal();
        }
        self.service
            .inner
            .managed_background
            .lock()
            .remove(&(self.session_id, self.id));
        self.service.inner.managed_background_changed.notify_all();
    }
}

impl AgentService {
    /// Waits, blocking, a few seconds at most for the background compaction
    /// of `session_id`, cancelled before its session is let go of, to end.
    /// Off the async runtime only.
    pub(in crate::service) fn await_cancelled_background_compaction(
        &self,
        session_id: AgentSessionId,
    ) {
        const DEADLINE: Duration = Duration::from_secs(5);
        self.wait_for_background_tasks(Some(session_id), Instant::now() + DEADLINE);
    }

    /// Waits, blocking, until no background compaction task of
    /// `session_id` (of any session when `None`) runs, or `deadline`
    /// passes; whether none does.
    pub(in crate::service) fn wait_for_background_tasks(
        &self,
        session_id: Option<AgentSessionId>,
        deadline: Instant,
    ) -> bool {
        let running = |tasks: &HashSet<(AgentSessionId, AgentCompactionId)>| {
            tasks
                .iter()
                .any(|(task_session, _)| session_id.is_none_or(|id| id == *task_session))
        };
        let mut tasks = self.inner.managed_background.lock();
        while running(&tasks) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            self.inner
                .managed_background_changed
                .wait_for(&mut tasks, remaining);
        }
        true
    }
}
