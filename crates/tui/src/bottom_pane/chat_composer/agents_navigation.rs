// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Left on an empty composer opens the command center, and the footer says
//! so (`← for agents`), as in Codex (`chat_composer/agents_navigation.rs`):
//! only while the composer has the keys and is empty, no popup is open and
//! no paste burst runs, `/agents` runs here, and Left is still the editor's
//! `move_left`. The keymap refuses a key or chord prefix that two actions
//! beside the composer would share, so Left kept by `move_left` is no other
//! action's there.
//!
//! In the same state Alt+Left and Alt+Right (`previous_agent`,
//! `next_agent`, Codex's `multi_agents.rs` shortcuts) show the previous or
//! next of the main session and its subagents; with text they are the
//! editor's word motion, which shares their keys.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::ChatComposer;
use crate::app::WorkspaceEvent;
use crate::app_event::AppEvent;
use crate::keymap::{self, Action};

impl ChatComposer {
    /// Whether plain Left opens the command center now.
    pub(crate) fn agents_navigation_available(&self) -> bool {
        self.empty_and_idle()
            && self.command_allowed("agents")
            && keymap::matches(Action::MoveLeft, &plain_left())
    }

    /// Takes plain Left when it opens the command center, and the keys
    /// that step through the agents on an empty composer.
    pub(super) fn handle_agents_navigation(&mut self, key: &KeyEvent) -> bool {
        let opens = key.kind == KeyEventKind::Press
            && key.code == KeyCode::Left
            && key.modifiers == KeyModifiers::NONE
            && self.agents_navigation_available();
        if opens {
            self.app_event_tx
                .send(AppEvent::Workspace(WorkspaceEvent::OpenAgents));
            return true;
        }
        let forward = if keymap::matches(Action::NextAgent, key) {
            true
        } else if keymap::matches(Action::PreviousAgent, key) {
            false
        } else {
            return false;
        };
        let steps = self.empty_and_idle();
        if steps {
            self.app_event_tx
                .send(AppEvent::Workspace(WorkspaceEvent::StepAgent { forward }));
        }
        steps
    }

    /// The composer has the keys, holds no text, shows no popup and buffers
    /// no paste.
    fn empty_and_idle(&self) -> bool {
        self.has_focus && self.is_empty() && !self.is_in_paste_burst() && !self.popup_active()
    }
}

fn plain_left() -> KeyEvent {
    KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)
}

#[cfg(test)]
mod tests;
