//! An entry that changes in place: the chat keeps a handle to it and
//! changes it as its activity goes on, and the transcript shows the same
//! entry from its start. Each change raises its revision, so a layout of an
//! earlier revision is not reused.

use std::sync::Arc;

use parking_lot::{MappedRwLockReadGuard, RwLock, RwLockReadGuard};
use ratatui::text::Line;

use super::{ActivityDisclosure, HistoryCell, HistoryRenderMode};

#[derive(Debug)]
pub(crate) struct LiveCell<C> {
    state: RwLock<Revised<C>>,
}

#[derive(Debug)]
struct Revised<C> {
    cell: C,
    revision: u64,
}

impl<C: HistoryCell> LiveCell<C> {
    pub(crate) fn new(cell: C) -> Arc<Self> {
        Arc::new(Self {
            state: RwLock::new(Revised { cell, revision: 0 }),
        })
    }

    /// Changes the cell under a new revision.
    pub(crate) fn update<R>(&self, change: impl FnOnce(&mut C) -> R) -> R {
        let mut state = self.state.write();
        // Raised first: a change a panic leaves halfway is laid out anew.
        state.revision = state.revision.wrapping_add(1);
        change(&mut state.cell)
    }

    pub(crate) fn cell(&self) -> MappedRwLockReadGuard<'_, C> {
        RwLockReadGuard::map(self.state.read(), |state| &state.cell)
    }
}

impl<C: HistoryCell> HistoryCell for LiveCell<C> {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.cell().display_lines(width)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        self.cell().raw_lines()
    }

    fn display_lines_for_mode(&self, width: u16, mode: HistoryRenderMode) -> Vec<Line<'static>> {
        self.cell().display_lines_for_mode(width, mode)
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.cell().desired_height(width)
    }

    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.cell().transcript_lines(width)
    }

    fn compact_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.cell().compact_lines(width)
    }

    fn expanded_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.cell().expanded_lines(width)
    }

    fn activity_ids(&self) -> Vec<String> {
        self.cell().activity_ids()
    }

    fn activity_disclosure(&self, width: u16) -> Option<ActivityDisclosure> {
        self.cell().activity_disclosure(width)
    }

    fn copy_source(&self) -> Option<String> {
        self.cell().copy_source()
    }

    fn revision(&self) -> u64 {
        self.state.read().revision
    }

    fn transcript_animation_tick(&self) -> Option<u64> {
        self.cell().transcript_animation_tick()
    }

    fn is_stream_continuation(&self) -> bool {
        self.cell().is_stream_continuation()
    }
}
