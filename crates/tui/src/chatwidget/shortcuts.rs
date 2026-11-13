// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The session's keys (Codex's `chatwidget/reasoning_shortcuts.rs` and its
//! mode cycle): Shift+Tab switches between the default and the plan mode
//! as `/mode` does, and Alt+, and Alt+. step the reasoning effort a level
//! through what `/effort` offers. Each is read while the composer has the
//! keys, where its command runs, and is refused during a task as its
//! command is.

use hanafubuki_agent::AgentSessionMode;
use ratatui::crossterm::event::KeyEvent;

use super::ChatWidget;
use crate::agent_bridge::EffortCall;
use crate::app_event::{AgentCommand, AppEvent, Op};
use crate::commands::{self, effort};
use crate::keymap::{self, Action};

impl ChatWidget {
    /// Takes `key` when it is one of the session's keys.
    pub(super) fn handle_session_key(&mut self, key: &KeyEvent) -> bool {
        if !self.bottom_pane.no_modal_or_popup_active() {
            return false;
        }
        if keymap::matches(Action::CycleMode, key) && self.bottom_pane.command_allowed("mode") {
            self.cycle_mode();
            return true;
        }
        let raise = if keymap::matches(Action::ReasoningUp, key) {
            true
        } else if keymap::matches(Action::ReasoningDown, key) {
            false
        } else {
            return false;
        };
        if !self.bottom_pane.command_allowed("effort") {
            return false;
        }
        self.step_effort(raise);
        true
    }

    fn cycle_mode(&mut self) {
        if self.refuses_during_task(&commands::MODE) {
            return;
        }
        let next = match self.mode {
            AgentSessionMode::Default => AgentSessionMode::Plan,
            AgentSessionMode::Plan => AgentSessionMode::Default,
        };
        self.app_event_tx.send(AppEvent::Op(Op::EnsureSession));
        self.app_event_tx
            .send(AppEvent::Op(Op::Command(AgentCommand::SetMode(next))));
    }

    fn step_effort(&mut self, raise: bool) {
        if self.bottom_pane.is_task_running() {
            return self.add_error_message(effort::DURING_TASK.to_string());
        }
        self.app_event_tx.send(AppEvent::Op(Op::EnsureSession));
        self.app_event_tx
            .send(AppEvent::Op(Op::Command(AgentCommand::Effort(
                EffortCall::Step { raise },
            ))));
    }
}
