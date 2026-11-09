// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The loading of older history, as the view sees it, and the cells the
//! app appends while the reader is elsewhere. The app loads the pages; the
//! view asks for them ([`TranscriptView::needs_history`]) and is told what
//! arrived ([`TranscriptView::history_loaded`]).

use std::collections::HashSet;
use std::ops::Range;

use super::*;
use crate::history_cell::SessionHeaderHistoryCell;

/// Where the loading of older history stands.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum TranscriptHistoryState {
    #[default]
    Idle,
    LoadingOlder,
    /// Loading every older page, for a jump to the beginning.
    LoadingBeginning,
    /// Older pages exist and none is loading.
    Partial,
    Failed,
    Complete,
}

impl TranscriptHistoryState {
    pub(crate) fn has_unloaded_history(self) -> bool {
        matches!(
            self,
            Self::LoadingOlder | Self::LoadingBeginning | Self::Partial | Self::Failed
        )
    }
}

impl TranscriptView {
    /// Notes output appended below a reader, before the footer is measured
    /// or the view painted.
    pub(crate) fn sync_history_tail(&mut self, cells: &[Arc<dyn HistoryCell>]) {
        let tail = cells.last().map(EntryKey::cell);
        if !self.is_following() && self.last_tail.is_some() && self.last_tail != tail {
            let previous = cells
                .iter()
                .rposition(|cell| Some(EntryKey::cell(cell)) == self.last_tail);
            let appended_rows = match previous {
                None => true,
                Some(index) => (index + 1..cells.len()).any(|index| {
                    self.current_layout(cells, index)
                        .is_some_and(|layout| layout.row_count() > 0)
                }),
            };
            self.unseen_activity |= appended_rows;
        }
        self.last_tail = tail;
        if self.is_following() {
            self.unseen_activity = false;
        }
    }

    pub(crate) fn cancel_beginning(&mut self) {
        if self.history == TranscriptHistoryState::LoadingBeginning {
            // The page in flight still arrives; the reader's new navigation
            // supersedes the jump.
            self.history = TranscriptHistoryState::LoadingOlder;
        }
    }

    /// An older page was inserted into `cells` at `inserted`.
    pub(crate) fn history_loaded(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        inserted: Range<usize>,
    ) {
        // With only a header and the live tail, the first page extends the
        // tail.
        if !inserted.is_empty()
            && inserted.end == cells.len()
            && self.last_tail == cells[..inserted.start].last().map(EntryKey::cell)
        {
            self.last_tail = cells.last().map(EntryKey::cell);
        }
        let inserted = self.prepend_snapshot_history(cells, inserted);
        if let Position::Reading(anchor) = self.position {
            let index = self.resolve(cells, anchor);
            self.position = Position::Reading(Anchor {
                key: self.entry_key(cells, index),
                index,
                ..anchor
            });
        }
        if !inserted.is_empty() {
            self.search_history_loaded(cells, inserted);
        }
    }

    /// Adds the page to a retained snapshot too, after the session header;
    /// the range it took there.
    fn prepend_snapshot_history(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        inserted: Range<usize>,
    ) -> Range<usize> {
        if inserted.is_empty() {
            return inserted;
        }
        let Some(snapshot) = self.snapshot_mut() else {
            return inserted;
        };
        let existing = snapshot
            .cells
            .iter()
            .map(EntryKey::cell)
            .collect::<HashSet<_>>();
        let older = cells[inserted.clone()]
            .iter()
            .filter(|cell| !existing.contains(&EntryKey::cell(cell)))
            .cloned()
            .collect::<Vec<_>>();
        if older.is_empty() {
            return 0..0;
        }
        let preceding = &cells[..inserted.start];
        let index = snapshot
            .cells
            .iter()
            .rposition(|retained| {
                retained.as_any().is::<SessionHeaderHistoryCell>()
                    || preceding.iter().any(|cell| Arc::ptr_eq(retained, cell))
            })
            .map_or(0, |index| index + 1);
        let retained = index..index + older.len();
        let mut extended = snapshot.cells.to_vec();
        extended.splice(index..index, older);
        snapshot.cells = extended.into();
        retained
    }
}
