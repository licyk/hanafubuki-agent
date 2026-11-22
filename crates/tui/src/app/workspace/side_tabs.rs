// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

//! Sidebar chat and agent tabs keep their own bridge, input and transcript.

use std::io::{self, Write};

use hanafubuki_agent::AgentSessionId;
use ratatui::backend::Backend;

use super::{App, Focus, Second};
use crate::agent_bridge::{AgentReply, SidebarSession};
use crate::tui::Tui;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TabKind {
    Temporary,
    Saved,
    Agent,
}

impl TabKind {
    pub(super) fn is_chat(self) -> bool {
        self != Self::Agent
    }
}

impl App {
    fn side_tabs(&self) -> impl Iterator<Item = &Second> {
        self.workspace.second.iter().chain(&self.workspace.hidden)
    }

    pub(in crate::app) fn sidebar_chats(&self) -> Vec<(AgentSessionId, String)> {
        let mut tabs = self
            .side_tabs()
            .filter(|tab| tab.kind.is_chat())
            .map(|tab| (tab.session, tab.title.clone()))
            .collect::<Vec<_>>();
        tabs.sort_by_key(|(id, _)| *id);
        tabs
    }

    pub(in crate::app) fn active_sidebar_agent(&self) -> Option<AgentSessionId> {
        self.workspace
            .second
            .as_ref()
            .filter(|tab| !tab.kind.is_chat())
            .map(|tab| tab.session)
    }

    pub(in crate::app) fn active_sidebar_chat(&self) -> Option<AgentSessionId> {
        self.workspace
            .second
            .as_ref()
            .filter(|tab| tab.kind.is_chat())
            .map(|tab| tab.session)
    }

    pub(in crate::app) fn update_sidebar_agents(
        &mut self,
        update: &crate::agent_bridge::TurnUpdate,
    ) {
        if let crate::agent_bridge::TurnUpdate::Subagents(
            crate::agent_bridge::SubagentsReply::Changed(agents),
        ) = update
        {
            if self.bridge.session_id() == self.workspace.shown {
                self.workspace.sidebar_agents = agents
                    .iter()
                    .skip(1)
                    .map(|agent| {
                        (
                            agent.session_id,
                            format!("{} · {:?}", agent.title, agent.status),
                        )
                    })
                    .collect();
                self.workspace.sidebar.chooser_dirty = true;
            }
        }
    }

    pub(in crate::app) fn request_sidebar_sessions(&self) {
        if let Some(parent) = self.main_session_id() {
            self.main_bridge().list_sidebar_sessions(parent);
        }
    }

    pub(in crate::app) fn hide_side_chat<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) {
        self.workspace.reading_tab = None;
        self.set_focus(tui, Focus::Main);
        if let Some(second) = self.workspace.second.take() {
            // Agent journals can be reopened; only a conversation owns a draft and live sink.
            if second.kind.is_chat() {
                self.workspace.hidden.push(second);
            }
            self.workspace.dragging = false;
            self.workspace.layout = None;
        }
    }

    pub(in crate::app) fn new_side_chat<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        saved: bool,
    ) {
        if self.workspace.opening_side {
            return;
        }
        match self.main_bridge().open_side(None, saved) {
            Ok(()) => self.workspace.opening_side = true,
            Err(error) => self.side_open_error(tui, error),
        }
    }

    pub(in crate::app) fn select_side_chat<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        id: AgentSessionId,
    ) {
        self.workspace.reading_tab = None;
        if self.active_sidebar_chat() == Some(id) {
            self.set_focus(tui, Focus::Second);
            return;
        }
        if let Some(index) = self
            .workspace
            .hidden
            .iter()
            .position(|tab| tab.session == id)
        {
            let tab = self.workspace.hidden.remove(index);
            self.hide_side_chat(tui);
            self.workspace.second = Some(tab);
            self.set_focus(tui, Focus::Second);
            return;
        }
        let Some(parent) = self.main_session_id() else {
            return;
        };
        self.workspace.reading_tab = Some(id);
        self.main_bridge().read_sidebar_session(parent, id, false);
    }

    pub(in crate::app) fn dock_subagent<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        id: AgentSessionId,
    ) {
        if self.active_sidebar_agent() == Some(id) {
            self.set_focus(tui, Focus::Second);
            return;
        }
        let Some(parent) = self.main_session_id() else {
            return;
        };
        if !self
            .workspace
            .sidebar_agents
            .iter()
            .any(|(agent, _)| *agent == id)
        {
            return;
        }
        self.workspace.reading_tab = Some(id);
        self.main_bridge().read_sidebar_session(parent, id, true);
    }

    pub(super) fn sidebar_session_read<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        parent: AgentSessionId,
        id: AgentSessionId,
        read: Result<SidebarSession, String>,
    ) {
        if self.workspace.reading_tab != Some(id) || self.main_session_id() != Some(parent) {
            return;
        }
        self.workspace.reading_tab = None;
        match read {
            Ok(SidebarSession::Saved(session, title)) => {
                self.install_second(tui, parent, id, TabKind::Saved, title);
                if let Some(update) = self.bridge.reply(AgentReply::SessionOpened(*session)) {
                    self.show_update(update);
                }
            }
            Ok(SidebarSession::Agent(opened)) => {
                self.install_second(tui, parent, id, TabKind::Agent, opened.title.clone());
                self.bridge.watch_only = true;
                for update in self.bridge.show_child(*opened) {
                    self.show_update(update);
                }
            }
            Err(error) => self.side_open_error(tui, error),
        }
    }

    pub(super) fn side_open_error<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        error: String,
    ) {
        if self.workspace.second.is_none() && self.workspace.sidebar.uses_fullscreen() {
            self.open_sidebar(tui);
            self.workspace.sidebar.notice = Some(error);
        } else {
            self.chat_widget.add_error_message(error);
        }
    }

    pub(in crate::app) fn side_close_losses(
        &self,
        id: AgentSessionId,
        archive: bool,
    ) -> Vec<&'static str> {
        let mut losses = Vec::new();
        if let Some(tab) = self
            .side_tabs()
            .find(|tab| tab.session == id && tab.kind.is_chat())
        {
            let chat = if tab.focus == Focus::Second {
                &self.chat_widget
            } else {
                &tab.parked.chat_widget
            };
            if chat.is_task_running() {
                losses.push("stop the running turn");
            }
            if !chat.composer_is_empty() || chat.has_pending_input() {
                losses.push("discard unsent input");
            }
            if tab.kind == TabKind::Temporary {
                losses.push("lose this temporary chat’s history");
            }
        }
        if archive {
            losses.push("remove this chat from Side chats");
        }
        losses
    }

    pub(in crate::app) fn close_side_tab<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        id: AgentSessionId,
        archive: bool,
    ) {
        if self.active_sidebar_agent() == Some(id) {
            self.leave_second(tui);
            return;
        }
        let kind = self
            .side_tabs()
            .find(|tab| tab.session == id)
            .map(|tab| tab.kind);
        let attached = kind == Some(TabKind::Saved)
            || self
                .workspace
                .saved_chats
                .iter()
                .any(|(saved, _)| *saved == id);
        if kind.is_none() && !attached {
            return;
        }
        if self.workspace.closing_tabs.insert(id) {
            self.main_bridge().close_side(id, archive && attached);
        }
    }

    pub(super) fn side_tab_closed<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        id: AgentSessionId,
        archive: bool,
        result: Result<(), String>,
    ) {
        self.workspace.closing_tabs.remove(&id);
        if let Err(error) = result {
            if self.workspace.sidebar.closing.remove(&id) {
                self.workspace.sidebar.notice = Some(error.clone());
            }
            self.chat_widget.add_error_message(error);
            return;
        }
        let visible_here = self.side_tabs().any(|tab| tab.session == id)
            || self
                .workspace
                .saved_chats
                .iter()
                .any(|(saved, _)| *saved == id);
        if !visible_here {
            return;
        }
        if self.active_sidebar_chat() == Some(id) {
            self.leave_second(tui);
        }
        self.workspace.hidden.retain(|tab| tab.session != id);
        if archive {
            self.workspace.saved_chats.retain(|(saved, _)| *saved != id);
        }
        self.request_sidebar_sessions();
        self.chat_widget
            .add_info_message("Side conversation closed.".into());
        self.sidebar_tab_closed(tui, id);
    }
}
