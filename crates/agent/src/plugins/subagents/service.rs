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
// for its next follow-up task; by default at most 3 agents of one parent
// run at once (Codex's 4 threads per session less the root, counted by
// running turns since nothing is evicted), only a session without a parent
// spawns (Codex's `agents.max_depth` default), and a wait lasts at most 5
// minutes (the turn's own tool timeout and deadline bound it); the limits
// and the wait bounds are the plugin's options; an agent's role
// (`agent_type`) is one of the Agent's roles, which only narrow it; its
// Provider, model and reasoning effort are those `spawn_agent` passes, else
// its role's, else its parent's, checked before the agent exists and set
// in one commit.

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
    AgentTurnOutcome, AgentTurnPhase, ModelReasoningEffort,
};
use parking_lot::Mutex;
use tokio::sync::Notify;

use super::models::refusal;
use super::options::SubagentLimits;
use super::{AgentSubagentState, AgentSubagentStatus};
use crate::roles::{AgentRoleCatalogReader, AgentSessionRole, DEFAULT_ROLE};
use crate::session_driver::{AgentSessionDriver, ChildModelRequest, SpawnModelCatalog};

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
    /// The role its session has, `default` when it narrows nothing.
    pub(super) role: String,
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
    /// What the agent takes of the role it is spawned as.
    pub(super) role: AgentSessionRole,
    /// The Provider (an id or a display name), model and reasoning effort
    /// passed for it, over the role's and the parent's.
    pub(super) provider: Option<String>,
    pub(super) model: Option<String>,
    pub(super) reasoning_effort: Option<ModelReasoningEffort>,
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
    role: String,
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
                    role: run.role.clone(),
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
    /// The roles an agent is spawned as, read at each call.
    pub(super) roles: AgentRoleCatalogReader,
    /// The plugin's options; the running-agent limit changes in place.
    pub(super) limits: Mutex<SubagentLimits>,
}

impl SubagentService {
    pub(super) fn limits(&self) -> SubagentLimits {
        *self.limits.lock()
    }

    /// Creates `request.task_name` under `parent` as `request.role` (its
    /// scope the parent's narrowed by the role's, the role's instructions
    /// after the base ones) on the Provider, model and reasoning effort
    /// the request passes, else the role's, else the parent's, and runs its
    /// first turn, the message, in the background. Every refusal comes
    /// before the agent exists. Returns the agent as it starts.
    pub(super) async fn spawn(
        &self,
        parent: AgentSessionId,
        request: SpawnRequest,
    ) -> Result<AgentSessionRuntimeSnapshot, AgentError> {
        let parent_session = self.session(parent).await?;
        if parent_session.parent_session_id.is_some() {
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
        let role = request.role;
        let model = self
            .driver
            .resolve_child_model(
                &parent_session,
                ChildModelRequest {
                    provider: request.provider,
                    model: request.model,
                    reasoning_effort: request.reasoning_effort,
                    role_model: role.model,
                    role_reasoning_effort: role.reasoning_effort,
                },
            )
            .await
            .map_err(refusal)?;
        let base = if request.fork {
            AgentChildSessionBase::ParentHead
        } else {
            AgentChildSessionBase::Empty
        };
        let name = request.task_name.clone();
        let (child, child_role) = self
            .blocking(move |driver| {
                let child = driver.create_child(AgentChildSessionRequest {
                    parent_session_id: parent,
                    origin: AgentSessionOrigin::Subagent,
                    persistence: AgentSessionPersistence::Durable,
                    base,
                    context: Vec::new(),
                    scope: role.scope,
                    role_instructions: role.instructions,
                })?;
                driver.rename(child.id, name)?;
                Ok((child.id, role_name(child.role)))
            })
            .await?;
        let snapshot = self
            .driver
            .apply_child_model(child, model)
            .await
            .map_err(AgentError::InvalidInput)?;
        let agent = ListedAgent {
            name: request.task_name,
            id: child,
            role: child_role,
            status: AgentRunStatus::PendingInit,
        };
        self.start(parent, agent, request.message).await?;
        Ok(snapshot)
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
            .or_insert_with(|| AgentRun::new(parent, agent))
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
        self.start(parent, agent, message).await?;
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

    /// What the session may spawn agents on, when it is not an agent
    /// itself.
    pub(super) async fn spawn_models(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Option<SpawnModelCatalog>, AgentError> {
        self.blocking(move |driver| {
            let session = driver.snapshot(session_id)?;
            if session.parent_session_id.is_some() {
                return Ok(None);
            }
            driver
                .spawn_model_catalog(&session)
                .map(Some)
                .map_err(AgentError::Persistence)
        })
        .await
    }

    fn ensure_capacity(&self, parent: AgentSessionId) -> Result<(), AgentError> {
        let maximum = self.limits().maximum_running_agents;
        if self.runs.running(parent) >= maximum {
            return Err(capacity_refusal(maximum));
        }
        Ok(())
    }

    /// Runs a turn of `message` on `agent` in the background, after the
    /// messages queued for it, with `parent`'s permission mode.
    async fn start(
        &self,
        parent: AgentSessionId,
        agent: ListedAgent,
        message: String,
    ) -> Result<(), AgentError> {
        let child = agent.id;
        self.driver.inherit_permission_mode(parent, child).await?;
        let maximum = self.limits().maximum_running_agents;
        let (text, turn) = {
            let mut runs = self.runs.runs.lock();
            let running = runs
                .values()
                .filter(|run| run.parent == parent && run.status == AgentRunStatus::Running)
                .count();
            if running >= maximum {
                return Err(capacity_refusal(maximum));
            }
            let run = runs
                .entry(child)
                .or_insert_with(|| AgentRun::new(parent, agent));
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
            role: role_name(child.role),
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
    fn new(parent: AgentSessionId, agent: ListedAgent) -> Self {
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

/// A session's role as an agent's listing names it.
fn role_name(role: Option<String>) -> String {
    role.unwrap_or_else(|| DEFAULT_ROLE.to_string())
}

fn capacity_refusal(maximum: usize) -> AgentError {
    refused(format!(
        "at most {maximum} agents can run at once; wait for one to finish or interrupt one"
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
