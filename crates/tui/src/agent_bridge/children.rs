//! What a subagent of the shown session asks of a person. The Agent tells
//! the host each approval and question a subagent's turn asks, and when it
//! is answered or the turn ends (`AgentBackgroundEvent::Child`): the bridge
//! shows it as the session's own, labelled with the subagent's title
//! (`Subagent counter: ...`), answers it on the subagent, and closes it when
//! it is answered elsewhere or the subagent's turn ends. Those still asked
//! when the session's own turn ends are shown again, since its end closes
//! what is shown.
//!
//! The Agent tells each once, as it happens: a session opened while a
//! subagent waits (resumed, or come back to from a subagent shown in its
//! place) reads its running subagents' journals once for what they still
//! ask.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use hanafubuki_agent::{
    AgentBackgroundEvent, AgentEvent, AgentEventKind, AgentGeneration, AgentHandle, AgentSessionId,
    AgentSubagentState, AgentTurnId, AgentUserInputAnswer, AgentUserInputRequest,
    AgentUserInputRequestId, ApprovalRequest, ApprovalRequestId, ApprovalUserDecision,
};

use super::{AgentBridge, AgentReply, TurnUpdate};
use crate::app_event::{AppEvent, AppEventSender};

/// What the subagents ask that the bridge shows, by request.
#[derive(Default)]
pub(super) struct ChildRequests {
    approvals: HashMap<ApprovalRequestId, Asked<ApprovalRequest>>,
    questions: HashMap<AgentUserInputRequestId, Asked<AgentUserInputRequest>>,
}

/// A request as shown, labelled, and the turn that answers it.
struct Asked<T> {
    child: AgentSessionId,
    turn_id: AgentTurnId,
    generation: AgentGeneration,
    shown: T,
}

impl AgentBridge {
    /// An event of a subagent's turn in `root`'s tree: what it asks is
    /// shown when `root` is the session shown.
    pub(super) fn child_event(
        &mut self,
        root: AgentSessionId,
        event: AgentEvent,
    ) -> Option<TurnUpdate> {
        if Some(root) != self.session_id {
            return None;
        }
        let child = event.session_id;
        let turn_id = event.turn_id?;
        // Opening a session reads its pending requests off-thread. A live
        // resolution can reach the interface before that initial read.
        if matches!(
            event.kind,
            AgentEventKind::ApprovalRequested { .. } | AgentEventKind::UserInputRequested { .. }
        ) && self.request_was_resolved(&event)
        {
            return None;
        }
        match event.kind {
            AgentEventKind::ApprovalRequested { mut request }
                if !self.children.approvals.contains_key(&request.request_id) =>
            {
                request.action.summary = format!(
                    "Subagent {}: {}",
                    self.child_title(child),
                    request.action.summary
                );
                let asked = Asked {
                    child,
                    turn_id,
                    generation: event.generation,
                    shown: request.clone(),
                };
                self.children.approvals.insert(request.request_id, asked);
                Some(TurnUpdate::ApprovalRequested(request))
            }
            AgentEventKind::UserInputRequested { mut request }
                if !self.children.questions.contains_key(&request.request_id) =>
            {
                let title = self.child_title(child);
                for question in &mut request.questions {
                    question.header = format!("Subagent {title}: {}", question.header);
                }
                let asked = Asked {
                    child,
                    turn_id,
                    generation: event.generation,
                    shown: request.clone(),
                };
                self.children.questions.insert(request.request_id, asked);
                Some(TurnUpdate::UserInputRequested(request))
            }
            AgentEventKind::ApprovalResolved { request_id, .. } => self
                .children
                .approvals
                .remove(&request_id)
                .map(|_| TurnUpdate::ApprovalResolved(request_id)),
            AgentEventKind::UserInputResolved { resolution } => self
                .children
                .questions
                .remove(&resolution.request_id)
                .map(|_| TurnUpdate::UserInputResolved(resolution.request_id)),
            kind if kind.is_terminal() => {
                self.close_child_requests(child);
                None
            }
            _ => None,
        }
    }

    /// Answers a subagent's approval shown in the parent; `None` when
    /// `request_id` is the session's own.
    pub(super) fn resolve_child_approval(
        &mut self,
        request_id: ApprovalRequestId,
        decision: ApprovalUserDecision,
    ) -> Option<Option<TurnUpdate>> {
        let asked = self.children.approvals.remove(&request_id)?;
        let answered = self.agent.resolve_approval(
            asked.child,
            asked.turn_id,
            asked.generation,
            request_id,
            decision,
        );
        Some(answered.err().map(|error| {
            TurnUpdate::Error(format!("the subagent's approval was not answered: {error}"))
        }))
    }

    /// Answers a subagent's question shown in the parent; the answers back
    /// when `request_id` is the session's own.
    pub(super) fn resolve_child_question(
        &mut self,
        request_id: AgentUserInputRequestId,
        answers: Vec<AgentUserInputAnswer>,
    ) -> Result<Option<TurnUpdate>, Vec<AgentUserInputAnswer>> {
        let Some(asked) = self.children.questions.remove(&request_id) else {
            return Err(answers);
        };
        let answered = self.agent.resolve_user_input(
            asked.child,
            asked.turn_id,
            asked.generation,
            request_id,
            answers,
        );
        Ok(answered.err().map(|error| {
            TurnUpdate::Error(format!("the subagent's question was not answered: {error}"))
        }))
    }

    /// The session's turn ended, which closes what is shown: what the
    /// subagents still ask is shown again.
    pub(super) fn reshow_child_requests(&self) {
        let approvals = (self.children.approvals.values())
            .map(|asked| TurnUpdate::ApprovalRequested(asked.shown.clone()));
        let questions = (self.children.questions.values())
            .map(|asked| TurnUpdate::UserInputRequested(asked.shown.clone()));
        for update in approvals.chain(questions) {
            self.send_update(update);
        }
    }

    /// Another session is shown: what its subagents still ask is read once.
    pub(super) fn read_child_requests(&mut self) {
        self.children = ChildRequests::default();
        let Some(parent) = self.session_id else {
            return;
        };
        let agent = self.agent.clone();
        let app_event_tx = self.app_event_tx.clone();
        std::thread::spawn(move || read_asked(&agent, parent, &app_event_tx));
    }

    /// `child`'s turn ended: what it asked is moot.
    fn close_child_requests(&mut self, child: AgentSessionId) {
        let approvals = self.children.approvals.iter();
        let closed_approvals = (approvals.filter(|(_, asked)| asked.child == child))
            .map(|(request_id, _)| *request_id)
            .collect::<Vec<_>>();
        let questions = self.children.questions.iter();
        let closed_questions = (questions.filter(|(_, asked)| asked.child == child))
            .map(|(request_id, _)| *request_id)
            .collect::<Vec<_>>();
        for request_id in closed_approvals {
            self.children.approvals.remove(&request_id);
            self.send_update(TurnUpdate::ApprovalResolved(request_id));
        }
        for request_id in closed_questions {
            self.children.questions.remove(&request_id);
            self.send_update(TurnUpdate::UserInputResolved(request_id));
        }
    }

    fn child_title(&self, child: AgentSessionId) -> String {
        self.agent
            .snapshot(child)
            .ok()
            .and_then(|snapshot| snapshot.title)
            .map_or_else(|| child.to_string(), |title| title.value)
    }

    fn request_was_resolved(&self, request: &AgentEvent) -> bool {
        let Ok(later) = self
            .agent
            .events_since(request.session_id, Some(request.sequence))
        else {
            return true;
        };
        later.events.iter().any(|event| {
            if event.turn_id == request.turn_id && event.kind.is_terminal() {
                return true;
            }
            match (&request.kind, &event.kind) {
                (
                    AgentEventKind::ApprovalRequested { request },
                    AgentEventKind::ApprovalResolved { request_id, .. },
                ) => request.request_id == *request_id,
                (
                    AgentEventKind::UserInputRequested { request },
                    AgentEventKind::UserInputResolved { resolution },
                ) => request.request_id == resolution.request_id,
                _ => false,
            }
        })
    }

    fn send_update(&self, update: TurnUpdate) {
        let update = AgentReply::Update(Box::new(update));
        self.app_event_tx.send(AppEvent::AgentReply(update));
    }
}

/// Tells the loop what `parent`'s running subagents still ask, as the Agent
/// would have, from their journals: an approval or a question asked and not
/// answered, in a turn that has not ended, and an approval not expired.
fn read_asked(agent: &AgentHandle, parent: AgentSessionId, app_event_tx: &AppEventSender) {
    let reader = agent.clone();
    let Ok(Ok(statuses)) = agent
        .runtime()
        .block_on(async move { reader.subagents(parent).await })
    else {
        return;
    };
    let running = statuses
        .iter()
        .filter(|status| status.status == AgentSubagentState::Running);
    for status in running {
        let Ok(batch) = agent.events_since(status.session_id, None) else {
            continue;
        };
        for event in asked(batch.events) {
            let child = AgentBackgroundEvent::Child {
                root: parent,
                session_id: status.session_id,
                event: Box::new(event),
            };
            app_event_tx.send(AppEvent::AgentBackground(child));
        }
    }
}

/// The requests of `events` (a journal) still waiting for an answer.
fn asked(events: Vec<AgentEvent>) -> Vec<AgentEvent> {
    let mut asked: Vec<AgentEvent> = Vec::new();
    for event in events {
        match &event.kind {
            AgentEventKind::ApprovalRequested { request } if request.expires_at_ms > now() => {
                asked.push(event)
            }
            AgentEventKind::UserInputRequested { .. } => asked.push(event),
            AgentEventKind::ApprovalResolved { request_id, .. } => asked.retain(|asked| {
                !matches!(&asked.kind, AgentEventKind::ApprovalRequested { request }
                    if request.request_id == *request_id)
            }),
            AgentEventKind::UserInputResolved { resolution } => asked.retain(|asked| {
                !matches!(&asked.kind, AgentEventKind::UserInputRequested { request }
                    if request.request_id == resolution.request_id)
            }),
            kind if kind.is_terminal() => asked.clear(),
            _ => {}
        }
    }
    asked
}

fn now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis())
}
