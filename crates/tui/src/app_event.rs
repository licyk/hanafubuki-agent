// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! What the app loop is told: the terminal's input, what the Agent said,
//! and the actions the widgets ask of the app or of the Agent.

use std::sync::mpsc::Sender;
use std::time::Instant;

use hanafubuki_agent::{
    AgentEvent, AgentUserInputAnswer, AgentUserInputRequestId, ApprovalRequestId,
    ApprovalUserDecision,
};
use ratatui::crossterm::event::KeyEvent;

use crate::agent_bridge::AgentReply;
use crate::history_cell::HistoryCell;

pub(crate) enum AppEvent {
    Key(KeyEvent),
    Paste(String),
    /// The terminal changed size, or must be drawn again.
    Redraw,
    /// Draw no later than this.
    ScheduleFrame(Instant),
    /// An event of the session's turns, as the Agent emitted it.
    Agent(Box<AgentEvent>),
    /// The Agent's slot changed state (stopped, faulted, restarted).
    AgentStatusChanged,
    /// A call into the Agent finished.
    AgentReply(AgentReply),
    /// Something for the Agent to do.
    Op(Op),
    InsertHistoryCell(Box<dyn HistoryCell>),
    StartCommitAnimation,
    StopCommitAnimation,
    CommitTick,
    Exit,
}

/// The widgets' requests of the Agent, which the bridge turns into calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Op {
    /// A message from the person: a new turn, or steering of the running one.
    UserInput(String),
    Interrupt,
    Approval {
        request_id: ApprovalRequestId,
        decision: ApprovalUserDecision,
    },
    UserInputAnswer {
        request_id: AgentUserInputRequestId,
        answers: Vec<AgentUserInputAnswer>,
    },
}

/// The app loop's channel, as the widgets and the Agent's sinks hold it.
#[derive(Clone, Debug)]
pub(crate) struct AppEventSender {
    app_event_tx: Sender<AppEvent>,
}

impl AppEventSender {
    pub(crate) fn new(app_event_tx: Sender<AppEvent>) -> Self {
        Self { app_event_tx }
    }

    /// Sends an event to the app loop; once the loop is gone, nothing
    /// listens and the event is dropped.
    pub(crate) fn send(&self, event: AppEvent) {
        let _ = self.app_event_tx.send(event);
    }
}
