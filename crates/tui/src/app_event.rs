// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! What the app loop is told: the terminal's input, what the Agent said,
//! and the actions the widgets ask of the app or of the Agent.

use std::sync::mpsc::Sender;
use std::time::Instant;

use hanafubuki_agent::{
    AgentEvent, AgentSessionId, AgentSessionMode, AgentUserInputAnswer, AgentUserInputRequestId,
    ApprovalRequestId, ApprovalUserDecision,
};
use ratatui::crossterm::event::{KeyEvent, MouseEvent};

use crate::agent_bridge::AgentReply;
use crate::file_search::FileMatch;
use crate::history_cell::HistoryCell;

pub(crate) enum AppEvent {
    Key(KeyEvent),
    Paste(String),
    /// The pointer, reported while a transcript view captures it.
    Mouse(MouseEvent),
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
    /// The `@` token being typed changed; an empty query ends the search.
    StartFileSearch(String),
    /// The best files for `query` so far.
    FileSearchResult {
        query: String,
        matches: Vec<FileMatch>,
    },
    /// `/diff`: run `git diff` in the workspace.
    Diff,
    /// Its output, for the pager.
    DiffResult(String),
    /// `/resume`: the sessions to pick from.
    OpenResumePicker,
    /// `/raw`: on, off, or (`None`) toggled.
    SetRawOutput(Option<bool>),
    /// `/tui`: the transcript's mode at the next launch.
    SetFullscreenTranscript(bool),
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
    Command(AgentCommand),
    /// A command that needs a session runs next: one is created first when
    /// there is none.
    EnsureSession,
}

/// What the slash commands ask of the Agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentCommand {
    NewSession,
    Resume(AgentSessionId),
    Compact,
    /// The models `/model` offers.
    ListModels,
    SetModel {
        provider_id: String,
        model: String,
    },
    SetMode(AgentSessionMode),
    Status,
    Mcp,
    Skills,
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
