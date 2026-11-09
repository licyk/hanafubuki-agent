// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The owned transcript's earlier pages (Codex's `history_pagination`): a
//! reopened session shows its latest items, and the view asks for the page
//! before them as the reader nears the start, or for all of them for a jump
//! to the beginning. A page is read through the facade on the loop's
//! thread, as the resume picker's are, and its cells go in after the
//! session's header, before the latest items.
//!
//! A page that cannot be read leaves the view's history failed, and the
//! jump to the beginning (Ctrl+Home) tries again.
//!
//! A replayed tool call whose output is on the next page is shown without
//! its output: the newer page left the output out, having no call for it.

use std::sync::Arc;

use super::App;
use crate::history_cell::{HistoryCell, SessionHeaderHistoryCell};
use crate::transcript_view::TranscriptHistoryState;

impl App {
    /// Reads the earlier pages the view asks for.
    pub(super) fn load_older_pages(&mut self) {
        while let Some(older) = self.older_transcript.clone() {
            // The pages go in after the header, which comes, with the
            // latest items after it, through the loop's channel.
            let Some(header) = self
                .transcript_cells
                .iter()
                .rposition(|cell| cell.as_any().is::<SessionHeaderHistoryCell>())
            else {
                return;
            };
            let view = &mut self.transcript_view;
            let beginning = view.history == TranscriptHistoryState::LoadingBeginning;
            // After a failure only a jump to the beginning tries again.
            if !beginning
                && (view.history == TranscriptHistoryState::Failed
                    || !view.needs_history(&self.transcript_cells))
            {
                return;
            }
            let (items, next) = match self.bridge.read_older_page(&older) {
                Ok(page) => page,
                Err(error) => {
                    self.transcript_view.history = TranscriptHistoryState::Failed;
                    self.chat_widget.add_error_message(error);
                    return;
                }
            };
            let cells: Vec<Arc<dyn HistoryCell>> = self
                .chat_widget
                .replayed_cells(items)
                .into_iter()
                .map(Arc::from)
                .collect();
            let at = header + 1;
            let inserted = at..at + cells.len();
            self.transcript_cells.splice(at..at, cells);
            let view = &mut self.transcript_view;
            view.history_loaded(&self.transcript_cells, inserted);
            view.history = match (&next, beginning) {
                (Some(_), true) => TranscriptHistoryState::LoadingBeginning,
                (Some(_), false) => TranscriptHistoryState::Partial,
                (None, _) => TranscriptHistoryState::Complete,
            };
            if next.is_none() && beginning {
                if view.is_select_mode() {
                    view.complete_select_beginning(&self.transcript_cells);
                } else {
                    view.jump_to_entry(&self.transcript_cells, 0);
                }
            }
            self.older_transcript = next;
        }
    }
}
