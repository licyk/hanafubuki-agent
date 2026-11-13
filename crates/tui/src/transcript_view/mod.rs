// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A viewport over the app's retained cells: scrolling, following the
//! latest output, mouse and keyboard selection, search, and the disclosure
//! of an activity's details, in a compact or a detailed presentation.
//!
//! The app owns the cells and loads older history; the view owns only its
//! reading and selection state and bounded layout caches. Positions are
//! anchors inside entries ([`position`]), so prepending history never moves
//! the reading position and rewrapping keeps the same text in view. A live
//! entry changes in place under a new revision, which its layout is cached
//! by; a reader inside it keeps the revision it reads.
//!
//! The app may highlight one entry (the prompt chosen while browsing to
//! edit an earlier message) and keep a reading position to return to
//! ([`TranscriptBookmark`]).
//!
//! Codex's transcript images, sticky prompt header, turn tips, named
//! bookmarks and clipboard writes are not here: a copy is handed to the
//! caller as a [`ViewAction`] with the selected text.

mod disclosure;
mod follow_control;
mod footer;
mod history;
mod input;
mod layout;
mod position;
mod rewrite;
mod search;
mod select_mode;
mod selection;
mod snapshot;
mod tail;
mod text;

use std::sync::Arc;
use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Widget};

use crate::history_cell::{
    transcript_entry_lines, ActivityDisclosure, HistoryCell, HistoryRenderMode,
};
use layout::LayoutCache;
use position::{Anchor, EntryKey, Position};
use search::Search;
use select_mode::SelectMode;
use selection::Selection;
use snapshot::ViewSnapshot;
use text::TextLayout;

pub(crate) use history::TranscriptHistoryState;
pub(crate) use input::ViewAction;
pub(crate) use tail::TailEntry;

/// A painted row of the view: which entry, which of its rows.
struct VisibleRow {
    index: usize,
    row: usize,
    layout: Arc<TextLayout>,
    key: EntryKey,
    activity_ids: Arc<[String]>,
}

/// A reading position the view can return to.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TranscriptBookmark(Position);

/// The scrolling and interaction state shared by the compact and the
/// detailed presentation.
pub(crate) struct TranscriptView {
    position: Position,
    follow_control: follow_control::FollowControl,
    cache: LayoutCache,
    area: Rect,
    visible: Vec<VisibleRow>,
    selection: Option<Selection>,
    select_mode: Option<SelectMode>,
    held_reading: Option<ViewSnapshot>,
    search: Search,
    detailed: bool,
    mode: HistoryRenderMode,
    pub(crate) history: TranscriptHistoryState,
    /// The other presentation's reading position, restored when it returns.
    saved_position: Option<Position>,
    unseen_activity: bool,
    tail_visible: bool,
    /// The last entry and its revision when last synced.
    last_tail: Option<(EntryKey, u64)>,
    last_click: Option<(Instant, u16, u16, u8)>,
    disclosure: disclosure::Disclosure,
    /// The entry drawn reversed, unless a selection or a search shows.
    highlight: Option<usize>,
    /// The first entry is separated from what is above the view too: the
    /// lines Scrollback wrote above its tail.
    leading_separator: bool,
    /// The view lays out Scrollback's tail, whose rows are written into the
    /// terminal's scrollback as they are shown, rather than entries it
    /// lays out again.
    scrollback_tail: bool,
}

impl Default for TranscriptView {
    fn default() -> Self {
        Self {
            position: Position::Latest,
            follow_control: follow_control::FollowControl::default(),
            cache: LayoutCache::default(),
            area: Rect::default(),
            visible: Vec::new(),
            selection: None,
            select_mode: None,
            held_reading: None,
            search: Search::default(),
            detailed: false,
            mode: HistoryRenderMode::Rich,
            history: TranscriptHistoryState::Complete,
            saved_position: None,
            unseen_activity: false,
            tail_visible: true,
            last_tail: None,
            last_click: None,
            disclosure: disclosure::Disclosure::default(),
            highlight: None,
            leading_separator: false,
            scrollback_tail: false,
        }
    }
}

impl TranscriptView {
    pub(crate) fn render(&mut self, area: Rect, buf: &mut Buffer, cells: &[Arc<dyn HistoryCell>]) {
        self.cache.begin_frame();
        self.sync_history_tail(cells);
        let current_cells = cells;
        let snapshot = self.snapshot_cells();
        let cells = snapshot.as_deref().unwrap_or(cells);
        Clear.render(area, buf);
        self.prepare_width(area.width);
        self.area = area;
        self.normalize_selection(cells);
        self.visible.clear();
        if area.is_empty() {
            self.tail_visible = false;
            return;
        }
        let (mut index, mut row) = self.start(cells);
        let mut entry_activity_ids: Option<(usize, Arc<[String]>)> = None;
        for y in area.top()..area.bottom() {
            let Some(layout) = self.layout(cells, index) else {
                break;
            };
            if row >= layout.row_count() {
                // Empty entries are legal (a cell with nothing to show).
                let Some(next) = self.next_nonempty(cells, index + 1) else {
                    break;
                };
                index = next;
                row = 0;
            }
            let Some(layout) = self.layout(cells, index) else {
                break;
            };
            let key = self.entry_key(cells, index);
            let activity_ids = match &entry_activity_ids {
                Some((previous, ids)) if *previous == index => Arc::clone(ids),
                _ => {
                    let ids = self.displayed_activity_ids(cells, index);
                    entry_activity_ids = Some((index, Arc::clone(&ids)));
                    ids
                }
            };
            let row_area = Rect::new(area.x, y, area.width, 1);
            layout.render(row_area, buf, row);
            self.render_disclosure(&activity_ids, &layout, row, row_area, buf);
            if self.highlight == Some(index) && self.selection.is_none() && !self.search.is_active()
            {
                layout.highlight(0..layout.text().len(), row_area, buf, row);
            }
            self.visible.push(VisibleRow {
                index,
                row,
                layout,
                key,
                activity_ids,
            });
            row += 1;
        }
        let dragged_pointer = self
            .selection
            .as_ref()
            .filter(|selection| selection.dragging && selection.moved)
            .and_then(|selection| selection.pointer);
        if let Some(pointer) = dragged_pointer {
            self.extend_selection(pointer.x, pointer.y);
        }
        self.tail_visible = self.current_tail_is_visible(current_cells);
        if self.tail_visible {
            self.unseen_activity = false;
        }
        self.render_selection(buf);
        self.render_select_cursor(buf);
        if self.history == TranscriptHistoryState::LoadingBeginning && self.is_following() {
            self.scroll(current_cells, 0);
        }
        self.render_search(buf);
    }

    /// Switches between the compact and the detailed presentation (Ctrl+T)
    /// or the rich and the raw one; each presentation keeps its own
    /// reading position.
    pub(crate) fn set_presentation(&mut self, detailed: bool, mode: HistoryRenderMode) {
        if self.detailed == detailed && self.mode == mode {
            return;
        }
        self.selection = None;
        self.select_mode = None;
        self.release_live_reading();
        self.cache.clear();
        // Search keeps its own origin, apart from either presentation's.
        if self.detailed != detailed && !self.search.is_active() {
            let previous = self.position;
            self.position = self.saved_position.take().unwrap_or(previous);
            self.saved_position = Some(previous);
        }
        self.restart_search();
        self.detailed = detailed;
        self.mode = mode;
    }

    pub(crate) fn has_active_interaction(&self) -> bool {
        self.select_mode.is_some()
            || self.selection.is_some()
            || self.search.is_active()
            || self.is_activity_focused()
    }

    pub(crate) fn is_detailed(&self) -> bool {
        self.detailed
    }

    pub(crate) fn mode(&self) -> HistoryRenderMode {
        self.mode
    }

    pub(crate) fn is_following(&self) -> bool {
        self.select_mode.is_none() && self.selection.is_none() && self.position == Position::Latest
    }

    pub(crate) fn can_return_to_latest(&self) -> bool {
        !self.is_following()
    }

    /// Jumps to the first entry; with older history still unloaded, the
    /// view holds its position until every page has arrived.
    pub(crate) fn jump_to_beginning(&mut self, cells: &[Arc<dyn HistoryCell>]) {
        if self.history == TranscriptHistoryState::LoadingBeginning {
            return;
        }
        if self.history.has_unloaded_history() {
            self.scroll(cells, 0);
            self.history = TranscriptHistoryState::LoadingBeginning;
        } else {
            self.jump_to_entry(cells, 0);
        }
    }

    pub(crate) fn jump_to_latest(&mut self) {
        self.cancel_search();
        self.cancel_beginning();
        self.position = Position::Latest;
        self.selection = None;
        self.select_mode = None;
        self.release_live_reading();
        self.unseen_activity = false;
        self.disclosure.focused = None;
    }

    pub(crate) fn scroll(&mut self, cells: &[Arc<dyn HistoryCell>], rows: isize) {
        if rows != 0 {
            self.last_click = None;
            self.cancel_beginning();
        }
        if self.area.is_empty() {
            return;
        }
        let current_cells = cells;
        let snapshot = self.snapshot_cells();
        let cells = snapshot.as_deref().unwrap_or(cells);
        let start = self.start(cells);
        let (index, row) = self.move_rows(cells, start.0, start.1, rows);
        if rows < 0
            && (index, row) == start
            && self.is_following()
            && !self.history.has_unloaded_history()
        {
            return;
        }
        if (index, row) != start {
            if let Some(selection) = &mut self.selection {
                selection.resume_on_empty = false;
            }
        }
        // Anchor visible content, not an empty entry before it.
        let index = self.next_nonempty(cells, index).unwrap_or(index);
        let bottom = self.bottom_start(cells);
        if rows > 0 && (index, row) >= bottom {
            if self.selection.is_none() && !self.search.is_active() {
                self.jump_to_latest();
            } else {
                self.release_live_reading();
                self.position = Position::Latest;
            }
            return;
        }
        if let Some(layout) = self.layout(cells, index) {
            let offset = layout.position_at(row, 0);
            self.position = Position::Reading(Anchor {
                key: self.entry_key(cells, index),
                index,
                offset,
                row_bias: layout.row_for_offset(offset) as isize - row as isize,
            });
            self.hold_live_reading(current_cells, layout);
        }
    }

    pub(crate) fn jump_to_entry(&mut self, cells: &[Arc<dyn HistoryCell>], index: usize) {
        self.cancel_beginning();
        self.release_live_reading();
        let snapshot = self.snapshot_cells();
        let cells = snapshot.as_deref().unwrap_or(cells);
        self.position = Position::Reading(Anchor {
            key: self.entry_key(cells, index),
            index,
            offset: 0,
            row_bias: 0,
        });
    }

    /// Shows the entry at `index` if none of its non-blank rows is in view,
    /// from its top.
    pub(crate) fn ensure_entry_visible(&mut self, cells: &[Arc<dyn HistoryCell>], index: usize) {
        let width = self.area.width;
        let shown = self.visible.iter().any(|row| {
            let start = row.layout.position_at(row.row, 0);
            let end = row.layout.position_at(row.row, width);
            row.index == index && !row.layout.text()[start..end].trim().is_empty()
        });
        if !shown {
            // A jump to the beginning under way goes on.
            let history = self.history;
            self.jump_to_entry(cells, index);
            self.history = history;
        }
    }

    pub(crate) fn set_highlight(&mut self, index: Option<usize>) {
        self.highlight = index;
    }

    pub(crate) fn bookmark(&self) -> TranscriptBookmark {
        TranscriptBookmark(self.position)
    }

    pub(crate) fn restore_bookmark(&mut self, bookmark: TranscriptBookmark) {
        match bookmark.0 {
            Position::Latest => self.jump_to_latest(),
            position => {
                self.release_live_reading();
                self.position = position;
            }
        }
    }

    /// Whether the view wants an older page: search ran out of loaded
    /// history, or the reader is near its start.
    pub(crate) fn needs_history(&mut self, cells: &[Arc<dyn HistoryCell>]) -> bool {
        self.search.needs_history(self.history)
            || (self.search.allows_viewport_paging() && self.near_start(cells))
    }
}

#[cfg(test)]
mod tests;
