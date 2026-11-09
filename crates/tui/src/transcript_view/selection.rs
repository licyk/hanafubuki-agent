// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A selection freezes the entries' order and their displayed revisions
//! while the cells keep changing: its snapshot shares the cells and pins
//! only the visible and the selected layouts.

use std::collections::hash_map::Entry;

use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Position as ScreenPosition;
use unicode_segmentation::UnicodeSegmentation;

use super::*;
use crate::text_selection::SelectionUnit;

pub(super) struct Selection {
    pub(super) snapshot: ViewSnapshot,
    pub(super) start: Anchor,
    pub(super) end: Anchor,
    pub(super) dragging: bool,
    pub(super) moved: bool,
    moved_vertically: bool,
    pointer_origin_row: u16,
    /// A click begun while following returns to the latest rows if it
    /// selects nothing.
    pub(super) resume_on_empty: bool,
    pub(super) pointer: Option<ScreenPosition>,
    origin: (Anchor, Anchor),
    unit: SelectionUnit,
    preferred_column: Option<u16>,
}

impl Selection {
    pub(super) fn keyboard(snapshot: ViewSnapshot, cursor: Anchor) -> Self {
        Self {
            snapshot,
            start: cursor,
            end: cursor,
            dragging: false,
            moved: true,
            moved_vertically: false,
            pointer_origin_row: 0,
            resume_on_empty: false,
            pointer: None,
            origin: (cursor, cursor),
            unit: SelectionUnit::Character,
            preferred_column: None,
        }
    }

    pub(super) fn set_keyboard_range(&mut self, start: Anchor, end: Anchor) {
        self.start = start;
        self.end = end;
        self.origin = (start, end);
        self.preferred_column = None;
    }

    pub(super) fn set_keyboard_marked_range(
        &mut self,
        start: Anchor,
        end: Anchor,
        origin: (Anchor, Anchor),
        unit: SelectionUnit,
    ) {
        self.set_keyboard_range(start, end);
        self.origin = origin;
        self.unit = unit;
    }

    pub(super) fn unit(&self) -> SelectionUnit {
        self.unit
    }
}

impl TranscriptView {
    pub(super) fn begin_selection(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        column: u16,
        row: u16,
        clicks: u8,
    ) {
        let Some((anchor, layout)) = self.hit_test(column, row) else {
            return;
        };
        self.cancel_beginning();
        let unit = SelectionUnit::from_clicks(clicks);
        let range = unit.range(layout.text(), anchor.offset);
        let snapshot = self.capture_snapshot(cells);
        self.held_reading = None;
        let start = Anchor {
            offset: range.start,
            ..anchor
        };
        let end = Anchor {
            offset: range.end,
            ..anchor
        };
        let was_following = self.is_following();
        self.selection = Some(Selection {
            snapshot,
            start,
            end,
            origin: (start, end),
            unit,
            preferred_column: None,
            dragging: true,
            moved: false,
            moved_vertically: false,
            pointer_origin_row: row,
            resume_on_empty: was_following,
            pointer: Some(ScreenPosition::new(column, row)),
        });
        if was_following {
            self.hold_position();
        }
    }

    /// Ends the selection and rejoins the current cells; the reading
    /// position stays, kept on its retained revision when its cell is gone
    /// or search still reads it.
    pub(crate) fn end_selection(&mut self, cells: &[Arc<dyn HistoryCell>]) {
        let Some(selection) = self.selection.take() else {
            return;
        };
        if let Some(mode) = &mut self.select_mode {
            let cursor = mode.cursor(&selection);
            mode.mark = None;
            let mut selection = selection;
            selection.set_keyboard_range(cursor, cursor);
            self.selection = Some(selection);
            return;
        }
        let empty = (selection.start.key, selection.start.offset)
            == (selection.end.key, selection.end.offset);
        if empty && selection.resume_on_empty {
            self.position = Position::Latest;
            self.release_live_reading();
            return;
        }
        if self.position == Position::Latest {
            self.hold_position();
        }
        if let Position::Reading(anchor) = self.position {
            if self.search.has_active_query()
                || !cells.iter().any(|cell| EntryKey::cell(cell) == anchor.key)
            {
                self.held_reading = Some(selection.snapshot);
                return;
            }
            let index = self.resolve(cells, anchor);
            let key = self.entry_key(cells, index);
            let (offset, row_bias) = if key == anchor.key {
                (anchor.offset, anchor.row_bias)
            } else {
                (0, 0)
            };
            self.position = Position::Reading(Anchor {
                key,
                index,
                offset,
                row_bias,
            });
        }
    }

    pub(super) fn extend_selection(&mut self, column: u16, row: u16) {
        let Some((end, layout)) = self.hit_test(column, row) else {
            return;
        };
        let Some(mut selection) = self.selection.take() else {
            return;
        };
        // A Shift-click starts a new pointer gesture from the same text.
        if !selection.dragging {
            selection.pointer_origin_row = row;
            selection.moved_vertically = false;
        }
        let cells = Arc::clone(&selection.snapshot.cells);
        let range = selection.unit.range(layout.text(), end.offset);
        let origin = selection.origin;
        let backwards = (self.resolve(&cells, end), end.offset)
            < (self.resolve(&cells, origin.0), origin.0.offset);
        selection.start = if backwards { origin.1 } else { origin.0 };
        selection.end = Anchor {
            offset: if backwards { range.start } else { range.end },
            ..end
        };
        selection.snapshot.pinned.entry(end.key).or_insert(layout);
        selection.moved = true;
        selection.moved_vertically |= selection.pointer_origin_row != row;
        selection.pointer = Some(ScreenPosition::new(column, row));
        selection.preferred_column = None;
        self.pin_selection_range(&cells, &mut selection);
        self.selection = Some(selection);
    }

    /// Whether text is selected; a pointer-down anchor alone selects none.
    pub(crate) fn has_selection_range(&self) -> bool {
        self.selection.as_ref().is_some_and(|selection| {
            selection.start.key != selection.end.key
                || selection.start.offset != selection.end.offset
        })
    }

    /// The selected text, entries joined by line breaks.
    pub(crate) fn selected_text(&mut self, cells: &[Arc<dyn HistoryCell>]) -> Option<String> {
        let mut text = String::new();
        for (index, (layout, range)) in self.selected_ranges(cells)?.into_iter().enumerate() {
            if index > 0 {
                text.push('\n');
            }
            text.push_str(layout.text().get(range)?);
        }
        text.retain(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'));
        Some(text)
    }

    fn selected_ranges(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
    ) -> Option<Vec<(Arc<TextLayout>, std::ops::Range<usize>)>> {
        if !self.has_selection_range() {
            return None;
        }
        let snapshot = self.snapshot_cells();
        let cells = snapshot.as_deref().unwrap_or(cells);
        let selection = self.selection.as_ref()?;
        let mut start = selection.start;
        let mut end = selection.end;
        start.index = self.resolve(cells, start);
        end.index = self.resolve(cells, end);
        if (start.index, start.offset) > (end.index, end.offset) {
            std::mem::swap(&mut start, &mut end);
        }
        if start == end {
            return None;
        }
        let mut ranges = Vec::new();
        for index in start.index..=end.index {
            let layout = self.layout(cells, index)?;
            if layout.row_count() == 0 {
                continue;
            }
            let begin = if index == start.index {
                start.offset
            } else {
                0
            };
            let finish = if index == end.index {
                end.offset
            } else {
                layout.text().len()
            };
            ranges.push((layout, begin..finish));
        }
        Some(ranges)
    }

    /// Whether a click begun at the latest rows is still only a click.
    pub(crate) fn has_pending_latest_selection(&self) -> bool {
        !self.has_selection_range()
            && self
                .selection
                .as_ref()
                .is_some_and(|selection| selection.dragging && selection.resume_on_empty)
    }

    /// Ends the pointer gesture, keeping the selected text, when the input
    /// goes elsewhere.
    pub(crate) fn end_drag(&mut self) {
        self.follow_control = Default::default();
        if self.has_pending_latest_selection() {
            self.position = Position::Latest;
            self.selection = None;
            self.release_live_reading();
        }
        if let Some(selection) = &mut self.selection {
            selection.dragging = false;
        }
    }

    /// Moves the selection's end with an arrow key; `false` for other keys.
    pub(super) fn selection_key(&mut self, cells: &[Arc<dyn HistoryCell>], code: KeyCode) -> bool {
        if !matches!(
            code,
            KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right
        ) {
            return false;
        }
        let Some(end) = self.selection.as_ref().map(|selection| selection.end) else {
            return false;
        };
        let snapshot = self.snapshot_cells();
        let cells = snapshot.as_deref().unwrap_or(cells);
        let column = if matches!(code, KeyCode::Up | KeyCode::Down) {
            let Some(layout) = self.layout(cells, self.resolve(cells, end)) else {
                return true;
            };
            Some(
                self.selection
                    .as_ref()
                    .and_then(|selection| selection.preferred_column)
                    .unwrap_or_else(|| layout.column_for_offset(end.offset)),
            )
        } else {
            None
        };
        let Some(next) = self.move_selection_endpoint(cells, end, code, column) else {
            return true;
        };
        if let Some(mut selection) = self.selection.take() {
            selection.end = next;
            selection.dragging = false;
            selection.preferred_column = column;
            self.pin_selection_range(cells, &mut selection);
            self.selection = Some(selection);
        }
        let row = self.layout(cells, next.index).map_or(0, |layout| {
            layout
                .row_for_offset(next.offset)
                .saturating_add_signed(next.row_bias.saturating_neg())
        });
        if !self
            .visible
            .iter()
            .any(|visible| visible.index == next.index && visible.row == row)
        {
            self.position = Position::Reading(next);
        }
        true
    }

    /// Scrolls while a drag holds the pointer on the view's top or bottom
    /// row; `true` when the view moved. Called on a timer while dragging.
    pub(crate) fn tick_selection(&mut self, cells: &[Arc<dyn HistoryCell>]) -> bool {
        let snapshot = self.snapshot_cells();
        let cells = snapshot.as_deref().unwrap_or(cells);
        // A drag along an edge row does not move the text under it.
        let Some(pointer) = self
            .selection
            .as_ref()
            .filter(|selection| selection.dragging && selection.moved_vertically)
            .and_then(|selection| selection.pointer)
        else {
            return false;
        };
        let direction = if pointer.y <= self.area.top() {
            -1
        } else if pointer.y >= self.area.bottom().saturating_sub(1) {
            1
        } else {
            return false;
        };
        let previous = self.position;
        self.scroll(cells, direction);
        previous != self.position
    }

    pub(super) fn normalize_selection(&mut self, cells: &[Arc<dyn HistoryCell>]) {
        if let Some(mut selection) = self.selection.take() {
            selection.start.index = self.resolve(cells, selection.start);
            selection.end.index = self.resolve(cells, selection.end);
            selection.origin.0.index = self.resolve(cells, selection.origin.0);
            selection.origin.1.index = self.resolve(cells, selection.origin.1);
            self.selection = Some(selection);
        }
    }

    pub(super) fn render_selection(&self, buf: &mut Buffer) {
        let Some(selection) = &self.selection else {
            return;
        };
        let (start, end) = if (selection.start.index, selection.start.offset)
            <= (selection.end.index, selection.end.offset)
        {
            (selection.start, selection.end)
        } else {
            (selection.end, selection.start)
        };
        for (y, visible) in self.visible.iter().enumerate() {
            if visible.index < start.index || visible.index > end.index {
                continue;
            }
            let begin = if visible.key == start.key {
                start.offset
            } else {
                0
            };
            let finish = if visible.key == end.key {
                end.offset
            } else {
                visible.layout.text().len()
            };
            let area = Rect::new(self.area.x, self.area.y + y as u16, self.area.width, 1);
            let continues = (visible.index + 1..=end.index)
                .filter_map(|index| {
                    selection
                        .snapshot
                        .pinned
                        .get(&self.entry_key(&selection.snapshot.cells, index))
                })
                .any(|layout| layout.row_count() > 0);
            visible
                .layout
                .highlight_selection(begin..finish, continues, area, buf, visible.row);
        }
    }

    /// The source position under a screen cell, clamped to the painted
    /// rows.
    pub(super) fn hit_test(&self, column: u16, row: u16) -> Option<(Anchor, Arc<TextLayout>)> {
        let row = row.saturating_sub(self.area.y) as usize;
        let visible = self
            .visible
            .get(row.min(self.visible.len().saturating_sub(1)))?;
        Some((
            Anchor {
                key: visible.key,
                index: visible.index,
                offset: visible
                    .layout
                    .position_at(visible.row, column.saturating_sub(self.area.x)),
                row_bias: 0,
            },
            Arc::clone(&visible.layout),
        ))
    }

    /// Reads from the first painted row instead of following.
    pub(super) fn hold_position(&mut self) {
        if let Some(first) = self.visible.first() {
            let offset = first.layout.position_at(first.row, 0);
            self.position = Position::Reading(Anchor {
                key: first.key,
                index: first.index,
                offset,
                row_bias: first.layout.row_for_offset(offset) as isize - first.row as isize,
            });
        }
    }

    pub(super) fn pin_selection_range(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        selection: &mut Selection,
    ) {
        selection.start.index = self.resolve(cells, selection.start);
        selection.end.index = self.resolve(cells, selection.end);
        for index in selection.start.index.min(selection.end.index)
            ..=selection.start.index.max(selection.end.index)
        {
            let key = self.entry_key(cells, index);
            if let Entry::Vacant(entry) = selection.snapshot.pinned.entry(key) {
                if let Some(layout) = self.layout(cells, index) {
                    entry.insert(layout);
                }
            }
        }
    }

    pub(super) fn move_selection_endpoint(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        end: Anchor,
        code: KeyCode,
        preferred_column: Option<u16>,
    ) -> Option<Anchor> {
        let mut index = self.resolve(cells, end);
        let layout = self.layout(cells, index)?;
        let offset = layout
            .text()
            .floor_char_boundary(end.offset.min(layout.text().len()));
        let mut row_bias = 0;
        let offset = match code {
            KeyCode::Up | KeyCode::Down => {
                let direction = if code == KeyCode::Up { -1 } else { 1 };
                let row = layout
                    .row_for_offset(offset)
                    .saturating_add_signed(end.row_bias.saturating_neg());
                let (next, row) = self.move_rows(cells, index, row, direction);
                index = next;
                let next_layout = self.layout(cells, index)?;
                let offset = next_layout.position_at(
                    row,
                    preferred_column.unwrap_or_else(|| layout.column_for_offset(offset)),
                );
                row_bias = next_layout.row_for_offset(offset) as isize - row as isize;
                offset
            }
            KeyCode::Left if offset == 0 && index > 0 => {
                index -= 1;
                self.layout(cells, index)?.text().len()
            }
            KeyCode::Right if offset == layout.text().len() => {
                if self.layout(cells, index + 1).is_some() {
                    index += 1;
                    0
                } else {
                    offset
                }
            }
            KeyCode::Left => layout.text()[..offset]
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(offset, _)| offset),
            KeyCode::Right => {
                offset
                    + layout.text()[offset..]
                        .graphemes(true)
                        .next()
                        .map_or(0, str::len)
            }
            _ => return None,
        };
        Some(Anchor {
            key: self.entry_key(cells, index),
            index,
            offset,
            row_bias,
        })
    }
}

#[cfg(test)]
mod tests;
