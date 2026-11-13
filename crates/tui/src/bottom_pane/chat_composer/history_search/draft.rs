// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The draft while a search's match shows in its place (Codex's
//! `history_search_draft.rs`): its text area (text, cursor, placeholders)
//! and the pastes and images they stand for, put aside whole and put back
//! on Esc. A message given back to the composer meanwhile (a turn
//! interrupted with messages waiting) goes into the draft put aside, so Esc
//! never loses it; taking the match drops the draft, as in Codex.

use super::ChatComposer;
use crate::bottom_pane::textarea::TextArea;
use crate::user_message::{MessageImage, UserMessage};

pub(super) struct SavedDraft {
    textarea: TextArea,
    pending_pastes: Vec<(String, String)>,
    images: Vec<MessageImage>,
}

impl ChatComposer {
    /// Shows `entry` in the draft's place, with its images, what matched
    /// `query` highlighted; the draft is put aside unless it already is.
    pub(super) fn show_match(&mut self, entry: &UserMessage, query: &str) {
        let shown = self.take_draft(TextArea::new());
        if let Some(search) = self.history_search.as_mut() {
            search.saved.get_or_insert(shown);
        }
        self.set_message(entry.clone());
        let highlights = super::match_ranges(self.textarea.text(), query);
        self.textarea.set_highlights(highlights);
    }

    /// The draft back in place of a match shown.
    pub(super) fn show_saved_draft(&mut self) {
        if let Some(saved) = self
            .history_search
            .as_mut()
            .and_then(|search| search.saved.take())
        {
            self.put_draft(saved);
        }
    }

    /// Runs `edit` on the draft, whether it shows or is put aside behind a
    /// match.
    pub(in crate::bottom_pane::chat_composer) fn edit_draft(
        &mut self,
        edit: impl FnOnce(&mut Self),
    ) {
        let Some(saved) = self
            .history_search
            .as_mut()
            .and_then(|search| search.saved.take())
        else {
            return edit(self);
        };
        let preview = self.take_draft(saved.textarea);
        self.pending_pastes = saved.pending_pastes;
        self.images = saved.images;
        edit(self);
        let saved = self.take_draft(preview.textarea);
        self.pending_pastes = preview.pending_pastes;
        self.images = preview.images;
        if let Some(search) = self.history_search.as_mut() {
            search.saved = Some(saved);
        }
    }

    /// The draft shown, `textarea` in its place, its pastes and images
    /// taken with it.
    fn take_draft(&mut self, textarea: TextArea) -> SavedDraft {
        SavedDraft {
            textarea: std::mem::replace(&mut self.textarea, textarea),
            pending_pastes: std::mem::take(&mut self.pending_pastes),
            images: std::mem::take(&mut self.images),
        }
    }

    fn put_draft(&mut self, draft: SavedDraft) {
        self.textarea = draft.textarea;
        self.pending_pastes = draft.pending_pastes;
        self.images = draft.images;
    }
}
