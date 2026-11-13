// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Ctrl+G (`composer.external_editor`) in the composer, as in Codex
//! (`App::launch_external_editor`'s gate, `ChatComposer::apply_external_edit`).
//! The key asks the app for the editor while the composer has the keys and
//! no popup or history search is open; otherwise it does nothing. The
//! editor gets the draft with its large pastes written out, and while it
//! runs the footer says how to come back. Its text becomes the draft with
//! the trailing blanks dropped, and a large paste that comes back
//! unchanged is its placeholder again; one edited stays text. An image's
//! label comes back as its placeholder wherever it is; an image whose label
//! is gone is dropped, and the rest are numbered again in their order.

use std::ops::Range;

use ratatui::crossterm::event::{KeyEvent, KeyEventKind};
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::ChatComposer;
use crate::app_event::AppEvent;
use crate::keymap::{self, Action};
use crate::user_message::{image_label, MessageImage};

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
        let mut pastes = std::mem::take(&mut self.pending_pastes);
        let mut images: Vec<Option<MessageImage>> = std::mem::take(&mut self.images)
            .into_iter()
            .map(Some)
            .collect();
        // Each placeholder in the draft's order, with what the editor got
        // for it: a paste's text, an image's label.
        let held: Vec<Held> = self
            .textarea
            .element_payloads()
            .into_iter()
            .filter_map(|placeholder| {
                if let Some(index) = pastes.iter().position(|(held, _)| *held == placeholder) {
                    let (placeholder, actual) = pastes.remove(index);
                    return Some(Held::Paste(placeholder, actual));
                }
                (1..=images.len())
                    .find(|number| image_label(*number) == placeholder)
                    .map(|number| Held::Image(number, placeholder))
            })
            .collect();
        // Each found where it first appears, apart from the others.
        let mut found: Vec<(Range<usize>, Held)> = Vec::new();
        for held in held {
            let wanted = held.text();
            let at = text
                .match_indices(wanted)
                .map(|(start, _)| start)
                .find(|start| {
                    let range = *start..start + wanted.len();
                    !found
                        .iter()
                        .any(|(other, _)| other.start < range.end && range.start < other.end)
                });
            if let Some(start) = at {
                found.push((start..start + wanted.len(), held));
            }
        }
        found.sort_by_key(|(range, _)| range.start);
        // The images kept, numbered again in their order.
        let kept: Vec<usize> = (1..=images.len())
            .filter(|number| {
                found
                    .iter()
                    .any(|(_, held)| matches!(held, Held::Image(kept, _) if kept == number))
            })
            .collect();
        for (index, image) in images.iter_mut().enumerate() {
            if !kept.contains(&(index + 1)) {
                if let Some(image) = image.take() {
                    image.remove_pasted_file();
                }
            }
        }
        self.textarea.set_text_clearing_elements("");
        let mut copied = 0;
        for (range, held) in found {
            self.textarea.insert_str(&text[copied..range.start]);
            copied = range.end;
            match held {
                Held::Paste(placeholder, actual) => {
                    self.textarea.insert_element(&placeholder);
                    self.pending_pastes.push((placeholder, actual));
                }
                Held::Image(number, _) => {
                    let new = kept.iter().position(|kept| *kept == number).unwrap_or(0) + 1;
                    self.textarea.insert_element(&image_label(new));
                }
            }
        }
        // Trimmed after the last placeholder: a paste may end in blanks.
        self.textarea.insert_str(text[copied..].trim_end());
        self.images = images.into_iter().flatten().collect();
        self.textarea.set_cursor(self.textarea.text().len());
        self.sync_popups();
    }
}

/// A placeholder of the draft the editor had.
enum Held {
    /// A large paste: its placeholder and its text.
    Paste(String, String),
    /// An image: its number and its label.
    Image(usize, String),
}

impl Held {
    /// What the editor got for it.
    fn text(&self) -> &str {
        match self {
            Held::Paste(_, actual) => actual,
            Held::Image(_, label) => label,
        }
    }
}

#[cfg(test)]
mod tests;
