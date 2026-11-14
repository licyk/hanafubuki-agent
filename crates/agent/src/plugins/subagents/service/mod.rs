// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core/src/tools/handlers/multi_agents_v2/` (what spawning,
// messaging, waiting for, listing and interrupting an agent do),
// `codex-rs/core/src/agent/status.rs` (an agent's status) and
// `codex-rs/core/src/config/mod.rs` (the limits and the wait bounds) at
// commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: an agent is a durable child session of
// origin `Subagent`, named by its title; in place of a mailbox, a finished
// turn is queued on the parent's durable wake queue, which the parent's
// next `wait_agent` takes (returning the final answer) or, with its wake
// policy on, its next step or its woken turn delivers; an agent messages
// its parent (`target` `parent`) through the same queue and never another
// agent; a message to a running agent steers its turn and one to an idle
// agent waits for its next follow-up task; by default at most 3 agents of one parent
// run at once (Codex's 4 threads per session less the root, counted by
// running turns since nothing is evicted), only a session without a parent
// spawns (Codex's `agents.max_depth` default), and a wait lasts at most 5
// minutes (the turn's own tool timeout and deadline bound it); the limits
// and the wait bounds are the plugin's options; an agent's role
// (`agent_type`) is one of the Agent's roles, which only narrow it; its
// Provider, model and reasoning effort are those `spawn_agent` passes, else
// its role's, else its parent's, checked before the agent exists and set
// in one commit.

//! What the subagent tools do, over the session driver. The runs live in
//! [`SubagentRuns`] (`runs.rs`), which outlives the plugin's restarts within
//! one Agent instance; what is not in it (an agent of an earlier run of the
//! Agent) is read from the kernel. What the facade asks is `statuses.rs`'s.

mod runs;
mod statuses;

use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_contracts::{BlockingService, CancellationToken};
use hanafubuki_agent_foundation::panic::catch_panic_async;
use hanafubuki_agent_kernel::{
    AgentChildSessionBase, AgentChildSessionRequest, AgentError, AgentSessionId,
    AgentSessionOrigin, AgentSessionPersistence, AgentSessionRuntimeSnapshot, AgentSubagentWake,
    AgentWakeFilter, AgentWakeQueued, AgentWakeSource, ModelReasoningEffort,
};
use parking_lot::Mutex;

use self::runs::AgentRun;
pub(crate) use self::runs::SubagentRuns;
pub(super) use self::runs::{AgentRunStatus, AgentUpdate};
use super::models::refusal;
use super::options::SubagentLimits;
use crate::roles::{AgentRoleCatalogReader, AgentSessionRole, DEFAULT_ROLE};
use crate::session_driver::{AgentSessionDriver, ChildModelRequest, SpawnModelCatalog};
use crate::wake::{root_of, Wakes};

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// The `target` a sub-agent names the agent that spawned it by.
pub(super) const PARENT_TARGET: &str = "parent";

/// An agent of a parent, as `list_agents` and `wait_agent` report it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ListedAgent {
    pub(super) name: String,
    pub(super) id: AgentSessionId,
    /// The role its session has, `default` when it narrows nothing.
    pub(super) role: String,
    pub(super) status: AgentRunStatus,
    /// When this instance set `status`, in Unix milliseconds.
    pub(super) since: Option<u64>,
}

/// The Provider, model and reasoning effort an agent's session runs on now,
/// read from its summary (a `/model` on it shows), never kept from its
/// spawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ChildModelView {
    pub(super) provider_id: String,
    pub(super) model: String,
    /// None: the Provider's default.
    pub(super) reasoning_effort: Option<ModelReasoningEffort>,
}

/// What a wait ended with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum WaitOutcome {
    /// The final statuses queued on the parent since its previous wait (or
    /// its turn's step that delivered them), by agent id.
    Updates(Vec<AgentUpdate>),
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

/// What the `subagents` row provides under its name, and what its tools
/// run.
pub(crate) struct SubagentService {
    pub(super) driver: AgentSessionDriver,
    pub(super) runs: SubagentRuns,
    /// Where its agents' turns tell the host what they ask of a person.
    pub(super) wakes: Wakes,
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
            since: None,
        };
        self.start(parent, agent, request.message).await?;
        Ok(snapshot)
    }

    /// Delivers `message` to `target` without starting a turn: it steers
    /// the turn the agent runs, or waits for its next one. A sub-agent's
    /// `parent` is the agent that spawned it ([`Self::message_parent`]).
    pub(super) async fn send_message(
        &self,
        parent: AgentSessionId,
        target: &str,
        message: String,
    ) -> Result<Delivery, AgentError> {
        if target == PARENT_TARGET {
            let session = self.session(parent).await?;
            if let (Some(spawner), Some(AgentSessionOrigin::Subagent)) =
                (session.parent_session_id, session.origin)
            {
                let name = session
                    .title
                    .map_or_else(|| parent.to_string(), |title| title.value);
                return self.message_parent(spawner, parent, name, message).await;
            }
        }
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

    /// Queues `message` from the sub-agent `child`, named `name`, on its
    /// parent's wake queue: the parent's running turn reads it at its next
    /// step, and an idle parent is woken to read it (D7).
    async fn message_parent(
        &self,
        parent: AgentSessionId,
        child: AgentSessionId,
        name: String,
        message: String,
    ) -> Result<Delivery, AgentError> {
        let source = AgentWakeSource::Subagent {
            session_id: child,
            name,
            event: AgentSubagentWake::Message { text: message },
        };
        match self.driver.enqueue_wake_event(parent, source).await {
            Ok(AgentWakeQueued::Delivering) => Ok(Delivery::Steered),
            Ok(AgentWakeQueued::Waiting) => Ok(Delivery::Queued),
            Ok(AgentWakeQueued::Disabled) => Err(refused(
                "your parent takes no messages: wakeOnBackgroundTasks is off; put what it must know in your final answer",
            )),
            Err(AgentError::LimitExceeded { limit, .. }) => Err(refused(format!(
                "your parent has {limit} unread messages; wait until it reads them"
            ))),
            Err(error) => Err(error),
        }
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

    /// Waits until a final status of an agent of `parent` is queued on it,
    /// none runs, the timeout passes or `cancellation` is cancelled; takes
    /// what is queued, so no step or woken turn delivers it again.
    pub(super) async fn wait(
        &self,
        parent: AgentSessionId,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<WaitOutcome, AgentError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            // Registered before anything is read, so a finish in between
            // still wakes this wait; the runs are read before the queue,
            // since a run queues its status before it stops running.
            let changed = self.runs.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let running = self.runs.running(parent);
            let mut updates = self
                .driver
                .take_wake_events(parent, AgentWakeFilter::SubagentStatuses)
                .await?
                .into_iter()
                .filter_map(AgentUpdate::of_event)
                .collect::<Vec<_>>();
            if !updates.is_empty() {
                updates.sort_by_key(|update| update.id);
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
        Ok(self
            .agents_on_models(parent)
            .await?
            .into_iter()
            .map(|(agent, _)| agent)
            .collect())
    }

    /// `parent`'s agents, oldest first, each with the model it runs on.
    pub(super) async fn agents_on_models(
        &self,
        parent: AgentSessionId,
    ) -> Result<Vec<(ListedAgent, ChildModelView)>, AgentError> {
        let runs = self.runs.clone();
        self.blocking(move |driver| runs.agents_on_models(driver, parent))
            .await
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
        let root = self.blocking(move |driver| root_of(driver, parent)).await?;
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
            run.set(AgentRunStatus::Running);
            (parts.join("\n\n"), run.turn)
        };
        self.runs.feed.hint(parent);
        // What its previous turn left queued on the parent is no news once
        // the next one runs: the parent hears of that one.
        let withdrawn = self
            .driver
            .take_wake_events(
                parent,
                AgentWakeFilter::SubagentStatus { session_id: child },
            )
            .await;
        if let Err(error) = withdrawn {
            log::debug!("the last status of subagent {child} was not withdrawn: {error}");
        }
        let runs = self.runs.clone();
        let driver = self.driver.clone();
        let events = self.wakes.child_events(root);
        tokio::spawn(async move {
            let outcome = catch_panic_async(driver.run_turn(child, text.into(), events)).await;
            let status = AgentRunStatus::of_turn(outcome);
            runs.finish(&driver, child, turn, status).await;
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

async fn wait_cancelled(cancellation: &CancellationToken) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(CANCELLATION_POLL_INTERVAL).await;
    }
}
