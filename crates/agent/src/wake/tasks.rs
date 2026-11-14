//! What a session's background work holds open, as its host reads it: the
//! counts ([`AgentBackgroundStatus`]) and a root's tree in full
//! ([`AgentBackgroundTasks`]). Both read the runs this instance keeps
//! before the session's queue: an agent counts as running from before
//! `spawn_agent` returns until its end is queued, so one that ended is
//! running or queued in what either answers, never neither.

use hanafubuki_agent_execution::AgentBackgroundTerminalSnapshot;
use hanafubuki_agent_kernel::{AgentBackgroundWork, AgentError, AgentSessionId, AgentWakePolicy};
use serde::Serialize;

use super::{AgentWakeState, Wakes};
use crate::plugins::AgentSubagentStatus;
use crate::session_driver::AgentSessionDriver;

/// What a session's background work holds open, and what came of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBackgroundStatus {
    /// Its sub-agents still working (terminals count from R5).
    pub work: AgentBackgroundWork,
    /// The events queued on it that no turn has delivered yet.
    pub pending_wake_events: usize,
    pub wake: AgentWakeState,
}

/// A root's background work in full: what a task manager lists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBackgroundTasks {
    /// The sub-agents of its tree, oldest first.
    pub agents: Vec<AgentSubagentStatus>,
    /// The background terminals of its tree (none are listed yet).
    pub terminals: Vec<AgentBackgroundTerminal>,
    /// The events queued on the root that no turn has delivered yet.
    pub pending_wake_events: usize,
    pub wake: AgentWakeState,
}

/// A background terminal of a session in a tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBackgroundTerminal {
    /// The session whose `exec_command` started it.
    pub owner: AgentSessionId,
    pub owner_title: Option<String>,
    pub terminal: AgentBackgroundTerminalSnapshot,
}

impl AgentBackgroundTasks {
    /// A root with nothing in the background, as a host knows it before it
    /// is told anything.
    pub(super) fn idle() -> Self {
        Self {
            agents: Vec::new(),
            terminals: Vec::new(),
            pending_wake_events: 0,
            wake: AgentWakeState::On,
        }
    }
}

impl Wakes {
    /// `session_id`'s counts. Blocks.
    pub(crate) fn status(
        &self,
        driver: &AgentSessionDriver,
        session_id: AgentSessionId,
    ) -> Result<AgentBackgroundStatus, AgentError> {
        let agents = self.runs.running(session_id);
        let (pending_wake_events, wake) = self.queue(driver, session_id)?;
        Ok(AgentBackgroundStatus {
            work: AgentBackgroundWork {
                agents,
                terminals: 0,
            },
            pending_wake_events,
            wake,
        })
    }

    /// `root`'s tree, its agents with their last answers when `answers`
    /// says so. Blocks.
    pub(crate) fn tasks(
        &self,
        driver: &AgentSessionDriver,
        root: AgentSessionId,
        answers: bool,
    ) -> Result<AgentBackgroundTasks, AgentError> {
        let agents = self.runs.statuses(driver, root, answers)?;
        let (pending_wake_events, wake) = self.queue(driver, root)?;
        Ok(AgentBackgroundTasks {
            agents,
            terminals: Vec::new(),
            pending_wake_events,
            wake,
        })
    }

    /// What is queued on `session_id`, and whether it is woken for it.
    fn queue(
        &self,
        driver: &AgentSessionDriver,
        session_id: AgentSessionId,
    ) -> Result<(usize, AgentWakeState), AgentError> {
        let session = driver.snapshot(session_id)?;
        let wake =
            if session.wake_policy == AgentWakePolicy::Off || !driver.wakes_on_background_tasks() {
                AgentWakeState::Off
            } else {
                match self.pauses.pause(session_id) {
                    Some(reason) => AgentWakeState::Paused { reason },
                    None => AgentWakeState::On,
                }
            };
        Ok((session.pending_wake_events, wake))
    }
}

/// The session at the top of `session_id`'s lineage: the session itself
/// when it has no parent. Blocks.
pub(crate) fn root_of(
    driver: &AgentSessionDriver,
    session_id: AgentSessionId,
) -> Result<AgentSessionId, AgentError> {
    let mut session = session_id;
    while let Some(parent) = driver.snapshot(session)?.parent_session_id {
        session = parent;
    }
    Ok(session)
}
