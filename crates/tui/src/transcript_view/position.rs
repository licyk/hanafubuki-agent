// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Where the view reads: the latest rows, or an anchor inside an entry.
//! An anchor names its entry by identity and a source offset in its text,
//! so prepending history never renumbers it and rewrapping keeps the text
//! at the top of the view.

use super::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum EntryKey {
    Cell(usize),
    /// Past the last entry, where there is none.
    End,
}

impl EntryKey {
    pub(super) fn cell(cell: &Arc<dyn HistoryCell>) -> Self {
        Self::Cell(Arc::as_ptr(cell).cast::<()>() as usize)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Anchor {
    pub(super) key: EntryKey,
    pub(super) index: usize,
    pub(super) offset: usize,
    /// Synthetic rows share source offsets: a positive bias is a row before
    /// the source row (a separator), a negative one a row after it (a
    /// disclosure control).
    pub(super) row_bias: isize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Position {
    #[default]
    Latest,
    Reading(Anchor),
}

impl TranscriptView {
    /// The entry and row at the top of the view.
    pub(super) fn start(&mut self, cells: &[Arc<dyn HistoryCell>]) -> (usize, usize) {
        match self.position {
            Position::Latest => self.bottom_start(cells),
            Position::Reading(anchor) => {
                let index = self.resolve(cells, anchor);
                let row = self.layout(cells, index).map_or(0, |layout| {
                    layout
                        .row_for_offset(anchor.offset)
                        .saturating_add_signed(anchor.row_bias.saturating_neg())
                        .min(layout.row_count().saturating_sub(1))
                });
                self.position = Position::Reading(Anchor {
                    key: self.entry_key(cells, index),
                    index,
                    ..anchor
                });
                (index, row)
            }
        }
    }

    /// The top of the view when its last row is the transcript's last.
    pub(super) fn bottom_start(&mut self, cells: &[Arc<dyn HistoryCell>]) -> (usize, usize) {
        let snapshot = self.snapshot_cells();
        let cells = snapshot.as_deref().unwrap_or(cells);
        let Some(last) = cells.len().checked_sub(1) else {
            return (0, 0);
        };
        let height = self.layout(cells, last).map_or(0, |l| l.row_count());
        self.move_rows(cells, last, height, -(self.area.height as isize))
    }

    pub(super) fn move_rows(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        mut index: usize,
        mut row: usize,
        rows: isize,
    ) -> (usize, usize) {
        if rows < 0 {
            let mut remaining = rows.unsigned_abs();
            while remaining > row && index > 0 {
                remaining -= row;
                index -= 1;
                row = self.layout(cells, index).map_or(0, |l| l.row_count());
            }
            // At the beginning, hidden entries are not content before it: the
            // first visible entry stays where it is.
            if index == 0 && remaining >= row {
                if let Some(first) = self.next_nonempty(cells, index) {
                    if let Some(layout) = self.layout(cells, first) {
                        return (first, usize::from(layout.separated));
                    }
                }
            }
            return (index, row.saturating_sub(remaining));
        }
        let mut remaining = row.saturating_add(rows as usize);
        while let Some(layout) = self.layout(cells, index) {
            if remaining < layout.row_count() {
                break;
            }
            remaining -= layout.row_count();
            index += 1;
        }
        (index, remaining)
    }

    pub(super) fn next_nonempty(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        mut index: usize,
    ) -> Option<usize> {
        while let Some(layout) = self.layout(cells, index) {
            if layout.row_count() > 0 {
                return Some(index);
            }
            index += 1;
        }
        None
    }

    /// The anchor's entry now: at its index if it is still there, else
    /// wherever it moved, else the nearest index left.
    pub(super) fn resolve(&self, cells: &[Arc<dyn HistoryCell>], anchor: Anchor) -> usize {
        let snapshot = self.snapshot_cells();
        let cells = snapshot.as_deref().unwrap_or(cells);
        if cells
            .get(anchor.index)
            .is_some_and(|cell| EntryKey::cell(cell) == anchor.key)
        {
            return anchor.index;
        }
        cells
            .iter()
            .position(|cell| EntryKey::cell(cell) == anchor.key)
            .unwrap_or_else(|| anchor.index.min(cells.len().saturating_sub(1)))
    }

    pub(super) fn entry_key(&self, cells: &[Arc<dyn HistoryCell>], index: usize) -> EntryKey {
        let snapshot = self.snapshot_cells();
        let cells = snapshot.as_deref().unwrap_or(cells);
        cells.get(index).map_or(EntryKey::End, EntryKey::cell)
    }

    /// Whether the view is within a screen of the oldest loaded entry, so
    /// an older page should load.
    pub(crate) fn near_start(&mut self, cells: &[Arc<dyn HistoryCell>]) -> bool {
        if self.area.is_empty() && self.is_following() {
            return false;
        }
        let (index, row) = self.start(cells);
        let threshold = usize::from(self.area.height);
        let mut distance = row;
        if distance > threshold {
            return false;
        }
        for previous in (0..index).rev() {
            distance = distance.saturating_add(
                self.layout(cells, previous)
                    .map_or(0, |layout| layout.row_count()),
            );
            if distance > threshold {
                return false;
            }
        }
        true
    }

    /// Whether the last row of the transcript as it is now is painted.
    pub(super) fn current_tail_is_visible(&mut self, cells: &[Arc<dyn HistoryCell>]) -> bool {
        // An empty entry does not hide the actual last row.
        for index in (0..cells.len()).rev() {
            let Some(layout) = self.current_layout(cells, index) else {
                continue;
            };
            if layout.row_count() == 0 {
                continue;
            }
            let key = EntryKey::cell(&cells[index]);
            return self.visible.last().is_some_and(|visible| {
                visible.key == key
                    && visible.layout.text() == layout.text()
                    && visible.row + 1 == layout.row_count()
            });
        }
        true
    }
}
