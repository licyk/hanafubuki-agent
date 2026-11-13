// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Ctrl+G (`composer.external_editor`) in the composer, as in Codex
//! (`App::launch_external_editor`'s gate, `ChatComposer::apply_external_edit`).
//! The key asks the app for the editor while the composer has the keys and
//! no popup or history search is open; otherwise it does nothing. The
//! editor gets the draft with its large pastes written out, and while it
//! runs the footer says how to come back. Its text becomes the draft with
//! the trailing blanks dropped, and a large paste that comes back
//! unchanged is its placeholder again; one edited stays text.

use ratatui::crossterm::event::{KeyEvent, KeyEventKind};
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::ChatComposer;
use crate::app_event::AppEvent;
use crate::keymap::{self, Action};

const EDITING: &str = "Save and close external editor to continue.";

impl ChatComposer {
    /// Takes the editor's key, whether or not it opens the editor now.
    pub(super) fn handle_external_editor_key(&mut self, key: &KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press || !keymap::matches(Action::ExternalEditor, key) {
            return false;
        }
        // What a burst still holds is part of the draft.
        if let Some(pasted) = self.paste_burst.flush_before_modified_input() {
            self.handle_paste(pasted);
        }
        if self.has_focus && !self.popup_active() {
            self.app_event_tx.send(AppEvent::ExternalEditor);
        }
        true
    }

    pub(crate) fn set_external_editor_open(&mut self, open: bool) {
        self.external_editor_open = open;
    }

    /// The footer's row while the editor has the draft.
    pub(super) fn external_editor_line(&self) -> Option<Line<'static>> {
        self.external_editor_open.then(|| Line::from(EDITING).dim())
    }

    pub(crate) fn apply_external_edit(&mut self, text: String) {
        let text = text.replace("\r\n", "\n");
        // Trimmed after the last paste found: a paste may end in blanks.
        let mut rest = text.as_str();
        let mut pastes = std::mem::take(&mut self.pending_pastes);
        // In the draft's order, each looked for after the one before.
        let in_order: Vec<(String, String)> = self
            .textarea
            .element_payloads()
            .into_iter()
            .filter_map(|placeholder| {
                let index = pastes.iter().position(|(held, _)| *held == placeholder)?;
                Some(pastes.remove(index))
            })
            .collect();
        self.textarea.set_text_clearing_elements("");
        for (placeholder, actual) in in_order {
            let Some(start) = rest.find(actual.as_str()) else {
                continue;
            };
            self.textarea.insert_str(&rest[..start]);
            self.textarea.insert_element(&placeholder);
            rest = &rest[start + actual.len()..];
            self.pending_pastes.push((placeholder, actual));
        }
        self.textarea.insert_str(rest.trim_end());
        self.textarea.set_cursor(self.textarea.text().len());
        self.sync_popups();
    }
}

#[cfg(test)]
mod tests;
