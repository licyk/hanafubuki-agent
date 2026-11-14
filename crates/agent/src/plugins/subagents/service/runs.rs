// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core/src/agent/status.rs` (an agent's status) at commit
// `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: an agent's statuses are those of its
// latest turn, which this Agent instance ran or the kernel recorded; a
// final status is what its parent's next wait reports.

//! The agents this Agent instance runs ([`SubagentRuns`]), each with the
//! status the model reads, and how a run's end reaches its parent: a final
//! status is queued on the parent's wake queue, which `wait_agent` takes
//! (and, with the parent's wake policy on, its next step or its woken turn
//! delivers).

use std::collections::HashMap;
use std::sync::Arc;

use hanafubuki_agent_kernel::{
    AgentError, AgentSessionId, AgentSessionOrigin, AgentSubagentWake, AgentTurnOutcome,
    AgentTurnPhase, AgentWakeEvent, AgentWakeSource,
};
use parking_lot::Mutex;
use tokio::sync::Notify;

use super::{role_name, ChildModelView, ListedAgent};
use crate::session_driver::AgentSessionDriver;
use crate::wake::Feed;

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

    /// What the parent hears of a run that ended so: Codex's final
    /// statuses. An interrupt, the parent's own or the person's, is no news
    /// for it.
    fn wake(&self) -> Option<AgentSubagentWake> {
        match self {
            Self::Completed(answer) => Some(AgentSubagentWake::Completed {
                answer: answer.clone(),
                truncated: false,
            }),
            Self::Errored(error) => Some(AgentSubagentWake::Errored {
                error: error.clone(),
            }),
            _ => None,
        }
    }
}

/// A final status taken from the parent's wake queue, as `wait_agent`
/// reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::plugins::subagents) struct AgentUpdate {
    pub(in crate::plugins::subagents) name: String,
    pub(in crate::plugins::subagents) id: AgentSessionId,
    pub(in crate::plugins::subagents) status: AgentRunStatus,
}

impl AgentUpdate {
    /// The update a queued sub-agent's status gives; none for anything else.
    pub(super) fn of_event(event: AgentWakeEvent) -> Option<Self> {
        let AgentWakeSource::Subagent {
            session_id,
            name,
            event,
        } = event.source
        else {
            return None;
        };
        let status = match event {
            AgentSubagentWake::Completed { answer, truncated } => {
                AgentRunStatus::Completed(answer.map(|answer| match truncated {
                    true => format!("{answer}… (cut; the rest is in the sub-agent's conversation)"),
                    false => answer,
                }))
            }
            AgentSubagentWake::Errored { error } => AgentRunStatus::Errored(error),
            AgentSubagentWake::ClosedByUser => {
                AgentRunStatus::Errored("it was closed by the user".to_string())
            }
            AgentSubagentWake::Lost => {
                AgentRunStatus::Errored("it stopped when the Agent restarted".to_string())
            }
            AgentSubagentWake::Message { .. } => return None,
        };
        Some(Self {
            name,
            id: session_id,
            status,
        })
    }
}

/// The agents this Agent instance runs, kept across the plugin's restarts.
#[derive(Clone)]
pub(crate) struct SubagentRuns {
    pub(super) runs: Arc<Mutex<HashMap<AgentSessionId, AgentRun>>>,
    /// Told when a run ends, once what its parent hears of it is queued.
    pub(super) changed: Arc<Notify>,
    /// Told of the parent whose agent started or ended, once its status is
    /// set.
    pub(super) feed: Arc<Feed>,
}

pub(super) struct AgentRun {
    pub(super) parent: AgentSessionId,
    name: String,
    pub(super) status: AgentRunStatus,
    /// When `status` was set, in Unix milliseconds; `None` for the status
    /// of an agent this instance has not run yet.
    pub(super) since: Option<u64>,
    /// Messages for its next turn.
    pub(super) queued: Vec<String>,
    /// How many turns this instance started on it.
    pub(super) turn: u64,
}

impl SubagentRuns {
    pub(crate) fn new(feed: Arc<Feed>) -> Self {
        Self {
            runs: Arc::default(),
            changed: Arc::default(),
            feed,
        }
    }

    /// How many agents of `parent` run: one counts from before its spawn
    /// returns until its end is queued on `parent`, so a parent that waits
    /// on it never sees it neither running nor reported.
    pub(crate) fn running(&self, parent: AgentSessionId) -> usize {
        self.runs
            .lock()
            .values()
            .filter(|run| run.parent == parent && run.status == AgentRunStatus::Running)
            .count()
    }

    /// Records how `child`'s `turn`-th run ended, unless a later run
    /// started since. A final status is queued on its parent before the
    /// run stops counting as running, so a wait that finds no agent running
    /// finds the status queued.
    pub(super) async fn finish(
        &self,
        driver: &AgentSessionDriver,
        child: AgentSessionId,
        turn: u64,
        status: AgentRunStatus,
    ) {
        let parent = self
            .runs
            .lock()
            .get(&child)
            .filter(|run| run.turn == turn)
            .map(|run| (run.parent, run.name.clone()));
        if let (Some((parent, name)), Some(event)) = (parent, status.wake()) {
            let source = AgentWakeSource::Subagent {
                session_id: child,
                name,
                event,
            };
            if let Err(error) = driver.enqueue_wake_event(parent, source).await {
                // An archived or deleted parent hears of nothing.
                log::debug!("the end of subagent {child} was not queued on {parent}: {error}");
            }
        }
        let parent = {
            let mut runs = self.runs.lock();
            let run = runs.get_mut(&child).filter(|run| run.turn == turn);
            run.map(|run| {
                run.set(status);
                run.parent
            })
        };
        self.changed.notify_waiters();
        if let Some(parent) = parent {
            self.feed.hint(parent);
        }
    }

    /// `parent`'s agents, oldest first, each with the model it runs on, the
    /// runs read at once. Blocks.
    pub(super) fn agents_on_models(
        &self,
        driver: &AgentSessionDriver,
        parent: AgentSessionId,
    ) -> Result<Vec<(ListedAgent, ChildModelView)>, AgentError> {
        let children = driver.children(parent, AgentSessionOrigin::Subagent)?;
        let runs = self.runs.lock();
        Ok(children
            .into_iter()
            .map(|child| {
                let model = ChildModelView {
                    provider_id: child.provider_id.clone(),
                    model: child.model.clone(),
                    reasoning_effort: child.reasoning_effort.clone(),
                };
                let run = runs.get(&child.session_id);
                let agent = ListedAgent {
                    name: child
                        .title
                        .map(|title| title.value)
                        .unwrap_or_else(|| child.session_id.to_string()),
                    id: child.session_id,
                    role: role_name(child.role),
                    status: run.map_or_else(
                        || AgentRunStatus::of_phase(child.latest_turn_phase),
                        |run| run.status.clone(),
                    ),
                    since: run.and_then(|run| run.since),
                };
                (agent, model)
            })
            .collect())
    }
}

impl AgentRun {
    pub(super) fn new(parent: AgentSessionId, agent: ListedAgent) -> Self {
        Self {
            parent,
            name: agent.name,
            status: agent.status,
            since: agent.since,
            queued: Vec::new(),
            turn: 0,
        }
    }

    pub(super) fn set(&mut self, status: AgentRunStatus) {
        let since = now_millis();
        self.status = status;
        self.since = Some(since);
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}
