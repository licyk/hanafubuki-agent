// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! What the app loop is told: the terminal's input, what the Agent said,
//! and the actions the widgets ask of the app or of the Agent.

use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;

use hanafubuki_agent::{
    AgentEvent, AgentSessionId, AgentSessionMode, AgentUserInputAnswer, AgentUserInputRequestId,
    ApprovalRequestId, ApprovalUserDecision,
};
use ratatui::crossterm::event::{KeyEvent, MouseEvent};

use hanafubuki_agent::AgentTuiPreferencesPatch;

use crate::agent_bridge::{
    AgentReply, GoalCall, HooksCall, McpCall, PluginsCall, ProviderCall, ReviewCall, SessionCall,
    SettingsCall, SubagentsCall,
};
use crate::app::WorkspaceEvent;
use crate::appearance::AppearanceEvent;
use crate::commands::CommandStep;
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
    /// An entry that changes in place, at its start: the transcript shows
    /// it from now on, and Scrollback writes it once it completes.
    InsertLiveEntry(Arc<dyn HistoryCell>),
    /// A live entry that completed: Scrollback writes it as it is shown.
    WriteLiveEntry(Arc<dyn HistoryCell>),
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
    /// What a slash command asks of the app itself.
    Command(AppCommand),
    /// `/theme`, `/statusline`, `/title`: a change, for this run or saved.
    Appearance(AppearanceEvent),
    /// A slash command's next view, asked by one of its views.
    CommandStep(CommandStep),
    /// What the second conversation's widgets and bridge sent (the side
    /// conversation, a subagent shown): the number of the conversation,
    /// so a closed one's late events are dropped.
    Second(u64, Box<AppEvent>),
    /// `/side`, `/subagents`, `/agents` and the panes: what the app does
    /// with its conversations.
    Workspace(WorkspaceEvent),
    StartCommitAnimation,
    StopCommitAnimation,
    CommitTick,
    Exit,
}

/// What a slash command asks of the app: the clipboard, the transcript,
/// the screen, the composer's submission, the resume picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AppCommand {
    /// Copies the text to the clipboard and says so.
    Copy(String),
    /// Owned mode: select mode in the transcript, to copy with the keys.
    SelectTranscript,
    /// `/clear`: the terminal's scrollback and screen (terminal mode) or
    /// the transcript (owned mode) are cleared.
    ClearScreen,
    /// Submits the text as the person's message.
    Submit(String),
    /// `/resume TEXT`: the session with this id, else the one session with
    /// this title, else the picker searching for it.
    ResumeNamed(String),
    /// Runs the text as if it were typed and sent: a command, or a message
    /// (`/presets`, `/config`'s entries).
    Replay(String),
    /// Saves the change into `[tui]` and applies it (the keymap, the
    /// presets), then says the message, or why it was not saved.
    SavePreferences(AgentTuiPreferencesPatch, String),
    /// Hides the warning with this key (`Warning::key`) for the run.
    DismissWarning(String),
}

/// The widgets' requests of the Agent, which the bridge turns into calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Op {
    /// A message from the person that starts a turn.
    UserInput(String),
    /// A message that steers the running turn; `id` names it in the answer
    /// (`TurnUpdate::Steered`).
    Steer {
        id: u64,
        text: String,
    },
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
    /// `/model forget`: what the Provider's refusals taught of the
    /// session's model is forgotten.
    ForgetLearnedModelFacts,
    SetMode(AgentSessionMode),
    /// The roles `/role` offers.
    ListRoles,
    SetRole(String),
    Status,
    Providers(ProviderCall),
    Mcp(McpCall),
    Skills,
    Session(SessionCall),
    Review(ReviewCall),
    Goal(GoalCall),
    Hooks(HooksCall),
    Plugins(PluginsCall),
    Settings(SettingsCall),
    Subagents(SubagentsCall),
}

/// The app loop's channel, as the widgets and the Agent's sinks hold it.
#[derive(Clone, Debug)]
pub(crate) struct AppEventSender {
    app_event_tx: Sender<AppEvent>,
    /// The second conversation's number: what it sends arrives wrapped in
    /// [`AppEvent::Second`].
    second: Option<u64>,
}

impl AppEventSender {
    pub(crate) fn new(app_event_tx: Sender<AppEvent>) -> Self {
        Self {
            app_event_tx,
            second: None,
        }
    }

    /// The sender of the second conversation numbered `id`.
    pub(crate) fn for_second(&self, id: u64) -> Self {
        Self {
            app_event_tx: self.app_event_tx.clone(),
            second: Some(id),
        }
    }

    /// The same channel, for the app's own events: they arrive unwrapped.
    pub(crate) fn untagged(&self) -> Self {
        Self {
            app_event_tx: self.app_event_tx.clone(),
            second: None,
        }
    }

    /// Sends an event to the app loop; once the loop is gone, nothing
    /// listens and the event is dropped.
    pub(crate) fn send(&self, event: AppEvent) {
        let event = match self.second {
            Some(id) => AppEvent::Second(id, Box::new(event)),
            None => event,
        };
        let _ = self.app_event_tx.send(event);
    }
}
