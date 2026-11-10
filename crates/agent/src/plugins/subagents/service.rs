// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core/src/tools/handlers/multi_agents_v2/` (what spawning,
// messaging, waiting for, listing and interrupting an agent do),
// `codex-rs/core/src/agent/status.rs` (an agent's status) and
// `codex-rs/core/src/config/mod.rs` (the limits and the wait bounds) at
// commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: an agent is a durable child session of
// origin `Subagent`, named by its title; there is no mailbox, so a finished
// turn is reported by the parent's next `wait_agent`, which returns the
// final answer, and agents never message their parent or each other; a
// message to a running agent steers its turn and one to an idle agent waits
// for its next follow-up task; at most 3 agents of one parent run at once
// (Codex's 4 threads per session less the root, counted by running turns
// since nothing is evicted), only a session without a parent spawns
// (Codex's `agents.max_depth` default), and a wait lasts at most 5 minutes
// (the turn's own tool timeout and deadline bound it).

//! What the subagent tools and the facade's subagent calls do, over the
//! session driver. The runs live in [`SubagentRuns`], which outlives the
//! plugin's restarts within one Agent instance; what is not in it (an agent
//! of an earlier run of the Agent) is read from the kernel.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_contracts::{BlockingService, CancellationToken};
use hanafubuki_agent_foundation::panic::catch_panic_async;
use hanafubuki_agent_kernel::{
    AgentChildSessionBase, AgentChildSessionRequest, AgentError, AgentSessionId,
    AgentSessionOrigin, AgentSessionPersistence, AgentSessionRuntimeSnapshot, AgentSessionSummary,
    AgentTurnOutcome, AgentTurnPhase,
};
use parking_lot::Mutex;
use tokio::sync::Notify;

use super::{AgentSubagentState, AgentSubagentStatus};
use crate::session_driver::AgentSessionDriver;

/// How many agents of one parent run a turn at once.
pub(super) const MAXIMUM_RUNNING_AGENTS: usize = 3;
pub(super) const DEFAULT_WAIT: Duration = Duration::from_secs(30);
pub(super) const MINIMUM_WAIT: Duration = Duration::from_secs(10);
pub(super) const MAXIMUM_WAIT: Duration = Duration::from_secs(300);
/// The longest last answer the facade's status carries.
const ANSWER_EXCERPT_CHARS: usize = 400;
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// An agent's status, as the model reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum AgentRunStatus {
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
    fn of_phase(phase: Option<AgentTurnPhase>) -> Self {
        match phase {
            None => Self::PendingInit,
            Some(phase) if !phase.is_terminal() => Self::Running,
            Some(AgentTurnPhase::Completed) => Self::Completed(None),
            Some(AgentTurnPhase::Failed) => Self::Errored("its last turn failed".to_string()),
            Some(_) => Self::Interrupted,
        }
    }

    fn of_turn(outcome: Result<Result<AgentTurnOutcome, AgentError>, String>) -> Self {
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

/// An agent of a parent, as `list_agents` and `wait_agent` report it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ListedAgent {
    pub(super) name: String,
    pub(super) id: AgentSessionId,
    pub(super) status: AgentRunStatus,
}

/// What a wait ended with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum WaitOutcome {
    /// These agents finished a turn since the parent's previous wait.
    Updates(Vec<ListedAgent>),
    /// No agent of the parent runs and none has an update.
    Idle,
    TimedOut,
}

/// How a message reached an agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Delivery {
    /// Added to the turn it runs.
    Steered,
    /// Kept for its next turn, which a follow-up task starts.
    Queued,
    /// It runs a new turn with it.
    Started,
}

/// What [`SubagentService::spawn`] creates.
pub(super) struct SpawnRequest {
    pub(super) task_name: String,
    pub(super) message: String,
    /// Whether the agent starts from the parent's history.
    pub(super) fork: bool,
}

/// The agents this Agent instance runs, kept across the plugin's restarts.
#[derive(Clone, Default)]
pub(crate) struct SubagentRuns {
    runs: Arc<Mutex<HashMap<AgentSessionId, AgentRun>>>,
    changed: Arc<Notify>,
}

struct AgentRun {
    parent: AgentSessionId,
    name: String,
    status: AgentRunStatus,
    /// Messages for its next turn.
    queued: Vec<String>,
    /// A final status the parent's waits have not reported yet.
    unreported: bool,
    /// How many turns this instance started on it.
    turn: u64,
}

impl SubagentRuns {
    fn running(&self, parent: AgentSessionId) -> usize {
        self.runs
            .lock()
            .values()
            .filter(|run| run.parent == parent && run.status == AgentRunStatus::Running)
            .count()
    }

    fn status(&self, child: AgentSessionId) -> Option<AgentRunStatus> {
        self.runs.lock().get(&child).map(|run| run.status.clone())
    }

    /// Records how `child`'s `turn`-th run ended, unless a later run
    /// started since.
    fn finish(&self, child: AgentSessionId, turn: u64, status: AgentRunStatus) {
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
    fn take_updates(&self, parent: AgentSessionId) -> (Vec<ListedAgent>, usize) {
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
                    status: run.status.clone(),
                });
            }
        }
        updates.sort_by_key(|update| update.id);
        (updates, running)
    }
}

/// What the `subagents` row provides under its name, and what its tools
/// run.
pub(crate) struct SubagentService {
    pub(super) driver: AgentSessionDriver,
    pub(super) runs: SubagentRuns,
    pub(super) blocking: Arc<BlockingService>,
}

impl SubagentService {
    /// Creates `request.task_name` under `parent` and runs its first turn,
    /// the message, in the background.
    pub(super) async fn spawn(
        &self,
        parent: AgentSessionId,
        request: SpawnRequest,
    ) -> Result<AgentSessionId, AgentError> {
        if self.session(parent).await?.parent_session_id.is_some() {
            return Err(refused(
                "a sub-agent cannot spawn agents of its own; only the main conversation spawns agents",
            ));
        }
        if self
            .agents(parent)
            .await?
            .iter()
            .any(|agent| agent.name == request.task_name)
        {
            return Err(refused(format!(
                "an agent named `{}` already exists; send it a follow-up task or choose another task_name",
                request.task_name
            )));
        }
        self.ensure_capacity(parent)?;
        let base = if request.fork {
            AgentChildSessionBase::ParentHead
        } else {
            AgentChildSessionBase::Empty
        };
        let name = request.task_name.clone();
        let child = self
            .blocking(move |driver| {
                let child = driver.create_child(AgentChildSessionRequest {
                    parent_session_id: parent,
                    origin: AgentSessionOrigin::Subagent,
                    persistence: AgentSessionPersistence::Durable,
                    base,
                    context: Vec::new(),
                })?;
                driver.rename(child.id, name)?;
                Ok(child.id)
            })
            .await?;
        self.start(parent, child, request.task_name, request.message)
            .await?;
        Ok(child)
    }

    /// Delivers `message` to `target` without starting a turn: it steers
    /// the turn the agent runs, or waits for its next one.
    pub(super) async fn send_message(
        &self,
        parent: AgentSessionId,
        target: &str,
        message: String,
    ) -> Result<Delivery, AgentError> {
        let agent = self.resolve(parent, target).await?;
        let Some(message) = self.steer(agent.id, message).await? else {
            return Ok(Delivery::Steered);
        };
        let mut runs = self.runs.runs.lock();
        runs.entry(agent.id)
            .or_insert_with(|| AgentRun::new(parent, agent.name, agent.status))
            .queued
            .push(message);
        Ok(Delivery::Queued)
    }

    /// Gives `target` a follow-up task: it steers the turn the agent runs,
    /// or starts a turn with it (after any queued messages).
    pub(super) async fn followup_task(
        &self,
        parent: AgentSessionId,
        target: &str,
        message: String,
    ) -> Result<Delivery, AgentError> {
        let agent = self.resolve(parent, target).await?;
        let Some(message) = self.steer(agent.id, message).await? else {
            return Ok(Delivery::Steered);
        };
        self.start(parent, agent.id, agent.name, message).await?;
        Ok(Delivery::Started)
    }

    /// Waits until an agent of `parent` finishes a turn, none runs, the
    /// timeout passes or `cancellation` is cancelled.
    pub(super) async fn wait(
        &self,
        parent: AgentSessionId,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<WaitOutcome, AgentError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            // Registered before the runs are read, so a finish in between
            // still wakes this wait.
            let changed = self.runs.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let (updates, running) = self.runs.take_updates(parent);
            if !updates.is_empty() {
                return Ok(WaitOutcome::Updates(updates));
            }
            if running == 0 {
                return Ok(WaitOutcome::Idle);
            }
            tokio::select! {
                _ = &mut changed => {}
                _ = tokio::time::sleep_until(deadline) => return Ok(WaitOutcome::TimedOut),
                _ = wait_cancelled(cancellation) => return Err(AgentError::Cancelled),
            }
        }
    }

    /// `parent`'s agents, oldest first.
    pub(super) async fn agents(
        &self,
        parent: AgentSessionId,
    ) -> Result<Vec<ListedAgent>, AgentError> {
        let children = self
            .blocking(move |driver| driver.children(parent, AgentSessionOrigin::Subagent))
            .await?;
        Ok(children
            .into_iter()
            .map(|child| self.listed(child))
            .collect())
    }

    /// Interrupts the turn `target` runs, if any; returns its status from
    /// before.
    pub(super) async fn interrupt(
        &self,
        parent: AgentSessionId,
        target: &str,
    ) -> Result<AgentRunStatus, AgentError> {
        let agent = self.resolve(parent, target).await?;
        self.stop_turn(agent.id).await?;
        Ok(agent.status)
    }

    /// Interrupts `child`'s turn; one that ended otherwise meanwhile is no
    /// error.
    async fn stop_turn(&self, child: AgentSessionId) -> Result<(), AgentError> {
        match self.driver.interrupt(child).await {
            Err(AgentError::TurnNotRunning) => Ok(()),
            result => result,
        }
    }

    /// The facade's view of `parent`'s agents, with each one's last answer
    /// cut to an excerpt.
    pub(crate) async fn statuses(
        &self,
        parent: AgentSessionId,
    ) -> Result<Vec<AgentSubagentStatus>, AgentError> {
        let mut statuses = Vec::new();
        for agent in self.agents(parent).await? {
            let last_answer = match &agent.status {
                AgentRunStatus::Completed(Some(answer)) => Some(answer.clone()),
                _ => {
                    let id = agent.id;
                    self.blocking(move |driver| driver.last_answer(id)).await?
                }
            };
            statuses.push(AgentSubagentStatus {
                session_id: agent.id,
                title: agent.name,
                status: match agent.status {
                    AgentRunStatus::Running => AgentSubagentState::Running,
                    AgentRunStatus::Errored(_) => AgentSubagentState::Failed,
                    _ => AgentSubagentState::Idle,
                },
                last_answer: last_answer.map(|answer| excerpt(&answer)),
            });
        }
        Ok(statuses)
    }

    /// Interrupts the turn the subagent `session_id` runs, if any.
    pub(crate) async fn interrupt_subagent(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentError> {
        if self.session(session_id).await?.origin != Some(AgentSessionOrigin::Subagent) {
            return Err(AgentError::InvalidInput(format!(
                "Agent session {session_id} is not a subagent"
            )));
        }
        self.stop_turn(session_id).await
    }

    /// The session's current state.
    pub(super) async fn session(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentSessionRuntimeSnapshot, AgentError> {
        self.blocking(move |driver| driver.snapshot(session_id))
            .await
    }

    fn ensure_capacity(&self, parent: AgentSessionId) -> Result<(), AgentError> {
        if self.runs.running(parent) >= MAXIMUM_RUNNING_AGENTS {
            return Err(capacity_refusal());
        }
        Ok(())
    }

    /// Runs a turn of `message` on `child` in the background, after the
    /// messages queued for it, with `parent`'s permission mode.
    async fn start(
        &self,
        parent: AgentSessionId,
        child: AgentSessionId,
        name: String,
        message: String,
    ) -> Result<(), AgentError> {
        self.driver.inherit_permission_mode(parent, child).await?;
        let (text, turn) = {
            let mut runs = self.runs.runs.lock();
            let running = runs
                .values()
                .filter(|run| run.parent == parent && run.status == AgentRunStatus::Running)
                .count();
            if running >= MAXIMUM_RUNNING_AGENTS {
                return Err(capacity_refusal());
            }
            let run = runs
                .entry(child)
                .or_insert_with(|| AgentRun::new(parent, name, AgentRunStatus::PendingInit));
            let mut parts = std::mem::take(&mut run.queued);
            parts.push(message);
            run.turn += 1;
            run.status = AgentRunStatus::Running;
            run.unreported = false;
            (parts.join("\n\n"), run.turn)
        };
        let runs = self.runs.clone();
        let driver = self.driver.clone();
        tokio::spawn(async move {
            let outcome = catch_panic_async(driver.run_turn(child, text.into())).await;
            runs.finish(child, turn, AgentRunStatus::of_turn(outcome));
        });
        Ok(())
    }

    /// Steers the turn `child` runs with `message`; hands the message back
    /// when it runs none.
    async fn steer(
        &self,
        child: AgentSessionId,
        message: String,
    ) -> Result<Option<String>, AgentError> {
        self.blocking(move |driver| match driver.steer(child, message.clone()) {
            Ok(()) => Ok(None),
            Err(AgentError::TurnNotRunning) => Ok(Some(message)),
            Err(error) => Err(error),
        })
        .await
    }

    /// The agent of `parent` that `target` names, by task name or id.
    async fn resolve(
        &self,
        parent: AgentSessionId,
        target: &str,
    ) -> Result<ListedAgent, AgentError> {
        self.agents(parent)
            .await?
            .into_iter()
            .find(|agent| agent.name == target || agent.id.to_string() == target)
            .ok_or_else(|| {
                refused(format!(
                    "no agent named `{target}`; list_agents lists the agents you spawned"
                ))
            })
    }

    fn listed(&self, child: AgentSessionSummary) -> ListedAgent {
        ListedAgent {
            name: child
                .title
                .map(|title| title.value)
                .unwrap_or_else(|| child.session_id.to_string()),
            id: child.session_id,
            status: self
                .runs
                .status(child.session_id)
                .unwrap_or_else(|| AgentRunStatus::of_phase(child.latest_turn_phase)),
        }
    }

    async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce(&AgentSessionDriver) -> Result<T, AgentError> + Send + 'static,
    ) -> Result<T, AgentError> {
        let driver = self.driver.clone();
        self.blocking
            .run("agent subagent operation", move || work(&driver))
            .await
            .map_err(|error| AgentError::Internal(error.to_string()))?
    }
}

impl AgentRun {
    fn new(parent: AgentSessionId, name: String, status: AgentRunStatus) -> Self {
        Self {
            parent,
            name,
            status,
            queued: Vec::new(),
            unreported: false,
            turn: 0,
        }
    }
}

fn capacity_refusal() -> AgentError {
    refused(format!(
        "at most {MAXIMUM_RUNNING_AGENTS} agents can run at once; wait for one to finish or interrupt one"
    ))
}

/// A refusal the model reads as the tool's error.
fn refused(message: impl Into<String>) -> AgentError {
    AgentError::InvalidInput(message.into())
}

fn excerpt(answer: &str) -> String {
    match answer.char_indices().nth(ANSWER_EXCERPT_CHARS) {
        Some((end, _)) => format!("{}…", &answer[..end]),
        None => answer.to_string(),
    }
}

async fn wait_cancelled(cancellation: &CancellationToken) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(CANCELLATION_POLL_INTERVAL).await;
    }
}
