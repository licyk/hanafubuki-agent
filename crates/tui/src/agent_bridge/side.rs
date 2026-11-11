//! The second conversation's bridge: the side conversation's opening and
//! closing on the facade, and a subagent opened to be shown beside its
//! parent. A second bridge shares the Agent with the first; what it asks
//! comes back through its own sender, so the app tells the conversations
//! apart.

use hanafubuki_agent::{
    AgentError, AgentEventKind, AgentEventSequence, AgentSessionId, AgentSessionOrigin,
    AgentTurnAccepted, ApprovalRequest,
};

use super::{AgentBridge, OpenedSession, TurnUpdate};
use crate::app::WorkspaceEvent;
use crate::app_event::{AppEvent, AppEventSender};

/// Why a side conversation cannot start yet (Codex's wording).
const SIDE_NEEDS_A_TURN: &str = concat!(
    "'/side' is unavailable until the current conversation has started. ",
    "Send a message first, then try /side again."
);
const SIDE_DURING_REVIEW: &str =
    "'/side' is unavailable while a review runs; /review cancel stops it.";
const SIDE_DURING_COMPACTION: &str =
    "'/side' is unavailable while the conversation is being compacted; try again once it is done.";

/// A side conversation the Agent opened, ready to be shown.
#[derive(Debug)]
pub(crate) struct OpenedSide {
    pub(crate) parent: AgentSessionId,
    pub(crate) session: OpenedSession,
}

/// A subagent opened to be shown: its transcript, its parent's title,
/// where its journal is followed from, and the approvals its turn already
/// waits on there.
pub(crate) struct OpenedChild {
    pub(crate) session: OpenedSession,
    pub(crate) title: String,
    pub(crate) parent_title: String,
    after: Option<AgentEventSequence>,
    running: Option<AgentTurnAccepted>,
    waiting: Vec<ApprovalRequest>,
}

impl AgentBridge {
    /// A bridge over the same Agent with no session, whose replies and
    /// events go through `app_event_tx`.
    pub(crate) fn companion(&self, app_event_tx: AppEventSender) -> AgentBridge {
        AgentBridge::new(
            self.runtime.clone(),
            self.agent.clone(),
            self.model.clone(),
            self.role.clone(),
            app_event_tx,
        )
    }

    /// Opens the session's side conversation (or finds the open one) on a
    /// thread of its own: the facade reads the history to fork it.
    /// `message` is sent there once it is shown.
    pub(crate) fn open_side(&self, message: Option<String>) -> Result<(), String> {
        if self.review_running() {
            return Err(SIDE_DURING_REVIEW.to_string());
        }
        let parent = self.session_id.ok_or(SIDE_NEEDS_A_TURN)?;
        let agent = self.agent.clone();
        let app_event_tx = self.app_event_tx.clone();
        std::thread::spawn(move || {
            let opened = agent
                .open_side_conversation(parent)
                .map(|snapshot| OpenedSide {
                    parent,
                    session: OpenedSession {
                        session_id: Some(snapshot.id),
                        model: snapshot.model,
                        mode: snapshot.mode,
                        // The history it inherits is the parent's, shown
                        // there: the side starts empty, as Codex's.
                        transcript: Vec::new(),
                        older: None,
                        setup_needed: false,
                    },
                })
                .map_err(side_error);
            app_event_tx.send(AppEvent::Workspace(WorkspaceEvent::SideOpened {
                opened: opened.map(Box::new),
                message,
            }));
        });
        Ok(())
    }

    /// Closes the side conversation `side`: its turn is interrupted and it
    /// is dropped.
    pub(crate) fn close_side(&self, side: AgentSessionId) {
        let app_event_tx = self.app_event_tx.clone();
        let agent = self.agent.clone();
        self.agent.runtime().handle().spawn(async move {
            let closed = agent
                .close_side_conversation(side)
                .await
                .map_err(|error| format!("the side conversation was not closed: {error}"));
            app_event_tx.send(AppEvent::Workspace(WorkspaceEvent::SideClosed(closed)));
        });
    }

    /// Opens `child`, a subagent (or another child) of a session, with its
    /// transcript as `/resume` shows it.
    pub(crate) fn open_child(&self, child: AgentSessionId) -> Result<OpenedChild, String> {
        let (session, snapshot) = self.open_with_snapshot(child)?;
        let parent = snapshot
            .parent_session_id
            .ok_or("this session is not a child of another")?;
        if snapshot.origin == Some(AgentSessionOrigin::Side) {
            return Err("a side conversation is opened with /side".to_string());
        }
        let parent_title = self
            .agent
            .snapshot(parent)
            .ok()
            .and_then(|parent| parent.title)
            .map_or_else(|| parent.to_string(), |title| title.value);
        let running = snapshot
            .turn
            .filter(|turn| !turn.phase.is_terminal())
            .map(|turn| AgentTurnAccepted {
                session_id: child,
                turn_id: turn.id,
                generation: turn.generation,
            });
        let after = snapshot
            .next_event_sequence
            .value()
            .checked_sub(1)
            .filter(|sequence| *sequence > 0)
            .map(AgentEventSequence::new);
        let waiting = match (running, after) {
            (Some(turn), Some(after)) => self.waiting_approvals(turn, after),
            _ => Vec::new(),
        };
        Ok(OpenedChild {
            session,
            title: snapshot
                .title
                .map_or_else(|| child.to_string(), |title| title.value),
            parent_title,
            after,
            running,
            waiting,
        })
    }

    /// The approvals `turn` asked for through `after` and that are not yet
    /// answered: the journal is followed after them, so they are shown from
    /// here.
    fn waiting_approvals(
        &self,
        turn: AgentTurnAccepted,
        after: AgentEventSequence,
    ) -> Vec<ApprovalRequest> {
        let Ok(batch) = self.agent.events_since(turn.session_id, None) else {
            return Vec::new();
        };
        let mut waiting: Vec<ApprovalRequest> = Vec::new();
        for event in batch.events {
            if event.sequence > after || event.turn_id != Some(turn.turn_id) {
                continue;
            }
            match event.kind {
                AgentEventKind::ApprovalRequested { request } => waiting.push(request),
                AgentEventKind::ApprovalResolved { request_id, .. } => {
                    waiting.retain(|request| request.request_id != request_id);
                }
                _ => {}
            }
        }
        waiting
    }

    /// Shows `child` in this (second) bridge and follows its journal.
    pub(crate) fn show_child(&mut self, child: OpenedChild) -> Vec<TurnUpdate> {
        let session_id = child.session.session_id;
        let mut updates = Vec::new();
        if let Some(update) = self.reply(super::AgentReply::SessionOpened(child.session)) {
            updates.push(update);
        }
        if let Some(session_id) = session_id {
            updates.extend(self.follow(session_id, child.after, child.running));
            updates.extend(child.waiting.into_iter().map(TurnUpdate::ApprovalRequested));
        }
        updates
    }
}

fn side_error(error: AgentError) -> String {
    match error {
        AgentError::InvalidInput(message) if message.contains("no turn yet") => {
            SIDE_NEEDS_A_TURN.to_string()
        }
        AgentError::CompactionAlreadyRunning => SIDE_DURING_COMPACTION.to_string(),
        error => format!("Failed to start side conversation: {error}"),
    }
}
