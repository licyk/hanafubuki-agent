// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The selected agent and the session's side conversation occupy independent
//! panes. The app owns the focused conversation; the side/agent pane and a
//! selected child's parent retain their widgets, bridges, drafts and streams
//! while parked. `routing` delivers each conversation's events to its owner.
//!
//! Fullscreen shows the selected agent beside the side conversation when wide
//! enough; Scrollback shows the focused conversation and flushes completed
//! output when switching. Ctrl+/ switches pane focus, while Alt+Left/Right
//! selects an agent without closing the side conversation.

mod agents;
mod child;
mod keys;
mod panes;
mod routing;
mod settings;
mod tasks;

use std::io::{self, Write};
use std::sync::Arc;
use std::time::Instant;

use hanafubuki_agent::AgentSessionId;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::{new_view, App};
use crate::agent_bridge::{
    AgentBridge, AgentReply, CatalogReply, CenterRead, CenterReader, OlderTranscript, OpenedSide,
};
use crate::agents_center::AgentsCenter;
use crate::app_event::AppEventSender;
use crate::chatwidget::ChatWidget;
use crate::history_cell::HistoryCell;
use crate::keymap::{self, Action};
use crate::settings_center::SettingsCenter;
use crate::transcript_view::{TranscriptHistoryState, TranscriptView};
use crate::tui::Tui;
use crate::user_message::UserMessage;
use child::Child;
pub(crate) use keys::PaneAction;
pub(super) use panes::{paint_chrome, PaneLayout};
pub(super) use routing::Route;

/// What the app does with its conversations.
pub(crate) enum WorkspaceEvent {
    /// `/side [message]`: the side conversation, opened or shown again;
    /// the message is sent there.
    OpenSide(Option<String>),
    /// `/side close`.
    CloseSide,
    /// The Agent opened the side conversation (or why it did not).
    SideOpened {
        opened: Result<Box<OpenedSide>, String>,
        message: Option<String>,
    },
    SideClosed(Result<(), String>),
    /// Shows a subagent in place of the main conversation.
    OpenChild(AgentSessionId),
    /// Alt+Right (`forward`) or Alt+Left on an empty composer: the next or
    /// previous of the main session and its subagents.
    StepAgent {
        forward: bool,
    },
    /// The subagents of `parent` were listed for a step.
    StepAgentListed {
        parent: AgentSessionId,
        forward: bool,
        listed: Result<Vec<AgentSessionId>, String>,
    },
    /// The focus moves to the other pane (a click in it).
    FocusOther,
    /// `/agents`.
    OpenAgents,
    Tasks(Box<crate::tasks::TasksEvent>),
    /// `/config [target]`: the settings center, at the tab, section,
    /// collection or page named; a record's page on `record`.
    OpenSettings {
        target: Option<String>,
        record: Option<String>,
    },
    /// What a page of the center asks of a collection.
    SettingsAction(SettingsPageAction),
    /// What a call of the settings center found.
    Settings(Box<CatalogReply>),
    /// What a read of the command center found, or its watch saw.
    AgentsCenter(Box<CenterRead>),
    /// `tui-state.json` was not written.
    TuiStateNotSaved(String),
    /// Whether the command whose views are the center's page still shows
    /// one; sent after the keys it took, so that the step one of its views
    /// asked for has shown its next view first. Only the latest check
    /// counts: one sent before a later key comes before that key's step.
    SettingsPageCheck(u64),
}

/// An action of a collection a page of the settings center runs (a
/// Provider's model chosen), at the revision the center read.
#[derive(Debug)]
pub(crate) struct SettingsPageAction {
    pub(crate) collection: String,
    pub(crate) record: Option<String>,
    pub(crate) action: String,
    pub(crate) values: serde_json::Map<String, serde_json::Value>,
}

/// Which conversation: the main one, or the second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Focus {
    Main,
    Second,
}

impl Focus {
    fn other(self) -> Self {
        match self {
            Self::Main => Self::Second,
            Self::Second => Self::Main,
        }
    }
}

/// The conversation the app's fields do not hold.
pub(super) struct Parked {
    pub(super) chat_widget: ChatWidget,
    pub(super) bridge: AgentBridge,
    pub(super) app_event_tx: AppEventSender,
    pub(super) transcript_cells: Vec<Arc<dyn HistoryCell>>,
    pub(super) transcript_view: TranscriptView,
    pub(super) tail: super::scrollback::Tail,
    pub(super) older_transcript: Option<OlderTranscript>,
}

pub(super) struct Second {
    /// The number its events carry.
    id: u64,
    parent: AgentSessionId,
    session: AgentSessionId,
    pub(super) parked: Parked,
    /// Which conversation the app's fields hold.
    pub(super) focus: Focus,
}

#[derive(Default)]
pub(super) struct Workspace {
    second: Option<Second>,
    child: Option<Child>,
    next_id: u64,
    /// The side conversation is being opened.
    opening_side: bool,
    /// The side pane's width as the divider was moved to.
    side_width: Option<u16>,
    /// The focused pane fills the screen.
    zoomed: bool,
    /// The divider is being dragged.
    dragging: bool,
    /// The panes as last drawn, for the pointer.
    layout: Option<PaneLayout>,
    /// The parked conversation whose event is being handled.
    handling: Option<Focus>,
    agents: Option<AgentsCenter>,
    pub(super) tasks: Option<crate::tasks::Tasks>,
    /// The command center's reads, while it is open.
    agents_reads: Option<CenterReader>,
    /// The session the main conversation showed, as last noted.
    shown: Option<AgentSessionId>,
    settings: Option<SettingsCenter>,
}

impl Workspace {
    /// The settings center, when it is open.
    #[cfg(test)]
    pub(crate) fn settings(&self) -> Option<&SettingsCenter> {
        self.settings.as_ref()
    }

    /// The settings center, when it is open, to tell it what a page it
    /// hosts saved.
    pub(crate) fn settings_mut(&mut self) -> Option<&mut SettingsCenter> {
        self.settings.as_mut()
    }

    pub(super) fn second_focused(&self) -> bool {
        self.second
            .as_ref()
            .is_some_and(|second| second.focus == Focus::Second)
    }

    pub(super) fn agents_open(&self) -> bool {
        self.agents.is_some()
    }

    pub(super) fn agents_mut(&mut self) -> Option<&mut AgentsCenter> {
        self.agents.as_mut()
    }

    pub(super) fn settings_open(&self) -> bool {
        self.settings.is_some()
    }

    /// Whether the conversation not in the app's fields has no streamed
    /// row queued.
    pub(super) fn parked_streams_idle(&self) -> bool {
        self.second
            .as_ref()
            .is_none_or(|second| second.parked.chat_widget.stream_queues_idle())
            && self
                .child
                .as_ref()
                .is_none_or(|child| child.parked.chat_widget.stream_queues_idle())
    }

    /// An event of the parked conversation is being handled.
    pub(super) fn handling_parked(&self) -> bool {
        self.handling.is_some()
    }

    pub(super) fn parked_mut(&mut self) -> Option<&mut Parked> {
        self.second.as_mut().map(|second| &mut second.parked)
    }

    pub(super) fn parent_mut(&mut self) -> Option<&mut Parked> {
        self.child.as_mut().map(|child| &mut child.parked)
    }

    pub(super) fn parent(&self) -> Option<&Parked> {
        self.child.as_ref().map(|child| &child.parked)
    }

    pub(super) fn is_dragging(&self) -> bool {
        self.dragging
    }

    #[cfg(test)]
    pub(super) fn last_layout(&self) -> Option<&PaneLayout> {
        self.layout.as_ref()
    }
}

impl App {
    pub(super) fn workspace_event<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        event: WorkspaceEvent,
    ) {
        match event {
            WorkspaceEvent::OpenSide(message) => self.open_side(tui, message),
            WorkspaceEvent::CloseSide => self.close_second(tui),
            WorkspaceEvent::SideOpened { opened, message } => {
                self.workspace.opening_side = false;
                match opened {
                    Ok(side) => self.side_opened(tui, *side, message),
                    Err(error) => self.chat_widget.add_error_message(error),
                }
            }
            WorkspaceEvent::SideClosed(closed) => match closed {
                Ok(()) => self
                    .chat_widget
                    .add_info_message("Side conversation closed.".to_string()),
                Err(error) => self.chat_widget.add_error_message(error),
            },
            WorkspaceEvent::OpenChild(child) => self.open_child(tui, child),
            WorkspaceEvent::StepAgent { forward } => self.list_agents_to_step(forward),
            WorkspaceEvent::StepAgentListed {
                parent,
                forward,
                listed,
            } => self.step_agent(tui, parent, forward, listed),
            WorkspaceEvent::FocusOther => {
                if let Some(focus) = self.workspace.second.as_ref().map(|second| second.focus) {
                    self.set_focus(tui, focus.other());
                }
            }
            WorkspaceEvent::OpenAgents => self.open_agents_center(tui),
            WorkspaceEvent::Tasks(event) => self.tasks_event(tui, *event),
            WorkspaceEvent::OpenSettings { target, record } => {
                self.open_settings_center(tui, target, record)
            }
            WorkspaceEvent::SettingsAction(action) => self.run_settings_action(tui, action),
            WorkspaceEvent::Settings(reply) => self.settings_reply(tui, *reply),
            WorkspaceEvent::SettingsPageCheck(check) => self.check_settings_page(check),
            WorkspaceEvent::AgentsCenter(read) => self.center_read(*read),
            WorkspaceEvent::TuiStateNotSaved(error) => self.tui_state_not_saved(error),
        }
        self.schedule_frame(Instant::now());
    }

    /// The session the main conversation shows.
    pub(super) fn main_session_id(&self) -> Option<AgentSessionId> {
        if let Some(child) = &self.workspace.child {
            return child.parked.bridge.session_id();
        }
        match &self.workspace.second {
            Some(second) if second.focus == Focus::Second => second.parked.bridge.session_id(),
            _ => self.bridge.session_id(),
        }
    }

    fn open_side<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        message: Option<String>,
    ) {
        if self.workspace.second.is_some() {
            self.set_focus(tui, Focus::Second);
            if let Some(message) = message {
                self.chat_widget
                    .submit_user_message(UserMessage::from_text(message));
            }
            return;
        }
        if self.workspace.opening_side {
            return;
        }
        match self.main_bridge().open_side(message) {
            Ok(()) => self.workspace.opening_side = true,
            Err(error) => self.chat_widget.add_error_message(error),
        }
    }

    fn side_opened<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        side: OpenedSide,
        message: Option<String>,
    ) {
        let Some(session) = side.session.session_id else {
            return;
        };
        // The interface moved to another session meanwhile.
        if self.workspace.second.is_some() || self.main_session_id() != Some(side.parent) {
            self.bridge.close_side(session);
            return;
        }
        self.install_second(tui, side.parent, session);
        if let Some(update) = self.bridge.reply(AgentReply::SessionOpened(side.session)) {
            self.show_update(update);
        }
        if let Some(message) = message {
            self.chat_widget
                .submit_user_message(UserMessage::from_text(message));
        }
    }

    /// A side conversation, with a chat and a bridge of its
    /// own, focused; the main one is parked.
    fn install_second<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        parent: AgentSessionId,
        session: AgentSessionId,
    ) {
        let (id, parked) = self.new_parked();
        self.workspace.zoomed = false;
        self.workspace.second = Some(Second {
            id,
            parent,
            session,
            parked,
            focus: Focus::Main,
        });
        self.set_focus(tui, Focus::Second);
    }

    fn new_parked(&mut self) -> (u64, Parked) {
        self.workspace.next_id += 1;
        let id = self.workspace.next_id;
        let app_event_tx = self.app_event_tx.for_second(id);
        let parked = Parked {
            chat_widget: self.chat_widget.second(app_event_tx.clone()),
            bridge: self.bridge.companion(app_event_tx.clone()),
            app_event_tx,
            transcript_cells: Vec::new(),
            transcript_view: new_view(self.render_mode, TranscriptHistoryState::Complete),
            tail: super::scrollback::Tail::default(),
            older_transcript: None,
        };
        (id, parked)
    }

    /// Gives `focus` the keys: in scrollback mode the interface switches to
    /// it, under its banner, the tail of the one it leaves written as far as
    /// it has completed.
    pub(super) fn set_focus<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        focus: Focus,
    ) {
        if self
            .workspace
            .second
            .as_ref()
            .is_none_or(|second| second.focus == focus)
        {
            return;
        }
        self.flush_before_switch(tui);
        self.swap_conversations();
        self.show_focus_banner(tui, focus);
    }

    /// Return from the side to the selected agent, or from a child to its parent.
    fn return_to_main<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        if self.workspace.second_focused() {
            self.set_focus(tui, Focus::Main);
        } else {
            self.leave_child(tui);
        }
    }

    fn close_second<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        if let Some(side) = self.workspace.second.as_ref().map(|second| second.session) {
            self.leave_second(tui);
            self.bridge.close_side(side);
        } else if self.workspace.child.is_some() {
            self.leave_child(tui);
        } else {
            self.chat_widget
                .add_info_message("No side conversation is open.".to_string());
        }
    }

    /// Drops the second conversation, the main one focused.
    fn leave_second<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        self.set_focus(tui, Focus::Main);
        self.workspace.second = None;
        self.workspace.zoomed = false;
        self.workspace.dragging = false;
        self.workspace.layout = None;
        self.schedule_frame(Instant::now());
    }

    /// After an event: a side conversation whose parent the main one no
    /// longer shows (`/new`, `/resume`, …) is closed.
    pub(super) fn sync_second<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        let Some(Second {
            parent, session, ..
        }) = self.workspace.second.as_ref()
        else {
            return;
        };
        let (parent, session) = (*parent, *session);
        if self.main_session_id() != Some(parent) {
            self.leave_second(tui);
            self.bridge.close_side(session);
        }
    }

    /// The panes' keys (a pane action, wherever it is read), and in a
    /// second conversation the key that edits an earlier message (Esc),
    /// which returns from it instead. `true` when one was taken.
    pub(super) fn handle_workspace_key<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        key: KeyEvent,
    ) -> bool {
        if self.overlay.is_some() {
            return false;
        }
        if (self.workspace.second_focused() || self.workspace.child.is_some())
            && key.kind == KeyEventKind::Press
            && key.code == KeyCode::Char('c')
            && key.modifiers == KeyModifiers::CONTROL
            && self.chat_widget.no_modal_or_popup_active()
        {
            if self.workspace.second_focused() {
                self.close_second(tui);
            } else {
                self.leave_child(tui);
            }
            return true;
        }
        if let Some(action) = PaneAction::of(&key) {
            self.pane_action(tui, action);
            return true;
        }
        if keymap::matches(Action::OpenAgents, &key) {
            self.open_agents_center(tui);
            return true;
        }
        if keymap::matches(Action::OpenTasks, &key) {
            self.tasks_event(tui, crate::tasks::TasksEvent::Open);
            return true;
        }
        let returns = (self.workspace.second_focused() || self.workspace.child.is_some())
            && key.kind == KeyEventKind::Press
            && keymap::matches(Action::EditPrevious, &key)
            && self.chat_widget.no_modal_or_popup_active()
            && self.chat_widget.composer_is_empty()
            && !self.chat_widget.is_task_running()
            && !self.chat_widget.has_pending_input();
        if returns {
            self.return_to_main(tui);
        }
        returns
    }

    pub(super) fn pane_action<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        action: PaneAction,
    ) {
        let focus = self.workspace.second.as_ref().map(|second| second.focus);
        match action {
            PaneAction::FocusMain => self.return_to_main(tui),
            PaneAction::FocusSecond => self.set_focus(tui, Focus::Second),
            PaneAction::ToggleFocus if focus == Some(Focus::Second) => {
                self.set_focus(tui, Focus::Main)
            }
            PaneAction::ToggleFocus => self.set_focus(tui, Focus::Second),
            PaneAction::DividerLeft => self.workspace.move_divider(4),
            PaneAction::DividerRight => self.workspace.move_divider(-4),
            PaneAction::EqualWidths => {
                if let Ok(size) = tui.terminal.size() {
                    self.workspace.side_width = Some(size.width.saturating_sub(1) / 2);
                }
            }
            PaneAction::ToggleZoom => self.workspace.zoomed = !self.workspace.zoomed,
            PaneAction::Close => self.close_second(tui),
        }
        self.schedule_frame(Instant::now());
    }
}
