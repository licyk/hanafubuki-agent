// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A viewport over the app's retained cells and the live tail after them:
//! scrolling, following the latest output, mouse and keyboard selection,
//! search, and the disclosure of an activity's details, in a compact or a
//! detailed presentation.
//!
//! The app owns the cells and loads older history; the view owns only its
//! reading and selection state and bounded layout caches. Positions are
//! anchors inside entries ([`position`]), so prepending history never moves
//! the reading position and rewrapping keeps the same text in view.
//!
//! Codex's transcript images, sticky prompt header, turn tips, bookmarks
//! and clipboard writes are not here: a copy is handed to the caller as a
//! [`ViewAction`] with the selected text.

mod disclosure;
mod follow_control;
mod footer;
mod history;
mod input;
mod layout;
mod position;
mod search;
mod select_mode;
mod selection;
mod snapshot;
mod text;

use std::sync::Arc;
use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Widget};

use crate::chatwidget::ActiveCellTranscriptKey;
use crate::history_cell::{
    transcript_entry_lines, ActivityDisclosure, HistoryCell, HistoryRenderMode,
};
use layout::{ActivityTranscriptLines, LayoutCache};
use position::{Anchor, EntryKey, Position};
use search::Search;
use select_mode::SelectMode;
use selection::Selection;
use snapshot::ViewSnapshot;
use text::TextLayout;

pub(crate) use history::TranscriptHistoryState;
pub(crate) use input::ViewAction;

/// A painted row of the view: which entry, which of its rows.
struct VisibleRow {
    index: usize,
    row: usize,
    layout: Arc<TextLayout>,
    key: EntryKey,
    activity_ids: Arc<[String]>,
}

/// The scrolling and interaction state shared by the compact and the
/// detailed presentation.
pub(crate) struct TranscriptView {
    position: Position,
    follow_control: follow_control::FollowControl,
    cache: LayoutCache,
    live: Option<Arc<TextLayout>>,
    live_separated: Option<Arc<TextLayout>>,
    live_key: Option<(u16, ActiveCellTranscriptKey)>,
    live_continuation: bool,
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
    last_tail: Option<EntryKey>,
    last_click: Option<(Instant, u16, u16, u8)>,
    disclosure: disclosure::Disclosure,
}

impl Default for TranscriptView {
    fn default() -> Self {
        Self {
            position: Position::Latest,
            follow_control: follow_control::FollowControl::default(),
            cache: LayoutCache::default(),
            live: None,
            live_separated: None,
            live_key: None,
            live_continuation: false,
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

    /// Lays out the live cells after `cells` in the view's presentation:
    /// their transcript when detailed, otherwise their compact lines with
    /// the disclosure of their activities. `key` identifies their content
    /// (`None` lays them out again); `true` when the tail changed.
    pub(crate) fn sync_live_cells(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        width: u16,
        key: Option<ActiveCellTranscriptKey>,
        active: &[&dyn HistoryCell],
    ) -> bool {
        let ids: Vec<String> = active.iter().flat_map(|cell| cell.activity_ids()).collect();
        let disclose = !ids.is_empty() && self.mode == HistoryRenderMode::Rich;
        let expanded = self.sync_live_activity(cells, ids);
        let (detailed, mode) = (self.detailed, self.mode);
        let transcript = |width: u16| joined_lines(active, |cell| cell.transcript_lines(width));
        self.sync_search_live_tail(width, key, transcript);
        if detailed {
            return self.sync_live_tail(width, key, transcript);
        }
        self.sync_live_activity_tail(width, key, expanded && disclose, |width| {
            let activity = joined_lines(active, |cell| {
                if disclose && expanded {
                    cell.expanded_lines(width)
                } else {
                    transcript_entry_lines(cell, width, false, mode)
                }
            })?;
            let disclosure = if disclose {
                active
                    .iter()
                    .filter(|cell| !cell.activity_ids().is_empty())
                    .map(|cell| cell.activity_disclosure(width))
                    .fold(None, combine_disclosures)
            } else {
                None
            };
            Some(ActivityTranscriptLines {
                activity,
                disclosure,
            })
        })
    }

    /// Lays out the live tail from its lines; `true` when it changed.
    pub(crate) fn sync_live_tail(
        &mut self,
        width: u16,
        key: Option<ActiveCellTranscriptKey>,
        lines: impl FnOnce(u16) -> Option<Vec<Line<'static>>>,
    ) -> bool {
        self.sync_live_layout(width, key, |width| {
            lines(width).map(|lines| TextLayout::new(lines, width))
        })
    }

    fn sync_live_activity_tail(
        &mut self,
        width: u16,
        key: Option<ActiveCellTranscriptKey>,
        expanded: bool,
        lines: impl FnOnce(u16) -> Option<ActivityTranscriptLines>,
    ) -> bool {
        self.sync_live_layout(width, key, |width| {
            lines(width).map(|lines| layout::activity_layout(lines, width, expanded))
        })
    }

    fn sync_live_layout(
        &mut self,
        width: u16,
        key: Option<ActiveCellTranscriptKey>,
        layout: impl FnOnce(u16) -> Option<TextLayout>,
    ) -> bool {
        let next = key.map(|key| (width, key));
        if key.is_some_and(|key| key.cacheable) && self.live_key == next {
            return false;
        }
        let revision_changed = self.live_key.is_none()
            || self.live_key.map(|(_, key)| key.revision) != next.map(|(_, key)| key.revision);
        self.live_key = next;
        self.live_separated = None;
        self.live_continuation = key.is_some_and(|key| key.is_stream_continuation);
        let live = layout(width).map(Arc::new);
        if self.live.as_ref().map(|layout| layout.text())
            != live.as_ref().map(|layout| layout.text())
        {
            if !self.is_following() && revision_changed && live.is_some() {
                self.unseen_activity = true;
            }
            self.invalidate_live_search();
        }
        let changed = self.live.is_some() || live.is_some();
        self.live = live;
        changed
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
        self.live_key = None;
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

    /// Whether the view wants an older page: search ran out of loaded
    /// history, or the reader is near its start.
    pub(crate) fn needs_history(&mut self, cells: &[Arc<dyn HistoryCell>]) -> bool {
        self.search.needs_history(self.history)
            || (self.search.allows_viewport_paging() && self.near_start(cells))
    }
}

/// The cells' lines, a blank line between two cells; `None` when they
/// have none.
fn joined_lines(
    cells: &[&dyn HistoryCell],
    mut lines: impl FnMut(&dyn HistoryCell) -> Vec<Line<'static>>,
) -> Option<Vec<Line<'static>>> {
    let mut joined = Vec::new();
    for cell in cells {
        let cell_lines = lines(*cell);
        if !joined.is_empty() && !cell_lines.is_empty() {
            joined.push(Line::default());
        }
        joined.extend(cell_lines);
    }
    (!joined.is_empty()).then_some(joined)
}

/// What several live activities' details add together.
fn combine_disclosures(
    combined: Option<ActivityDisclosure>,
    next: Option<ActivityDisclosure>,
) -> Option<ActivityDisclosure> {
    match (combined, next) {
        (None, other) | (other, None) => other,
        (Some(ActivityDisclosure::OutputLines(a)), Some(ActivityDisclosure::OutputLines(b))) => {
            Some(ActivityDisclosure::OutputLines(a + b))
        }
        _ => Some(ActivityDisclosure::Generic),
    }
}

#[cfg(test)]
mod tests;
