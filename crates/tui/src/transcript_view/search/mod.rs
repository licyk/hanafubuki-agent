// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Incremental literal search (F3) over the full text of the transcript,
//! whatever the compact presentation hides.
//!
//! Search keeps one match and one entry it is scanning; each frame scans a
//! bounded piece of text (`scan`). Reaching the oldest loaded entry asks
//! for an older page, which the app loads. Accepting a match keeps it on
//! screen; cancelling restores the presentation and position from before.

mod scan;

use std::ops::Range;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

use super::input::JumpTarget;
use super::select_mode::SelectSearchOrigin;
use super::*;
use crate::bottom_pane::TextArea;
use crate::key_hint::normalize_key_parts;

const QUERY_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Direction {
    Older,
    Newer,
}

#[derive(Clone, Copy)]
pub(super) struct Cursor {
    anchor: Anchor,
    direction: Direction,
}

#[derive(Clone, Copy, Default)]
pub(super) enum Progress {
    #[default]
    Idle,
    Restart,
    Scanning(Cursor),
    AwaitingHistory,
    Found,
    Exhausted,
}

#[derive(Clone)]
struct Match {
    anchor: Anchor,
    end: usize,
    layout: Arc<TextLayout>,
}

/// A reading search that select mode's own search replaced, restored when
/// that one is left empty.
struct SavedSelectSearch {
    query: String,
    current: Option<Match>,
    progress: Progress,
    page_start: Option<EntryKey>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SearchMode {
    Closed,
    Editing,
    Reading,
}

pub(super) struct Search {
    mode: SearchMode,
    pub(super) editor: TextArea,
    folded_query: String,
    saved_position: Position,
    saved_snapshot: Option<ViewSnapshot>,
    saved_detailed: bool,
    current: Option<Match>,
    saved_select: Option<SavedSelectSearch>,
    pub(super) progress: Progress,
    scanning_layout: Option<(EntryKey, Arc<TextLayout>)>,
    /// The first entry of the page last loaded, where an older scan waits.
    page_start: Option<EntryKey>,
    pub(super) query_truncated: bool,
    pub(super) live: Option<Arc<TextLayout>>,
    live_key: Option<(u16, ActiveCellTranscriptKey)>,
}

impl Default for Search {
    fn default() -> Self {
        Self {
            mode: SearchMode::Closed,
            editor: TextArea::new(),
            folded_query: String::new(),
            saved_position: Position::default(),
            saved_snapshot: None,
            saved_detailed: false,
            current: None,
            saved_select: None,
            progress: Progress::Idle,
            scanning_layout: None,
            page_start: None,
            query_truncated: false,
            live: None,
            live_key: None,
        }
    }
}

impl TranscriptView {
    pub(crate) fn begin_search(&mut self) {
        self.cancel_beginning();
        if self.is_search_editing() {
            return;
        }
        if self.search.is_reading() {
            self.pin_current_search_match();
        }
        if let (Some(mode), Some(selection)) = (&mut self.select_mode, &mut self.selection) {
            let cursor = mode.cursor(selection);
            mode.search_origin = Some(SelectSearchOrigin {
                mark: mode.mark,
                preferred_column: mode.preferred_column,
                start: selection.start,
                end: selection.end,
            });
            mode.mark = None;
            selection.set_keyboard_range(cursor, cursor);
        }
        let saved_position = self.position;
        let saved_snapshot = if self.select_mode.is_some() {
            None
        } else {
            self.selection
                .take()
                .map(|selection| selection.snapshot)
                .or(self.held_reading.take())
        };
        if self.search.is_reading() {
            if self.select_mode.is_some() {
                self.search.saved_select = Some(SavedSelectSearch {
                    query: self.search.editor.text().to_string(),
                    current: self.search.current.clone(),
                    progress: self.search.progress,
                    page_start: self.search.page_start,
                });
                self.search.editor.set_text_clearing_elements("");
                self.search.saved_position = saved_position;
                self.search.saved_snapshot = None;
                self.search.saved_detailed = self.detailed;
            }
            self.held_reading = saved_snapshot;
            self.search.mode = SearchMode::Editing;
            if self.search.current.is_none() {
                self.search.progress = Progress::Restart;
            }
            return;
        }
        self.search.mode = SearchMode::Editing;
        self.disclosure.focused = None;
        self.search.saved_position = saved_position;
        self.search.saved_snapshot = saved_snapshot;
        self.search.saved_detailed = self.detailed;
        self.search.progress = Progress::Restart;
    }

    pub(crate) fn is_search_editing(&self) -> bool {
        self.search.mode == SearchMode::Editing
    }

    /// `true` when search took the key: while the query is edited it
    /// takes every key but paging and the jumps.
    pub(super) fn handle_search_key(
        &mut self,
        key: KeyEvent,
        cells: &[Arc<dyn HistoryCell>],
    ) -> bool {
        if !self.search.is_active() {
            return false;
        }
        let (code, modifiers) = normalize_key_parts(key.code, key.modifiers);
        match (code, modifiers) {
            (KeyCode::Esc, _) if self.search.is_reading() => self.jump_to_latest(),
            (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.cancel_search();
            }
            // A held Enter can arrive as presses; it stays in search, so
            // accepting a match never submits the composer.
            (KeyCode::Enter, _) if self.search.is_reading() => {
                return modifiers == KeyModifiers::NONE;
            }
            (KeyCode::Enter, KeyModifiers::NONE) => self.accept_search(),
            (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                self.next_search_match(cells, Direction::Newer);
            }
            (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                self.next_search_match(cells, Direction::Older);
            }
            (KeyCode::PageUp | KeyCode::PageDown, KeyModifiers::NONE) => return false,
            _ if JumpTarget::from_key(key).is_some() || self.search.is_reading() => return false,
            _ => self.edit_search_query(key),
        }
        true
    }

    /// Accepts the shown match, even while an older one is still sought.
    /// The first scan, and an empty or unmatched query, stay open until
    /// they find one.
    fn accept_search(&mut self) {
        if self.search.editor.is_empty() && self.restore_saved_select_search() {
            return;
        }
        // An idle query can also mean scrolling released a live match.
        if !self.search.editor.is_empty()
            && (self.search.current.is_some() || matches!(self.search.progress, Progress::Idle))
        {
            self.search.mode = SearchMode::Reading;
            self.search.progress = match self.search.progress {
                Progress::Exhausted => Progress::Exhausted,
                _ if self.search.current.is_some() => Progress::Found,
                _ => Progress::Idle,
            };
            self.search.scanning_layout = None;
            self.search.saved_select = None;
        }
    }

    fn edit_search_query(&mut self, key: KeyEvent) {
        let before = self.search.editor.text().to_string();
        let cursor = self.search.editor.cursor();
        self.search.editor.input(key);
        if self.search.editor.text().len() > QUERY_BYTES {
            self.search.editor.set_text_clearing_elements(&before);
            self.search.editor.set_cursor(cursor);
            self.search.query_truncated = true;
        } else if self.search.editor.text() != before {
            if self.held_reading.take().is_some() {
                self.visible.clear();
                self.position = Position::Latest;
            }
            self.search.query_changed();
        }
    }

    /// Restores the presentation and position from before the search.
    pub(crate) fn cancel_search(&mut self) {
        if !self.search.is_active() {
            return;
        }
        if self.select_mode.is_some() && self.restore_saved_select_search() {
            return;
        }
        let position = self.search.saved_position;
        let saved_detailed = self.search.saved_detailed;
        let snapshot = self.search.saved_snapshot.take().or_else(|| {
            let Position::Reading(anchor) = position else {
                return None;
            };
            let mut snapshot = self.held_reading.take().filter(|snapshot| {
                snapshot
                    .cells
                    .iter()
                    .any(|cell| EntryKey::cell(cell) == anchor.key)
            })?;
            if self.detailed != saved_detailed {
                snapshot.pinned.clear();
                snapshot.activities.clear();
            }
            Some(snapshot)
        });
        self.set_presentation(saved_detailed, self.mode);
        self.close_search();
        self.position = position;
        self.held_reading = snapshot;
        self.rewrap_snapshot(self.area.width);
    }

    fn close_search(&mut self) {
        self.search.mode = SearchMode::Closed;
        self.search.editor.set_text_clearing_elements("");
        self.search.query_changed();
        self.search.progress = Progress::Idle;
        self.search.saved_select = None;
        self.search.live = None;
        self.search.live_key = None;
    }

    pub(super) fn accept_search_into_select_mode(&mut self, cells: &[Arc<dyn HistoryCell>]) {
        let Some(found) = self.search.current.as_ref() else {
            self.search.mode = SearchMode::Editing;
            return;
        };
        let anchor = found.anchor;
        let layout = Arc::clone(&found.layout);
        self.move_select_cursor_to_search_match(anchor, &layout, cells);
    }

    pub(super) fn restore_select_search_origin(&mut self) {
        let (Some(mode), Some(selection)) = (&mut self.select_mode, &mut self.selection) else {
            return;
        };
        let Some(origin) = mode.search_origin.take() else {
            return;
        };
        mode.mark = origin.mark;
        mode.preferred_column = origin.preferred_column;
        selection.set_keyboard_range(origin.start, origin.end);
    }

    pub(super) fn cancel_pending_select_search(&mut self) {
        if self.search.is_reading()
            && matches!(
                self.search.progress,
                Progress::Scanning(_) | Progress::AwaitingHistory
            )
        {
            self.search.progress = if self.search.current.is_some() {
                Progress::Found
            } else {
                Progress::Idle
            };
            self.search.scanning_layout = None;
        }
    }

    /// `n`/`N` in select mode: the next match after or before the caret.
    pub(super) fn repeat_select_search(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        newer: bool,
    ) -> bool {
        if !self.search.is_reading() {
            return false;
        }
        let direction = if newer {
            Direction::Newer
        } else {
            Direction::Older
        };
        let Some(cursor) = self
            .select_mode
            .zip(self.selection.as_ref())
            .map(|(mode, selection)| mode.cursor(selection))
        else {
            return false;
        };
        self.pin_current_search_match();
        let index = self.resolve(cells, cursor);
        let Some(layout) = self.layout(cells, index) else {
            return false;
        };
        let offset = layout
            .text()
            .floor_char_boundary(cursor.offset.min(layout.text().len()));
        let offset = if direction == Direction::Newer {
            let row = layout
                .row_for_offset(offset)
                .saturating_add_signed(cursor.row_bias.saturating_neg());
            layout.after_cursor_grapheme(row, offset)
        } else {
            offset
        };
        self.search.current = None;
        self.search.scanning_layout = None;
        self.search.page_start = None;
        self.search.progress = Progress::Scanning(Cursor {
            anchor: Anchor {
                key: self.entry_key(cells, index),
                index,
                offset,
                row_bias: 0,
            },
            direction,
        });
        true
    }

    /// Drops search's expansion when select mode ends, keeping the view.
    pub(super) fn close_search_after_select_mode(&mut self) {
        if !self.search.is_active() {
            return;
        }
        self.pin_current_search_match();
        self.close_search();
        self.rewrap_snapshot(self.area.width);
    }

    fn pin_current_search_match(&mut self) {
        let retained_match = self
            .search
            .current
            .as_ref()
            .map(|found| (found.anchor.key, Arc::clone(&found.layout)));
        if let (Some((key, layout)), Some(snapshot)) = (retained_match, self.snapshot_mut()) {
            snapshot.pinned.insert(key, layout);
        }
    }

    /// Pastes into the query while it is edited; `false` otherwise.
    pub(crate) fn paste_search(&mut self, text: &str) -> bool {
        if !self.is_search_editing() {
            return false;
        }
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let remaining = QUERY_BYTES.saturating_sub(self.search.editor.text().len());
        let end = normalized
            .grapheme_indices(true)
            .map(|(offset, grapheme)| offset + grapheme.len())
            .take_while(|end| *end <= remaining)
            .last()
            .unwrap_or_default();
        if end > 0 {
            if self.held_reading.take().is_some() {
                self.visible.clear();
                self.position = Position::Latest;
            }
            self.search.editor.insert_str(&normalized[..end]);
            self.search.query_changed();
        }
        self.search.query_truncated = end < normalized.len();
        true
    }

    /// Starts over after a presentation change made the searched text and
    /// its offsets stale.
    pub(crate) fn restart_search(&mut self) {
        if self.search.is_active() {
            self.search.query_changed();
            // Reading does not jump when scrolling releases a live revision.
            if self.search.is_reading() {
                self.search.progress = Progress::Idle;
            }
        }
    }

    /// Drops a scan of the live tail when its text changes; a match being
    /// read has its own snapshot.
    pub(super) fn invalidate_live_search(&mut self) {
        let scanning_live = matches!(
            self.search.progress,
            Progress::Scanning(Cursor {
                anchor: Anchor {
                    key: EntryKey::Live,
                    ..
                },
                ..
            })
        );
        if self.held_reading.is_none() && scanning_live {
            self.restart_search();
        }
    }

    pub(super) fn invalidate_held_search(&mut self) {
        if self.search.has_active_query() {
            self.restart_search();
            // Scrolling away from a retained match does not find it again.
            self.search.progress = Progress::Idle;
        }
    }

    /// Rewraps what search holds before scanning at a new width: offsets
    /// belong to a layout.
    pub(crate) fn prepare_width(&mut self, width: u16) {
        if width == self.area.width {
            return;
        }
        self.rewrap_snapshot(width);
        if let Some(found) = &mut self.search.current {
            found.layout = Arc::new(found.layout.rewrap(width));
        }
        if let Some(found) = self
            .search
            .saved_select
            .as_mut()
            .and_then(|saved| saved.current.as_mut())
        {
            found.layout = Arc::new(found.layout.rewrap(width));
        }
        self.area.width = width;
        if self.search.current.is_none()
            || !matches!(self.search.progress, Progress::Found | Progress::Exhausted)
        {
            self.restart_search();
        }
    }

    fn restore_saved_select_search(&mut self) -> bool {
        if !self.search.restore_saved_select() {
            return false;
        }
        self.position = self.search.saved_position;
        true
    }
}

impl Search {
    pub(super) fn match_layout(&self, key: EntryKey) -> Option<Arc<TextLayout>> {
        self.current
            .as_ref()
            .filter(|found| found.anchor.key == key)
            .map(|found| Arc::clone(&found.layout))
    }

    pub(super) fn match_anchor(&self) -> Option<Anchor> {
        self.current.as_ref().map(|found| found.anchor)
    }

    pub(super) fn has_match(&self) -> bool {
        self.current.is_some()
    }

    pub(super) fn is_active(&self) -> bool {
        self.mode != SearchMode::Closed
    }

    pub(super) fn is_reading(&self) -> bool {
        self.mode == SearchMode::Reading
    }

    pub(super) fn has_active_query(&self) -> bool {
        self.is_active() && !self.editor.is_empty()
    }

    pub(super) fn needs_history(&self, history: TranscriptHistoryState) -> bool {
        matches!(self.progress, Progress::AwaitingHistory)
            && history == TranscriptHistoryState::Partial
    }

    pub(super) fn allows_viewport_paging(&self) -> bool {
        !self.is_active()
            || (self.has_active_query()
                && matches!(
                    self.progress,
                    Progress::Idle | Progress::Found | Progress::Exhausted
                ))
    }

    fn query_changed(&mut self) {
        self.folded_query = self
            .editor
            .text()
            .chars()
            .flat_map(char::to_lowercase)
            .collect();
        self.current = None;
        self.scanning_layout = None;
        self.page_start = None;
        self.progress = Progress::Restart;
        self.query_truncated = false;
    }

    fn restore_saved_select(&mut self) -> bool {
        let Some(saved) = self.saved_select.take() else {
            return false;
        };
        self.editor.set_text_clearing_elements(&saved.query);
        self.folded_query = saved.query.chars().flat_map(char::to_lowercase).collect();
        self.current = saved.current;
        self.progress = saved.progress;
        self.scanning_layout = None;
        self.page_start = saved.page_start;
        self.mode = SearchMode::Reading;
        true
    }
}

#[cfg(test)]
mod tests;
