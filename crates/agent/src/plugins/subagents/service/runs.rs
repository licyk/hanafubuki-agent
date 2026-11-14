// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core/src/agent/status.rs` (an agent's status) at commit
// `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: an agent's statuses are those of its
// latest turn, which this Agent instance ran or the kernel recorded; a
// final status is what its parent's next wait reports.

//! The agents this Agent instance runs ([`SubagentRuns`]), each with the
//! status the model reads, and how a run's end is recorded.

use std::collections::HashMap;
use std::sync::Arc;

use hanafubuki_agent_kernel::{AgentError, AgentSessionId, AgentTurnOutcome, AgentTurnPhase};
use parking_lot::Mutex;
use tokio::sync::Notify;

use super::ListedAgent;

/// An agent's status, as the model reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::plugins::subagents) enum AgentRunStatus {
    /// Spawned, but no turn of it is known.
    PendingInit,
    Running,
    /// Its turn was interrupted; it takes messages and follow-up tasks.
    Interrupted,
    /// Its turn ended with this final answer, when it is known.
    Completed(Option<String>),
    Errored(String),
}

impl AgentRunStatus {
    /// What the latest turn of an agent this instance did not run says.
    pub(super) fn of_phase(phase: Option<AgentTurnPhase>) -> Self {
        match phase {
            None => Self::PendingInit,
            Some(phase) if !phase.is_terminal() => Self::Running,
            Some(AgentTurnPhase::Completed) => Self::Completed(None),
            Some(AgentTurnPhase::Failed) => Self::Errored("its last turn failed".to_string()),
            Some(_) => Self::Interrupted,
        }
    }

    pub(super) fn of_turn(outcome: Result<Result<AgentTurnOutcome, AgentError>, String>) -> Self {
        match outcome {
            Ok(Ok(outcome)) => Self::Completed(Some(outcome.final_answer)),
            Ok(Err(AgentError::Cancelled)) => Self::Interrupted,
            Ok(Err(error)) => Self::Errored(error.to_string()),
            Err(panic) => {
                log::error!("a subagent's turn panicked: {panic}");
                Self::Errored("the turn failed with an internal error".to_string())
            }
        }
    }

    /// Whether the parent's next wait reports it: Codex's final statuses.
    fn is_final(&self) -> bool {
        matches!(self, Self::Completed(_) | Self::Errored(_))
    }
}

/// The agents this Agent instance runs, kept across the plugin's restarts.
#[derive(Clone, Default)]
pub(crate) struct SubagentRuns {
    pub(super) runs: Arc<Mutex<HashMap<AgentSessionId, AgentRun>>>,
    pub(super) changed: Arc<Notify>,
}

pub(super) struct AgentRun {
    pub(super) parent: AgentSessionId,
    name: String,
    role: String,
    pub(super) status: AgentRunStatus,
    /// Messages for its next turn.
    pub(super) queued: Vec<String>,
    /// A final status the parent's waits have not reported yet.
    pub(super) unreported: bool,
    /// How many turns this instance started on it.
    pub(super) turn: u64,
}

impl SubagentRuns {
    pub(super) fn running(&self, parent: AgentSessionId) -> usize {
        self.runs
            .lock()
            .values()
            .filter(|run| run.parent == parent && run.status == AgentRunStatus::Running)
            .count()
    }

    pub(super) fn status(&self, child: AgentSessionId) -> Option<AgentRunStatus> {
        self.runs.lock().get(&child).map(|run| run.status.clone())
    }

    /// Records how `child`'s `turn`-th run ended, unless a later run
    /// started since.
    pub(super) fn finish(&self, child: AgentSessionId, turn: u64, status: AgentRunStatus) {
        if let Some(run) = self
            .runs
            .lock()
            .get_mut(&child)
            .filter(|run| run.turn == turn)
        {
            run.unreported = status.is_final();
            run.status = status;
        }
        self.changed.notify_waiters();
    }

    /// The final statuses of `parent`'s agents not reported yet, which are
    /// now reported, and how many of its agents still run.
    pub(super) fn take_updates(&self, parent: AgentSessionId) -> (Vec<ListedAgent>, usize) {
        let mut runs = self.runs.lock();
        let mut updates = Vec::new();
        let mut running = 0;
        for (id, run) in runs.iter_mut().filter(|(_, run)| run.parent == parent) {
            running += usize::from(run.status == AgentRunStatus::Running);
            if run.unreported {
                run.unreported = false;
                updates.push(ListedAgent {
                    name: run.name.clone(),
                    id: *id,
                    role: run.role.clone(),
                    status: run.status.clone(),
                });
            }
        }
        updates.sort_by_key(|update| update.id);
        (updates, running)
    }
}

impl AgentRun {
    pub(super) fn new(parent: AgentSessionId, agent: ListedAgent) -> Self {
        Self {
            parent,
            name: agent.name,
            role: agent.role,
            status: agent.status,
            queued: Vec::new(),
            unreported: false,
            turn: 0,
        }
    }
}
