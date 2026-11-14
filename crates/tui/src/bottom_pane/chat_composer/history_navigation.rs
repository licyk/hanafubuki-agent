// Modified from OpenAI Codex 57a808a392 (Apache-2.0).
// Use the Agent's bounded history read instead of Codex's per-entry lookups.

use ratatui::crossterm::event::{KeyEvent, KeyEventKind};

use super::ChatComposer;
use crate::app_event::AppEvent;
use crate::keymap::{self, Action};

impl ChatComposer {
    pub(super) fn navigate_history(&mut self, key: &KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release
            || self.is_in_paste_burst()
            || !self
                .history
                .should_handle_navigation(self.textarea.text(), self.textarea.cursor())
        {
            return false;
        }
        let entry = if keymap::matches(Action::HistoryPrevious, key) {
            if self.history.request_stored_read() {
                self.app_event_tx.send(AppEvent::ReadRecentMessages);
            }
            self.history.navigate_up()
        } else {
            self.history.navigate_down()
        };
        if let Some(entry) = entry {
            self.set_message(entry);
        }
        // At the oldest/newest boundary retain the draft and cursor.
        true
    }

    pub(crate) fn on_recent_messages(&mut self, messages: Vec<String>) {
        if self.is_in_paste_burst()
            || !self
                .history
                .should_handle_navigation(self.textarea.text(), self.textarea.cursor())
        {
            self.history.cancel_pending_navigation();
        }
        if let Some(result) = self.history.on_stored_read(messages) {
            self.apply_history_search_result(result);
        }
        if let Some(entry) = self.history.resume_navigation() {
            self.set_message(entry);
        }
    }
}

#[cfg(test)]
mod tests;
