//! The session's subagents onto the facade: `/subagents`' list, the one
//! Alt+Left and Alt+Right step through, the interrupt `/agents` asks for,
//! and the list kept current while it is open.
//!
//! A subagent's turns are run by the `subagents` plugin, not by the
//! interface. When the Agent says the shown session's tree changed
//! (`AgentBackgroundEvent::Changed`: an agent started, ended or changed its
//! model, an update was queued or read), the bridge reads the tree once
//! (`background_tasks`): the statuses go to the open `/subagents` list, and
//! the session's status (what it waits on) is sent again. Nothing is read
//! while nothing changes. What a subagent asks of a person is `children`'s.
//!
//! A subagent shown in the second conversation is followed from its
//! journal, read every [`FOLLOW_INTERVAL`] from where its transcript ended:
//! its events are handed to that conversation's bridge as if its sink had
//! sent them. The Agent tells the host none of a subagent's streamed output,
//! so a shown one is the only thing still read on a timer.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent::{
    AgentError, AgentEventKind, AgentEventSequence, AgentHandle, AgentSessionId,
    AgentSubagentState, AgentSubagentStatus, AgentTurnAccepted, AgentTurnId, ModelReasoningEffort,
};

use super::{AgentBridge, AgentReply, TurnUpdate};
use crate::app::WorkspaceEvent;
use crate::app_event::{AppEvent, AppEventSender};

/// How often a subagent shown in the second conversation is read.
const FOLLOW_INTERVAL: Duration = Duration::from_millis(150);

/// What `/subagents` asks of the Agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SubagentsCall {
    /// The session's subagents.
    List,
}

/// What the Agent answered about the session's subagents.
#[derive(Debug)]
pub(crate) enum SubagentsReply {
    /// `/subagents`: the session's subagents, oldest first.
    Listed(Vec<NavigationAgent>),
    /// The Agent said they changed, and they were read again.
    Changed(Vec<NavigationAgent>),
    Failed(String),
}

/// The following of a shown subagent's journal, which ends when dropped.
pub(super) struct Following(Arc<AtomicBool>);

impl Drop for Following {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

impl AgentBridge {
    pub(super) fn subagents_call(&mut self, call: SubagentsCall) -> Option<TurnUpdate> {
        match call {
            SubagentsCall::List => {
                let Some(parent) = self.session_id else {
                    return Some(reply(SubagentsReply::Listed(Vec::new())));
                };
                self.spawn(move |agent| async move {
                    let listed = match navigation_agents(&agent, parent).await {
                        Ok(statuses) => SubagentsReply::Listed(statuses),
                        Err(error) => SubagentsReply::Failed(subagents_error(error)),
                    };
                    Ok(Some(AgentReply::Update(Box::new(reply(listed)))))
                });
            }
        }
        None
    }

    /// The subagents of `parent`, read off the interface's thread, for
    /// Alt+Left or Alt+Right (`forward`) to step through.
    pub(crate) fn list_subagents_to_step(&self, parent: AgentSessionId, forward: bool) {
        let agent = self.agent.clone();
        let app_event_tx = self.app_event_tx.clone();
        self.agent.runtime().handle().spawn(async move {
            let listed = navigation_agents(&agent, parent)
                .await
                .map(|statuses| {
                    statuses
                        .into_iter()
                        .filter(|status| status.session_id != parent)
                        .map(|status| status.session_id)
                        .collect()
                })
                .map_err(subagents_error);
            app_event_tx.send(AppEvent::Workspace(WorkspaceEvent::StepAgentListed {
                parent,
                forward,
                listed,
            }));
        });
    }

    /// The shown session's tree changed: what it waits on is sent again,
    /// and its subagents are read again for the open list.
    pub(crate) fn tree_changed(&mut self, root: AgentSessionId) {
        let Some(session) = self.session_id else {
            return;
        };
        if navigation_root(&self.agent, session).ok() != Some(root) {
            return;
        }
        self.send_session_status();
        self.spawn(move |agent| async move {
            let read = navigation_agents(&agent, root)
                .await
                .map_err(|error| format!("the subagents were not read: {error}"))?;
            let changed = reply(SubagentsReply::Changed(read));
            Ok(Some(AgentReply::Update(Box::new(changed))))
        });
    }

    /// Follows `session_id`, a subagent this bridge shows, from after
    /// `after`: its events come in as if its sink had sent them, and a
    /// turn it runs is the bridge's to interrupt and answer.
    pub(crate) fn follow(
        &mut self,
        session_id: AgentSessionId,
        after: Option<AgentEventSequence>,
        running: Option<AgentTurnAccepted>,
    ) -> Option<TurnUpdate> {
        let stop = Arc::new(AtomicBool::new(false));
        self.following = Some(Following(stop.clone()));
        let agent = self.agent.clone();
        let app_event_tx = self.app_event_tx.clone();
        std::thread::spawn(move || run_follow(&agent, session_id, after, &stop, &app_event_tx));
        let turn = running?;
        self.turn = Some(turn);
        self.turn_running = true;
        Some(TurnUpdate::Started)
    }

    /// A followed session's turn the bridge did not start is its own once
    /// one of its events arrives.
    pub(super) fn adopt_followed_turn(&mut self, kind: &AgentEventKind, turn: Option<AgentTurnId>) {
        if self.following.is_none() || self.turn_running || turn.is_none() || kind.is_terminal() {
            return;
        }
        self.turn_running = true;
        if !matches!(kind, AgentEventKind::TurnStarted { .. }) {
            let started = AgentReply::Update(Box::new(TurnUpdate::Started));
            self.app_event_tx.send(AppEvent::AgentReply(started));
        }
    }
}

fn reply(reply: SubagentsReply) -> TurnUpdate {
    TurnUpdate::Subagents(reply)
}

fn subagents_error(error: AgentError) -> String {
    match error {
        AgentError::FeatureUnavailable { reason, .. } => {
            format!("subagents are unavailable: {reason}")
        }
        error => error.to_string(),
    }
}

fn run_follow(
    agent: &AgentHandle,
    session_id: AgentSessionId,
    mut after: Option<AgentEventSequence>,
    stop: &AtomicBool,
    app_event_tx: &AppEventSender,
) {
    while !stop.load(Ordering::Acquire) {
        let Ok(batch) = agent.events_since(session_id, after) else {
            return;
        };
        for event in batch.events {
            after = Some(event.sequence);
            app_event_tx.send(AppEvent::Agent(Box::new(event)));
        }
        std::thread::sleep(FOLLOW_INTERVAL);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NavigationAgent {
    pub(crate) session_id: AgentSessionId,
    pub(crate) title: String,
    pub(crate) status: AgentSubagentState,
    pub(crate) model: String,
    pub(crate) reasoning_effort: Option<ModelReasoningEffort>,
    pub(crate) last_answer: Option<String>,
}

impl From<AgentSubagentStatus> for NavigationAgent {
    fn from(agent: AgentSubagentStatus) -> Self {
        Self {
            session_id: agent.session_id,
            title: agent.title,
            status: agent.status,
            model: agent.model,
            reasoning_effort: agent.reasoning_effort,
            last_answer: agent.last_answer,
        }
    }
}

pub(super) fn navigation_root(
    agent: &AgentHandle,
    mut session: AgentSessionId,
) -> Result<AgentSessionId, AgentError> {
    while let Some(parent) = agent.snapshot(session)?.parent_session_id {
        session = parent;
    }
    Ok(session)
}

async fn navigation_agents(
    agent: &AgentHandle,
    session: AgentSessionId,
) -> Result<Vec<NavigationAgent>, AgentError> {
    let root = navigation_root(agent, session)?;
    let snapshot = agent.snapshot(root)?;
    let mut agents = vec![NavigationAgent {
        session_id: root,
        title: snapshot.title.map_or_else(
            || "Main agent".into(),
            |title| format!("Main agent · {}", title.value),
        ),
        status: match snapshot.turn.as_ref().map(|turn| turn.phase) {
            Some(hanafubuki_agent::AgentTurnPhase::WaitingForUserInput) => {
                AgentSubagentState::Waiting
            }
            Some(hanafubuki_agent::AgentTurnPhase::Failed) => AgentSubagentState::Failed,
            Some(phase) if !phase.is_terminal() => AgentSubagentState::Running,
            _ => AgentSubagentState::Idle,
        },
        model: snapshot.model,
        reasoning_effort: snapshot.reasoning_effort,
        last_answer: None,
    }];
    agents.extend(
        agent
            .background_tasks(root)
            .await?
            .agents
            .into_iter()
            .map(NavigationAgent::from),
    );
    Ok(agents)
}
