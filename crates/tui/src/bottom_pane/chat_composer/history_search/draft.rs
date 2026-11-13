// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The draft while a search's match shows in its place (Codex's
//! `history_search_draft.rs`): its text area (text, cursor, placeholders)
//! and the pastes they stand for, put aside whole and put back on Esc. A
//! message given back to the composer meanwhile (a turn interrupted with
//! messages waiting) goes into the draft put aside, so Esc never loses it;
//! taking the match drops the draft, as in Codex.

use std::ops::Range;

use super::ChatComposer;
use crate::bottom_pane::textarea::TextArea;

pub(super) struct SavedDraft {
    textarea: TextArea,
    pending_pastes: Vec<(String, String)>,
}

impl ChatComposer {
    /// Shows `entry` in the draft's place, `highlights` highlighted; the
    /// draft is put aside unless it already is.
    pub(super) fn show_match(&mut self, entry: &str, highlights: Vec<Range<usize>>) {
        let mut preview = TextArea::new();
        preview.set_text_clearing_elements(entry);
        preview.set_cursor(entry.len());
        preview.set_highlights(highlights);
        let shown = SavedDraft {
            textarea: std::mem::replace(&mut self.textarea, preview),
            pending_pastes: std::mem::take(&mut self.pending_pastes),
        };
        if let Some(search) = self.history_search.as_mut() {
            search.saved.get_or_insert(shown);
        }
    }

    /// The draft back in place of a match shown.
    pub(super) fn show_saved_draft(&mut self) {
        if let Some(saved) = self
            .history_search
            .as_mut()
            .and_then(|search| search.saved.take())
        {
            self.textarea = saved.textarea;
            self.pending_pastes = saved.pending_pastes;
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
        let preview = SavedDraft {
            textarea: std::mem::replace(&mut self.textarea, saved.textarea),
            pending_pastes: std::mem::replace(&mut self.pending_pastes, saved.pending_pastes),
        };
        edit(self);
        let saved = SavedDraft {
            textarea: std::mem::replace(&mut self.textarea, preview.textarea),
            pending_pastes: std::mem::replace(&mut self.pending_pastes, preview.pending_pastes),
        };
        if let Some(search) = self.history_search.as_mut() {
            search.saved = Some(saved);
        }
    }
}
