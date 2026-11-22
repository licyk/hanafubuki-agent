// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The selected agent occupies the main pane; its parent stays live while parked.

use std::io::{self, Write};
use std::time::Instant;

use hanafubuki_agent::AgentSessionId;
use ratatui::backend::Backend;
use ratatui::text::Line;

use super::{Focus, Parked};
use crate::agent_bridge::AgentBridge;
use crate::app::App;
use crate::tui::Tui;

pub(super) struct Child {
    pub(super) id: u64,
    pub(super) session: AgentSessionId,
    pub(super) title: String,
    pub(super) parent_title: String,
    pub(super) parked: Parked,
}

impl App {
    pub(super) fn main_bridge(&self) -> &AgentBridge {
        if let Some(parent) = self.workspace.parent() {
            &parent.bridge
        } else if let Some(second) = self
            .workspace
            .second
            .as_ref()
            .filter(|second| second.focus == Focus::Second)
        {
            &second.parked.bridge
        } else {
            &self.bridge
        }
    }

    pub(super) fn open_child<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        child: AgentSessionId,
    ) {
        self.set_focus(tui, Focus::Main);
        if self.main_session_id() == Some(child) {
            return self.leave_child(tui);
        }
        if self
            .workspace
            .child
            .as_ref()
            .is_some_and(|shown| shown.session == child)
        {
            return;
        }
        let opened = match self.main_bridge().open_child(child) {
            Ok(opened) => opened,
            Err(error) => {
                return self
                    .chat_widget
                    .add_error_message(format!("The subagent was not shown: {error}"))
            }
        };
        self.leave_child(tui);
        self.flush_before_switch(tui);
        let (id, mut parked) = self.new_parked();
        self.swap_parked(&mut parked);
        self.workspace.child = Some(Child {
            id,
            session: child,
            title: opened.title.clone(),
            parent_title: opened.parent_title.clone(),
            parked,
        });
        self.show_focus_banner(tui, Focus::Main);
        for update in self.bridge.show_child(opened) {
            self.show_update(update);
        }
    }

    pub(super) fn leave_child<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        self.set_focus(tui, Focus::Main);
        if self.workspace.child.is_none() {
            return;
        }
        self.flush_before_switch(tui);
        let mut child = self.workspace.child.take().unwrap();
        self.swap_parked(&mut child.parked);
        self.show_focus_banner(tui, Focus::Main);
    }

    pub(super) fn flush_before_switch<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) {
        self.reset_backtrack();
        if !self.using_owned_view() {
            self.tail.flush_all();
            let width = tui.terminal.last_known_screen_size.width;
            let lines = self
                .tail
                .take(&mut self.transcript_view, width, Some(u16::MAX))
                .0;
            if self.alt_screen_open() {
                self.deferred_history_lines.extend(lines);
            } else {
                tui.insert_history_lines(lines);
            }
        }
    }

    pub(super) fn show_focus_banner<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        focus: Focus,
    ) {
        if !self.using_owned_view() {
            let lines = vec![Line::from("").into(), self.workspace.banner(focus).into()];
            if self.alt_screen_open() {
                self.deferred_history_lines.extend(lines);
            } else {
                tui.insert_history_lines(lines);
            }
        }
        self.schedule_frame(Instant::now());
    }
}
