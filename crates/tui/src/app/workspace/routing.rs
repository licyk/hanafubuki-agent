// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Route events to the focused, side, or parked parent conversation.

use std::io::{self, Write};

use ratatui::backend::Backend;

use super::{Focus, Parked, WorkspaceEvent};
use crate::app::App;
use crate::app_event::AppEvent;
use crate::tui::Tui;

/// Where an event goes.
pub(in crate::app) enum Route {
    /// To the conversation the app's fields hold, or the app itself.
    Here(AppEvent),
    Parked(AppEvent),
    Parent(AppEvent),
    Workspace(WorkspaceEvent),
    /// A closed conversation's.
    Dropped,
}

impl App {
    /// Where `event` goes: a conversation's own events to that
    /// conversation, the rest to the app.
    pub(in crate::app) fn route_event(&mut self, event: AppEvent) -> Route {
        let (event, owner) = match event {
            AppEvent::Second(id, inner) => {
                if self.workspace.second.as_ref().map(|second| second.id) == Some(id) {
                    (*inner, Some(Focus::Second))
                } else if self.workspace.child.as_ref().map(|child| child.id) == Some(id) {
                    (*inner, Some(Focus::Main))
                } else {
                    return Route::Dropped;
                }
            }
            event => (event, self.workspace.child.is_none().then_some(Focus::Main)),
        };
        let focus = self.workspace.second.as_ref().map(|second| second.focus);
        match event {
            AppEvent::Workspace(event) => Route::Workspace(event),
            AppEvent::Exit if owner == Some(Focus::Second) => {
                Route::Workspace(WorkspaceEvent::CloseSide)
            }
            AppEvent::Exit if owner == Some(Focus::Main) && self.workspace.child.is_some() => {
                Route::Workspace(WorkspaceEvent::OpenChild(self.main_session_id().unwrap()))
            }
            // All conversations hear of the Agent's state and keep streaming.
            AppEvent::AgentBackground(hanafubuki_agent::AgentBackgroundEvent::Changed { root }) => {
                if self.workspace.second.is_some() {
                    self.on_parked(|app| app.bridge.tree_changed(root));
                }
                if self.workspace.child.is_some() {
                    self.swap_parent();
                    self.bridge.tree_changed(root);
                    self.swap_parent();
                }
                Route::Here(event)
            }
            AppEvent::AgentStatusChanged | AppEvent::CommitTick => {
                if self.workspace.second.is_some() {
                    self.on_parked(|app| app.conversation_tick(&event));
                }
                if self.workspace.child.is_some() {
                    self.swap_parent();
                    self.conversation_tick(&event);
                    self.swap_parent();
                }
                Route::Here(event)
            }
            AppEvent::Agent(_)
            | AppEvent::AgentReply(_)
            | AppEvent::AgentBackground(_)
            | AppEvent::Op(_)
            | AppEvent::InsertHistoryCell(_)
            | AppEvent::InsertLiveEntry(_)
            | AppEvent::LiveEntryCompleted(_)
            | AppEvent::StreamUpdate(_)
            | AppEvent::ConsolidateStream(_)
            | AppEvent::CommandStep(_)
            | AppEvent::Command(_)
            | AppEvent::RecentMessagesRead(_)
                if owner.is_none() || focus.is_some_and(|focus| Some(focus) != owner) =>
            {
                if owner.is_none() {
                    Route::Parent(event)
                } else {
                    Route::Parked(event)
                }
            }
            // The editor edits, and an image goes into, the focused
            // composer's draft.
            AppEvent::ExternalEditor | AppEvent::PasteImage
                if owner.is_none() || focus.is_some_and(|focus| Some(focus) != owner) =>
            {
                Route::Dropped
            }
            event => Route::Here(event),
        }
    }

    /// Handles the parked conversation's `event` with its fields swapped
    /// in; `false` when the app should exit.
    pub(in crate::app) fn handle_parked<B: Backend<Error = io::Error> + Write>(
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

    fn conversation_tick(&mut self, event: &AppEvent) {
        match event {
            AppEvent::CommitTick => self.chat_widget.on_commit_tick(),
            _ => {
                if let Some(update) = self.bridge.status_changed() {
                    self.chat_widget.handle_turn_update(update);
                }
            }
        }
    }

    pub(in crate::app) fn handle_parent<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        event: AppEvent,
    ) -> bool {
        self.swap_parent();
        self.workspace.handling = Some(Focus::Main);
        let keep_running = self.handle_here(tui, event);
        self.workspace.handling = None;
        self.swap_parent();
        keep_running
    }

    fn swap_parent(&mut self) {
        let Some(mut child) = self.workspace.child.take() else {
            return;
        };
        self.swap_parked(&mut child.parked);
        self.workspace.child = Some(child);
    }

    pub(super) fn swap_conversations(&mut self) {
        let Some(mut second) = self.workspace.second.take() else {
            return;
        };
        self.swap_parked(&mut second.parked);
        second.focus = second.focus.other();
        self.workspace.second = Some(second);
    }

    pub(super) fn swap_parked(&mut self, parked: &mut Parked) {
        std::mem::swap(&mut self.chat_widget, &mut parked.chat_widget);
        std::mem::swap(&mut self.bridge, &mut parked.bridge);
        std::mem::swap(&mut self.app_event_tx, &mut parked.app_event_tx);
        std::mem::swap(&mut self.transcript_cells, &mut parked.transcript_cells);
        std::mem::swap(&mut self.transcript_view, &mut parked.transcript_view);
        std::mem::swap(&mut self.tail, &mut parked.tail);
        std::mem::swap(&mut self.older_transcript, &mut parked.older_transcript);
        parked.chat_widget.set_input_focus(false);
        self.chat_widget.set_input_focus(true);
    }
}
