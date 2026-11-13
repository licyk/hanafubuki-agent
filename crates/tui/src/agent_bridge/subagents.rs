//! The session's subagents onto the facade: `/subagents`' list, the one
//! Alt+Left and Alt+Right step through, the interrupt `/agents` asks for,
//! and a watch over them while they run.
//!
//! A subagent's turns are run by the `subagents` plugin, not by the
//! interface, so their events reach no sink of the interface's: they land
//! in the subagent's own journal. While any subagent of the session runs
//! (or the session's turn does, which may spawn one), the watch reads
//! `subagents(parent)` and the journal of each running subagent every
//! [`WATCH_INTERVAL`] on a thread of its own: the statuses go to the open
//! `/subagents` list, and an approval a subagent waits on (in Manual mode)
//! is shown as the parent's, labelled with the subagent's title, and
//! answered on the subagent. The watch ends once nothing runs.
//!
//! A subagent shown in the second conversation is followed the same way:
//! its journal is read from where its transcript ended, and its events are
//! handed to that conversation's bridge as if its sink had sent them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hanafubuki_agent::{
    AgentError, AgentEventKind, AgentEventSequence, AgentGeneration, AgentHandle, AgentSessionId,
    AgentSubagentState, AgentSubagentStatus, AgentTurnAccepted, AgentTurnId, ApprovalRequest,
    ApprovalRequestId, ApprovalUserDecision,
};

use super::{AgentBridge, AgentReply, TurnUpdate};
use crate::app::WorkspaceEvent;
use crate::app_event::{AppEvent, AppEventSender};

/// How often the watch reads the subagents while they run.
const WATCH_INTERVAL: Duration = Duration::from_millis(400);
/// How often a subagent shown in the second conversation is read.
const FOLLOW_INTERVAL: Duration = Duration::from_millis(150);
/// The tools whose call may start a subagent's turn.
const STARTING_TOOLS: [&str; 3] = ["spawn_agent", "send_message", "followup_task"];

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
    Listed(Vec<AgentSubagentStatus>),
    /// The watch read them again and they changed.
    Changed(Vec<AgentSubagentStatus>),
    Failed(String),
}

/// An approval a subagent waits on, and where its answer goes.
#[derive(Debug, Clone)]
pub(crate) struct ChildApproval {
    pub(crate) parent: AgentSessionId,
    pub(crate) child: AgentSessionId,
    pub(crate) title: String,
    pub(crate) turn_id: AgentTurnId,
    pub(crate) generation: AgentGeneration,
    pub(crate) request: ApprovalRequest,
}

/// The watch's thread: `stop` ends it, `alive` says it has not ended.
pub(super) struct Watch {
    parent: AgentSessionId,
    stop: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// The following of a shown subagent's journal, which ends when dropped.
pub(super) struct Following(Arc<AtomicBool>);

impl Drop for Following {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// A subagent's approval shown in the parent, labelled, and where it is
/// answered.
#[derive(Debug, Clone)]
pub(super) struct ChildTarget {
    child: AgentSessionId,
    turn_id: AgentTurnId,
    generation: AgentGeneration,
    request: ApprovalRequest,
}

impl AgentBridge {
    pub(super) fn subagents_call(&mut self, call: SubagentsCall) -> Option<TurnUpdate> {
        match call {
            SubagentsCall::List => {
                let Some(parent) = self.session_id else {
                    return Some(reply(SubagentsReply::Listed(Vec::new())));
                };
                self.spawn(move |agent| async move {
                    let listed = match agent.subagents(parent).await {
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
            let listed = agent
                .subagents(parent)
                .await
                .map(|statuses| {
                    statuses
                        .into_iter()
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

    /// What the bridge makes of `/subagents`' list: it starts the watch
    /// when one runs.
    pub(super) fn subagents_listed(&mut self, statuses: &[AgentSubagentStatus]) {
        if statuses
            .iter()
            .any(|status| status.status == AgentSubagentState::Running)
        {
            self.watch_subagents();
        }
    }

    /// A tool that may start a subagent's turn started in the session's
    /// turn: the watch starts.
    pub(super) fn observe_tool(&mut self, name: &str) {
        if STARTING_TOOLS.contains(&name) {
            self.watch_subagents();
        }
    }

    /// Starts the watch over the session's subagents, unless it runs.
    pub(crate) fn watch_subagents(&mut self) {
        let Some(parent) = self.session_id else {
            return;
        };
        if self
            .watch
            .as_ref()
            .is_some_and(|watch| watch.parent == parent && watch.alive.load(Ordering::Acquire))
        {
            return;
        }
        let watch = Watch {
            parent,
            stop: Arc::new(AtomicBool::new(false)),
            alive: Arc::new(AtomicBool::new(true)),
        };
        let (stop, alive) = (watch.stop.clone(), watch.alive.clone());
        let agent = self.agent.clone();
        let app_event_tx = self.app_event_tx.clone();
        std::thread::spawn(move || {
            run_watch(&agent, parent, &stop, &app_event_tx);
            alive.store(false, Ordering::Release);
        });
        self.watch = Some(watch);
    }

    /// An approval a subagent of the session waits on: it is answered on
    /// the subagent.
    pub(super) fn child_approval(&mut self, approval: ChildApproval) -> Option<TurnUpdate> {
        if Some(approval.parent) != self.session_id {
            return None;
        }
        let mut request = approval.request;
        request.action.summary = format!("Subagent {}: {}", approval.title, request.action.summary);
        self.child_approvals.insert(
            request.request_id,
            ChildTarget {
                child: approval.child,
                turn_id: approval.turn_id,
                generation: approval.generation,
                request: request.clone(),
            },
        );
        Some(TurnUpdate::ApprovalRequested(request))
    }

    /// The session's turn ended, which closes the approvals shown: the
    /// subagents' still waiting are shown again.
    pub(super) fn reshow_child_approvals(&self) {
        for target in self.child_approvals.values() {
            let update = TurnUpdate::ApprovalRequested(target.request.clone());
            self.app_event_tx
                .send(AppEvent::AgentReply(AgentReply::Update(Box::new(update))));
        }
    }

    pub(super) fn child_approval_resolved(
        &mut self,
        parent: AgentSessionId,
        request_id: ApprovalRequestId,
    ) -> Option<TurnUpdate> {
        if Some(parent) != self.session_id {
            return None;
        }
        self.child_approvals
            .remove(&request_id)
            .map(|_| TurnUpdate::ApprovalResolved(request_id))
    }

    /// Answers a subagent's approval shown in the parent; `None` when
    /// `request_id` is the session's own.
    pub(super) fn resolve_child_approval(
        &mut self,
        request_id: ApprovalRequestId,
        decision: ApprovalUserDecision,
    ) -> Option<Option<TurnUpdate>> {
        let target = self.child_approvals.remove(&request_id)?;
        let answered = self.agent.resolve_approval(
            target.child,
            target.turn_id,
            target.generation,
            request_id,
            decision,
        );
        Some(answered.err().map(|error| {
            TurnUpdate::Error(format!("the subagent's approval was not answered: {error}"))
        }))
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

/// One subagent's journal as the watch reads it: where it stopped, and the
/// approvals it found waiting.
#[derive(Default)]
struct ChildJournal {
    after: Option<AgentEventSequence>,
    pending: Vec<ApprovalRequestId>,
}

fn run_watch(
    agent: &AgentHandle,
    parent: AgentSessionId,
    stop: &AtomicBool,
    app_event_tx: &AppEventSender,
) {
    let mut last = None;
    let mut journals: HashMap<AgentSessionId, ChildJournal> = HashMap::new();
    while !stop.load(Ordering::Acquire) {
        let call = agent.clone();
        let Ok(Ok(statuses)) = agent
            .runtime()
            .block_on(async move { call.subagents(parent).await })
        else {
            return;
        };
        let running: Vec<&AgentSubagentStatus> = statuses
            .iter()
            .filter(|status| status.status == AgentSubagentState::Running)
            .collect();
        for status in &running {
            let journal = journals.entry(status.session_id).or_default();
            read_child_journal(agent, parent, status, journal, app_event_tx);
        }
        // An approval of a subagent that no longer runs is moot.
        for (child, journal) in &mut journals {
            if !running.iter().any(|status| status.session_id == *child) {
                for request_id in journal.pending.drain(..) {
                    send(
                        app_event_tx,
                        AgentReply::ChildApprovalResolved { parent, request_id },
                    );
                }
            }
        }
        let parent_running = agent
            .snapshot(parent)
            .ok()
            .and_then(|snapshot| snapshot.turn)
            .is_some_and(|turn| !turn.phase.is_terminal());
        let any_running = !running.is_empty();
        if last.as_ref() != Some(&statuses) {
            let changed = TurnUpdate::Subagents(SubagentsReply::Changed(statuses.clone()));
            send(app_event_tx, AgentReply::Update(Box::new(changed)));
            last = Some(statuses);
        }
        if !any_running && !parent_running {
            return;
        }
        std::thread::sleep(WATCH_INTERVAL);
    }
}

/// Reads what `status`'s journal added: an approval asked and not yet
/// answered is shown, one answered (or whose turn ended) is withdrawn.
fn read_child_journal(
    agent: &AgentHandle,
    parent: AgentSessionId,
    status: &AgentSubagentStatus,
    journal: &mut ChildJournal,
    app_event_tx: &AppEventSender,
) {
    let Ok(batch) = agent.events_since(status.session_id, journal.after) else {
        return;
    };
    for event in batch.events {
        journal.after = Some(event.sequence);
        match event.kind {
            AgentEventKind::ApprovalRequested { request } => {
                let Some(turn_id) = event.turn_id else {
                    continue;
                };
                if request.expires_at_ms <= now_millis() {
                    continue;
                }
                journal.pending.push(request.request_id);
                let approval = ChildApproval {
                    parent,
                    child: status.session_id,
                    title: status.title.clone(),
                    turn_id,
                    generation: event.generation,
                    request,
                };
                send(app_event_tx, AgentReply::ChildApproval(Box::new(approval)));
            }
            AgentEventKind::ApprovalResolved { request_id, .. } => {
                if let Some(index) = journal.pending.iter().position(|id| *id == request_id) {
                    journal.pending.remove(index);
                    send(
                        app_event_tx,
                        AgentReply::ChildApprovalResolved { parent, request_id },
                    );
                }
            }
            kind if kind.is_terminal() => {
                for request_id in journal.pending.drain(..) {
                    send(
                        app_event_tx,
                        AgentReply::ChildApprovalResolved { parent, request_id },
                    );
                }
            }
            _ => {}
        }
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

fn send(app_event_tx: &AppEventSender, reply: AgentReply) {
    app_event_tx.send(AppEvent::AgentReply(reply));
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis())
}
