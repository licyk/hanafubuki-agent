// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The workspace (Codex's layer H: `app/side.rs`, `app/side_pane.rs`,
//! `app/side_focus.rs`, `app/workspace_hud.rs`): a second conversation
//! beside the main one, which is the session's side conversation (`/side`)
//! or one of its subagents shown (`/subagents`), and the command center
//! (`/agents`, in `agents`).
//!
//! Each conversation is a whole chat: its chat widget, its bridge, its
//! cells and transcript view. The app's own fields hold the conversation
//! that has the focus; the other is parked here ([`Parked`]), and focusing
//! it swaps the two. What the second conversation's widgets and bridge
//! send arrives wrapped in [`AppEvent::Second`] (their sender is
//! [`AppEventSender::for_second`]); a conversation's events are handled
//! with its fields swapped in, so the parked one stays live: its turn
//! streams, its approvals wait for it.
//!
//! In fullscreen, a side conversation is a pane beside the transcript
//! (`panes`), when the screen is wide enough; a subagent fills the screen.
//! In scrollback mode the interface switches to the second conversation,
//! and what the other commits waits until it is shown again. Esc (on an
//! empty composer, nothing running or waiting) returns to the main
//! conversation; each conversation's chat queues its own messages; the
//! panes' keymap actions, the leader Ctrl+X then a key by default, move
//! between them (`keys`). Codex's sidebar
//! of several conversations, its chooser and tab strip are not ported: a
//! session has at most one side conversation.

mod agents;
mod config;
mod keys;
mod panes;

use std::io::{self, Write};
use std::sync::Arc;
use std::time::Instant;

use hanafubuki_agent::AgentSessionId;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{KeyEvent, KeyEventKind};
use ratatui::text::Line;

use super::{new_view, App};
use crate::agent_bridge::{AgentBridge, AgentReply, OlderTranscript, OpenedSide};
use crate::agents_center::AgentsCenter;
use crate::app_event::{AppEvent, AppEventSender};
use crate::chatwidget::ChatWidget;
use crate::config_center::{ConfigCenter, ConfigSection};
use crate::history_cell::HistoryCell;
use crate::keymap::{self, Action};
use crate::transcript_view::{TranscriptHistoryState, TranscriptView};
use crate::tui::Tui;
pub(crate) use keys::PaneAction;
pub(super) use panes::{paint_chrome, PaneLayout};

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
    /// The focus moves to the other pane (a click in it).
    FocusOther,
    /// `/agents`.
    OpenAgents,
    /// `/config [section]`: the full-screen configuration center.
    OpenConfig {
        section: ConfigSection,
        direct: bool,
    },
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

    fn index(self) -> usize {
        match self {
            Self::Main => 0,
            Self::Second => 1,
        }
    }
}

/// What the second conversation is.
pub(super) enum SecondKind {
    /// The side conversation of `parent`.
    Side {
        parent: AgentSessionId,
        session: AgentSessionId,
    },
    /// A subagent (or another child) shown, its parent kept aside.
    Child {
        session: AgentSessionId,
        title: String,
        parent_title: String,
    },
}

/// The conversation the app's fields do not hold.
pub(super) struct Parked {
    pub(super) chat_widget: ChatWidget,
    pub(super) bridge: AgentBridge,
    pub(super) app_event_tx: AppEventSender,
    pub(super) transcript_cells: Vec<Arc<dyn HistoryCell>>,
    pub(super) transcript_view: TranscriptView,
    pub(super) older_transcript: Option<OlderTranscript>,
}

pub(super) struct Second {
    /// The number its events carry.
    id: u64,
    pub(super) kind: SecondKind,
    pub(super) parked: Parked,
    /// Which conversation the app's fields hold.
    pub(super) focus: Focus,
    /// Scrollback mode: what each conversation committed while hidden.
    held: [Vec<Line<'static>>; 2],
}

#[derive(Default)]
pub(super) struct Workspace {
    second: Option<Second>,
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
    config: Option<ConfigCenter>,
}

impl Workspace {
    pub(super) fn second_focused(&self) -> bool {
        self.second
            .as_ref()
            .is_some_and(|second| second.focus == Focus::Second)
    }

    pub(super) fn agents_open(&self) -> bool {
        self.agents.is_some()
    }

    /// An event of the parked conversation is being handled.
    pub(super) fn handling_parked(&self) -> bool {
        self.handling.is_some()
    }

    /// Scrollback mode: where a parked conversation's committed lines
    /// wait, while one of its events is handled.
    pub(super) fn held_lines(&mut self) -> Option<&mut Vec<Line<'static>>> {
        let who = self.handling?;
        Some(&mut self.second.as_mut()?.held[who.index()])
    }

    pub(super) fn parked_mut(&mut self) -> Option<&mut Parked> {
        self.second.as_mut().map(|second| &mut second.parked)
    }

    pub(super) fn is_dragging(&self) -> bool {
        self.dragging
    }

    #[cfg(test)]
    pub(super) fn last_layout(&self) -> Option<&PaneLayout> {
        self.layout.as_ref()
    }
}

/// Where an event goes.
pub(super) enum Route {
    /// To the conversation the app's fields hold, or the app itself.
    Here(AppEvent),
    Parked(AppEvent),
    Workspace(WorkspaceEvent),
    /// A closed conversation's.
    Dropped,
}

impl App {
    /// Where `event` goes: a conversation's own events to that
    /// conversation, the rest to the app.
    pub(super) fn route_event(&mut self, event: AppEvent) -> Route {
        let (event, owner) = match event {
            AppEvent::Second(id, inner) => {
                if self.workspace.second.as_ref().map(|second| second.id) != Some(id) {
                    return Route::Dropped;
                }
                (*inner, Focus::Second)
            }
            event => (event, Focus::Main),
        };
        let focus = self.workspace.second.as_ref().map(|second| second.focus);
        match event {
            AppEvent::Workspace(event) => Route::Workspace(event),
            // Both conversations hear of the Agent's state, and both may
            // stream.
            AppEvent::AgentStatusChanged | AppEvent::CommitTick if focus.is_some() => {
                self.on_parked(|app| match &event {
                    AppEvent::CommitTick => app.chat_widget.on_commit_tick(),
                    _ => {
                        if let Some(update) = app.bridge.status_changed() {
                            app.chat_widget.handle_turn_update(update);
                        }
                    }
                });
                Route::Here(event)
            }
            AppEvent::Agent(_)
            | AppEvent::AgentReply(_)
            | AppEvent::Op(_)
            | AppEvent::InsertHistoryCell(_)
            | AppEvent::CommandStep(_)
            | AppEvent::Command(_)
                if focus.is_some_and(|focus| focus != owner) =>
            {
                Route::Parked(event)
            }
            event => Route::Here(event),
        }
    }

    /// Handles the parked conversation's `event` with its fields swapped
    /// in; `false` when the app should exit.
    pub(super) fn handle_parked<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        event: AppEvent,
    ) -> bool {
        let Some(second) = self.workspace.second.as_ref() else {
            return true;
        };
        let parked = second.focus.other();
        self.swap_conversations();
        self.workspace.handling = Some(parked);
        let keep_running = self.handle_here(tui, event);
        self.workspace.handling = None;
        self.swap_conversations();
        keep_running
    }

    fn on_parked(&mut self, call: impl FnOnce(&mut Self)) {
        self.swap_conversations();
        call(self);
        self.swap_conversations();
    }

    /// Exchanges the app's conversation with the parked one.
    fn swap_conversations(&mut self) {
        let Some(second) = self.workspace.second.as_mut() else {
            return;
        };
        let parked = &mut second.parked;
        std::mem::swap(&mut self.chat_widget, &mut parked.chat_widget);
        std::mem::swap(&mut self.bridge, &mut parked.bridge);
        std::mem::swap(&mut self.app_event_tx, &mut parked.app_event_tx);
        std::mem::swap(&mut self.transcript_cells, &mut parked.transcript_cells);
        std::mem::swap(&mut self.transcript_view, &mut parked.transcript_view);
        std::mem::swap(&mut self.older_transcript, &mut parked.older_transcript);
        second.focus = second.focus.other();
    }

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
            WorkspaceEvent::FocusOther => {
                if let Some(focus) = self.workspace.second.as_ref().map(|second| second.focus) {
                    self.set_focus(tui, focus.other());
                }
            }
            WorkspaceEvent::OpenAgents => self.open_agents_center(tui),
            WorkspaceEvent::OpenConfig { section, direct } => {
                self.open_config_center(tui, section, direct)
            }
        }
        self.schedule_frame(Instant::now());
    }

    /// The session the main conversation shows.
    pub(super) fn main_session_id(&self) -> Option<AgentSessionId> {
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
        match self.workspace.second.as_ref().map(|second| &second.kind) {
            Some(SecondKind::Side { .. }) => {
                self.set_focus(tui, Focus::Second);
                if let Some(message) = message {
                    self.chat_widget.submit_user_message(message);
                }
                return;
            }
            Some(SecondKind::Child { .. }) => {
                return self
                    .chat_widget
                    .add_error_message("Return to the main conversation first (Esc).".to_string());
            }
            None => {}
        }
        if self.workspace.opening_side {
            return;
        }
        match self.bridge.open_side(message) {
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
        if self.workspace.second.is_some() || self.bridge.session_id() != Some(side.parent) {
            self.bridge.close_side(session);
            return;
        }
        self.install_second(
            tui,
            SecondKind::Side {
                parent: side.parent,
                session,
            },
        );
        if let Some(update) = self.bridge.reply(AgentReply::SessionOpened(side.session)) {
            self.show_update(update);
        }
        if let Some(message) = message {
            self.chat_widget.submit_user_message(message);
        }
    }

    fn open_child<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        child: AgentSessionId,
    ) {
        match self.workspace.second.as_ref().map(|second| &second.kind) {
            Some(SecondKind::Child { session, .. }) if *session == child => {
                return self.set_focus(tui, Focus::Second);
            }
            Some(SecondKind::Child { .. }) => self.leave_second(tui),
            Some(SecondKind::Side { .. }) => {
                self.set_focus(tui, Focus::Main);
                return self.chat_widget.add_error_message(
                    "Close the side conversation first: /side close.".to_string(),
                );
            }
            None => {}
        }
        let opened = match self.bridge.open_child(child) {
            Ok(opened) => opened,
            Err(error) => {
                return self
                    .chat_widget
                    .add_error_message(format!("The subagent was not shown: {error}"))
            }
        };
        self.install_second(
            tui,
            SecondKind::Child {
                session: child,
                title: opened.title.clone(),
                parent_title: opened.parent_title.clone(),
            },
        );
        for update in self.bridge.show_child(opened) {
            self.show_update(update);
        }
    }

    /// A second conversation of `kind`, with a chat and a bridge of its
    /// own, focused; the main one is parked.
    fn install_second<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        kind: SecondKind,
    ) {
        self.workspace.next_id += 1;
        let id = self.workspace.next_id;
        let app_event_tx = self.app_event_tx.for_second(id);
        let parked = Parked {
            chat_widget: self.chat_widget.second(app_event_tx.clone()),
            bridge: self.bridge.companion(app_event_tx.clone()),
            app_event_tx,
            transcript_cells: Vec::new(),
            transcript_view: new_view(self.render_mode, TranscriptHistoryState::Complete),
            older_transcript: None,
        };
        self.workspace.zoomed = false;
        self.workspace.second = Some(Second {
            id,
            kind,
            parked,
            focus: Focus::Main,
            held: Default::default(),
        });
        self.set_focus(tui, Focus::Second);
    }

    /// Gives `focus` the keys: in scrollback mode the interface switches to
    /// it, with what it committed meanwhile.
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
        self.reset_backtrack();
        self.swap_conversations();
        if !self.mode.is_owned() {
            let banner = self.workspace.banner(focus);
            let held = self
                .workspace
                .second
                .as_mut()
                .map(|second| std::mem::take(&mut second.held[focus.index()]))
                .unwrap_or_default();
            let mut lines = vec![Line::from(""), banner];
            lines.extend(held);
            if self.alt_screen_open() {
                self.deferred_history_lines.extend(lines);
            } else {
                tui.insert_history_lines(lines);
            }
        }
        self.schedule_frame(Instant::now());
    }

    /// Esc or `focus_pane_left`: the main conversation again. A side conversation
    /// stays open; a subagent shown is let go.
    fn return_to_main<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        match self.workspace.second.as_ref().map(|second| &second.kind) {
            Some(SecondKind::Side { .. }) => self.set_focus(tui, Focus::Main),
            Some(SecondKind::Child { .. }) => self.leave_second(tui),
            None => {}
        }
    }

    /// `/side close` and `hide_pane`: the side conversation is closed (its
    /// turn interrupted, the session dropped), a subagent shown let go.
    fn close_second<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        let side = match self.workspace.second.as_ref().map(|second| &second.kind) {
            Some(SecondKind::Side { session, .. }) => Some(*session),
            Some(SecondKind::Child { .. }) => None,
            None => {
                return self
                    .chat_widget
                    .add_info_message("No side conversation is open.".to_string())
            }
        };
        self.leave_second(tui);
        if let Some(side) = side {
            self.bridge.close_side(side);
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
            kind: SecondKind::Side { parent, session },
            ..
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
        if let Some(action) = PaneAction::of(&key) {
            self.pane_action(tui, action);
            return true;
        }
        let returns = self.workspace.second_focused()
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
            PaneAction::ToggleFocus if focus == Some(Focus::Second) => self.return_to_main(tui),
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
