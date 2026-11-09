// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Select mode: a keyboard caret over the transcript, with vi-like motions
//! and a mark that selects from where it was set to the caret.
//!
//! An empty `Selection` is the caret. The mode keeps only the mark's fixed
//! and moving ends; the selection keeps the retained revisions and makes
//! the text to copy.

mod motion;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

use super::*;
use crate::key_hint::normalize_key_parts;
use crate::text_selection::{is_copy_key, SelectionUnit};

#[derive(Clone, Copy, Default)]
pub(super) struct SelectMode {
    pub(super) mark: Option<Mark>,
    pub(super) preferred_column: Option<u16>,
    pub(super) search_origin: Option<SelectSearchOrigin>,
}

#[derive(Clone, Copy)]
pub(super) struct Mark {
    anchor: Anchor,
    active: Anchor,
    anchor_edges: Option<(Anchor, Anchor)>,
    active_edges: Option<(Anchor, Anchor)>,
    unit: SelectionUnit,
}

/// The caret and mark a search started from, restored when it is
/// cancelled.
#[derive(Clone, Copy)]
pub(super) struct SelectSearchOrigin {
    pub(super) mark: Option<Mark>,
    pub(super) preferred_column: Option<u16>,
    pub(super) start: Anchor,
    pub(super) end: Anchor,
}

#[derive(Clone, Copy)]
enum CursorMotion {
    Character(KeyCode),
    Rows(isize),
    WordForward,
    WordBackward,
    WordEnd,
    LineStart,
    LineEnd,
    Beginning,
    End,
}

impl SelectMode {
    pub(super) fn cursor(self, selection: &Selection) -> Anchor {
        self.mark.map_or(selection.end, |mark| mark.active)
    }
}

impl TranscriptView {
    /// Enters select mode at search's accepted match, else at the lowest
    /// visible selectable position; `false` when there is nothing to
    /// select.
    #[allow(
        dead_code,
        reason = "`/keymap` (P5) binds it; Codex has no default key for select mode"
    )]
    pub(crate) fn begin_select_mode(&mut self, cells: &[Arc<dyn HistoryCell>]) -> bool {
        if self.select_mode.is_some() {
            return true;
        }
        self.cancel_beginning();
        let has_range = self.has_selection_range();
        if self.selection.is_none() {
            let visible_cursor = self.visible.iter().rev().find_map(|visible| {
                let offset = visible.layout.first_cursor_position(visible.row)?;
                Some(Anchor {
                    key: visible.key,
                    index: visible.index,
                    offset,
                    row_bias: visible.layout.row_for_offset(offset) as isize - visible.row as isize,
                })
            });
            let cursor = match self.search.match_anchor().or(visible_cursor) {
                Some(cursor) => Some(cursor),
                None => self.select_boundary(cells, true),
            };
            let Some(cursor) = cursor else {
                return false;
            };
            let snapshot = self.capture_snapshot(cells);
            self.selection = Some(Selection::keyboard(snapshot, cursor));
        }
        if self.position == Position::Latest {
            self.hold_position();
        }
        self.select_mode = Some(SelectMode::default());
        if has_range {
            self.adopt_select_mouse_range();
        } else if self.search.is_reading() {
            self.accept_search_into_select_mode(cells);
        }
        self.reveal_select_cursor(cells);
        true
    }

    pub(crate) fn is_select_mode(&self) -> bool {
        self.select_mode.is_some()
    }

    /// Finishes a jump to the beginning once the last older page is in.
    pub(crate) fn complete_select_beginning(&mut self, cells: &[Arc<dyn HistoryCell>]) {
        if self.select_mode.is_some() {
            self.move_select_cursor(cells, CursorMotion::Beginning);
        }
    }

    /// Leaves the mode, keeping its displayed revision where it was read.
    pub(crate) fn end_select_mode(&mut self) {
        if self.select_mode.take().is_none() {
            return;
        }
        self.cancel_beginning();
        if self.position == Position::Latest {
            self.hold_position();
        }
        if let Some(selection) = self.selection.take() {
            self.held_reading = Some(selection.snapshot);
        }
    }

    pub(crate) fn handle_select_mode_key(
        &mut self,
        key: KeyEvent,
        cells: &[Arc<dyn HistoryCell>],
    ) -> Option<ViewAction> {
        if self.select_mode.is_none() || key.kind == KeyEventKind::Release {
            return None;
        }
        let (code, modifiers) = normalize_key_parts(key.code, key.modifiers);
        let pressed = key.kind == KeyEventKind::Press;
        let plain = modifiers == KeyModifiers::NONE;
        let enter = code == KeyCode::Enter && plain && pressed;
        if is_copy_key(key) || (code == KeyCode::Char('y') && plain) || enter {
            let Some(text) = self.selected_text(cells).filter(|text| !text.is_empty()) else {
                if enter {
                    return Some(self.exit_select_mode());
                }
                return Some(ViewAction::Changed);
            };
            return Some(if enter {
                ViewAction::CopyAndFollow(text)
            } else {
                ViewAction::Copy(text)
            });
        }
        if pressed && plain {
            match code {
                KeyCode::Esc if self.select_mode.is_some_and(|mode| mode.mark.is_some()) => {
                    self.clear_select_mark();
                    return Some(ViewAction::Changed);
                }
                KeyCode::Esc | KeyCode::Char('q') => return Some(self.exit_select_mode()),
                KeyCode::Char('/') => {
                    self.begin_search();
                    return Some(ViewAction::Changed);
                }
                KeyCode::Char('v') | KeyCode::Char(' ') => {
                    self.start_select_mark(cells, SelectionUnit::Character);
                    return Some(ViewAction::Changed);
                }
                KeyCode::Char('o') => {
                    self.swap_select_ends(cells);
                    return Some(ViewAction::Changed);
                }
                _ => {}
            }
        }
        if pressed && code == KeyCode::Char('v') && modifiers == KeyModifiers::SHIFT {
            self.start_select_mark(cells, SelectionUnit::Line);
            return Some(ViewAction::Changed);
        }
        if code == KeyCode::Char('n')
            && matches!(modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
            && self.repeat_select_search(cells, modifiers == KeyModifiers::SHIFT)
        {
            return Some(ViewAction::Changed);
        }
        let arrow = matches!(
            code,
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down
        );
        if arrow
            && modifiers == KeyModifiers::SHIFT
            && self.select_mode.is_some_and(|mode| mode.mark.is_none())
        {
            self.start_select_mark(cells, SelectionUnit::Character);
        }
        let motion = self.select_motion(code, modifiers)?;
        self.move_select_cursor(cells, motion);
        Some(ViewAction::Changed)
    }

    fn select_motion(&self, code: KeyCode, modifiers: KeyModifiers) -> Option<CursorMotion> {
        let page = isize::try_from(self.area.height.saturating_sub(1).max(1)).unwrap_or(isize::MAX);
        let half = isize::try_from(self.area.height.div_ceil(2)).unwrap_or(isize::MAX);
        let motion =
            match (code, modifiers) {
                (KeyCode::Up, KeyModifiers::NONE | KeyModifiers::SHIFT) => CursorMotion::Rows(-1),
                (KeyCode::Down, KeyModifiers::NONE | KeyModifiers::SHIFT) => CursorMotion::Rows(1),
                (KeyCode::Left | KeyCode::Right, KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                    CursorMotion::Character(code)
                }
                (KeyCode::Char('h'), KeyModifiers::NONE) => CursorMotion::Character(KeyCode::Left),
                (KeyCode::Char('l'), KeyModifiers::NONE) => CursorMotion::Character(KeyCode::Right),
                (KeyCode::Char('k'), KeyModifiers::NONE) => CursorMotion::Rows(-1),
                (KeyCode::Char('j'), KeyModifiers::NONE) => CursorMotion::Rows(1),
                (KeyCode::Char('w'), KeyModifiers::NONE) => CursorMotion::WordForward,
                (KeyCode::Char('b'), KeyModifiers::NONE) => CursorMotion::WordBackward,
                (KeyCode::Char('e'), KeyModifiers::NONE) => CursorMotion::WordEnd,
                (KeyCode::Home | KeyCode::Char('0'), KeyModifiers::NONE) => CursorMotion::LineStart,
                (KeyCode::End, KeyModifiers::NONE)
                | (KeyCode::Char('$'), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                    CursorMotion::LineEnd
                }
                (KeyCode::PageUp, KeyModifiers::NONE)
                | (KeyCode::Char('b'), KeyModifiers::CONTROL)
                | (KeyCode::Char(' '), KeyModifiers::SHIFT) => CursorMotion::Rows(-page),
                (KeyCode::PageDown, KeyModifiers::NONE)
                | (KeyCode::Char('f'), KeyModifiers::CONTROL) => CursorMotion::Rows(page),
                (KeyCode::Char('u'), KeyModifiers::CONTROL) => CursorMotion::Rows(-half),
                (KeyCode::Char('d'), KeyModifiers::CONTROL) => CursorMotion::Rows(half),
                (KeyCode::Char('g'), KeyModifiers::NONE)
                | (KeyCode::Home, KeyModifiers::CONTROL) => CursorMotion::Beginning,
                (KeyCode::Char('g'), KeyModifiers::SHIFT)
                | (KeyCode::End, KeyModifiers::CONTROL) => CursorMotion::End,
                _ => return None,
            };
        Some(motion)
    }

    fn exit_select_mode(&mut self) -> ViewAction {
        self.end_select_mode();
        self.close_search_after_select_mode();
        ViewAction::ExitSelectMode
    }

    fn start_select_mark(&mut self, cells: &[Arc<dyn HistoryCell>], unit: SelectionUnit) {
        self.cancel_pending_select_search();
        let Some(mut mode) = self.select_mode else {
            return;
        };
        let Some(selection) = self.selection.as_ref() else {
            return;
        };
        let cursor = mode.cursor(selection);
        mode.mark = match mode.mark {
            Some(mark) if mark.unit == unit => {
                self.clear_select_mark();
                return;
            }
            Some(mark) => Some(Mark {
                anchor_edges: None,
                active_edges: None,
                unit,
                ..mark
            }),
            None => Some(Mark {
                anchor: cursor,
                active: cursor,
                anchor_edges: None,
                active_edges: None,
                unit,
            }),
        };
        mode.preferred_column = None;
        self.select_mode = Some(mode);
        self.sync_select_range(cells);
    }

    fn clear_select_mark(&mut self) {
        self.cancel_pending_select_search();
        let (Some(mode), Some(selection)) = (&mut self.select_mode, &mut self.selection) else {
            return;
        };
        let cursor = mode.cursor(selection);
        mode.mark = None;
        mode.preferred_column = None;
        selection.set_keyboard_range(cursor, cursor);
    }

    fn swap_select_ends(&mut self, cells: &[Arc<dyn HistoryCell>]) {
        self.cancel_pending_select_search();
        let Some(mode) = &mut self.select_mode else {
            return;
        };
        let Some(mark) = &mut mode.mark else {
            return;
        };
        std::mem::swap(&mut mark.anchor, &mut mark.active);
        std::mem::swap(&mut mark.anchor_edges, &mut mark.active_edges);
        mode.preferred_column = None;
        self.sync_select_range(cells);
        self.reveal_select_cursor(cells);
    }

    pub(super) fn reveal_select_cursor(&mut self, cells: &[Arc<dyn HistoryCell>]) {
        let (Some(mode), Some(selection)) = (self.select_mode, self.selection.as_ref()) else {
            return;
        };
        let cursor = mode.cursor(selection);
        let index = self.resolve(cells, cursor);
        let row = self.layout(cells, index).map_or(0, |layout| {
            layout
                .row_for_offset(cursor.offset)
                .saturating_add_signed(cursor.row_bias.saturating_neg())
        });
        if !self
            .visible
            .iter()
            .any(|visible| visible.index == index && visible.row == row)
        {
            self.position = Position::Reading(Anchor { index, ..cursor });
        }
    }

    pub(super) fn render_select_cursor(&self, buf: &mut Buffer) {
        let (Some(mode), Some(selection)) = (self.select_mode, self.selection.as_ref()) else {
            return;
        };
        let cursor = mode.cursor(selection);
        let Some((y, visible)) = self.visible.iter().enumerate().find(|(_, visible)| {
            visible.key == cursor.key
                && visible.row
                    == visible
                        .layout
                        .row_for_offset(cursor.offset)
                        .saturating_add_signed(cursor.row_bias.saturating_neg())
        }) else {
            return;
        };
        let column = visible
            .layout
            .column_for_offset_on_row(visible.row, cursor.offset);
        if column < self.area.width {
            let area = Rect::new(self.area.x + column, self.area.y + y as u16, 1, 1);
            buf.set_style(area, crate::style::selection_style().underlined());
        }
    }

    /// Takes a mouse selection over as the mark, its ends as they were
    /// selected (by character, word or line).
    pub(super) fn adopt_select_mouse_range(&mut self) {
        self.cancel_pending_select_search();
        let has_range = self.has_selection_range();
        let Some(selection) = &self.selection else {
            return;
        };
        let (start, end, unit) = (selection.start, selection.end, selection.unit());
        let snapshot = self.snapshot_cells();
        let cells = snapshot.as_deref().unwrap_or_default();
        let mark = if !has_range {
            None
        } else if self.anchor_order(cells, start) <= self.anchor_order(cells, end) {
            let active = self.before_grapheme(cells, end);
            let anchor_end = self
                .selection_endpoint_bounds(cells, start, unit)
                .map_or(start, |edges| edges.1);
            let active_start = self
                .selection_endpoint_bounds(cells, active, unit)
                .map_or(active, |edges| edges.0);
            Some(Mark {
                anchor: start,
                active,
                anchor_edges: Some((start, anchor_end)),
                active_edges: Some((active_start, end)),
                unit,
            })
        } else {
            let anchor = self.before_grapheme(cells, start);
            let anchor_start = self
                .selection_endpoint_bounds(cells, anchor, unit)
                .map_or(anchor, |edges| edges.0);
            let active_end = self
                .selection_endpoint_bounds(cells, end, unit)
                .map_or(end, |edges| edges.1);
            Some(Mark {
                anchor,
                active: end,
                anchor_edges: Some((anchor_start, start)),
                active_edges: Some((end, active_end)),
                unit,
            })
        };
        if let Some(mode) = &mut self.select_mode {
            mode.preferred_column = None;
            mode.mark = mark;
        }
    }

    pub(super) fn move_select_cursor_to_search_match(
        &mut self,
        mut anchor: Anchor,
        layout: &TextLayout,
        cells: &[Arc<dyn HistoryCell>],
    ) {
        let text = layout.text();
        let graphemes = || text.grapheme_indices(true).map(|(offset, _)| offset);
        let containing = graphemes().rfind(|offset| *offset <= anchor.offset);
        anchor.offset = containing
            .filter(|offset| layout.is_cursor_position(*offset))
            .or_else(|| {
                graphemes()
                    .find(|offset| *offset >= anchor.offset && layout.is_cursor_position(*offset))
            })
            .or_else(|| graphemes().rfind(|offset| layout.is_cursor_position(*offset)))
            .unwrap_or(anchor.offset);

        let (Some(mut mode), Some(mut selection)) =
            (self.select_mode.take(), self.selection.take())
        else {
            return;
        };
        if let Some(origin) = mode.search_origin.take() {
            mode.mark = origin.mark;
            mode.preferred_column = origin.preferred_column;
            selection.set_keyboard_range(origin.start, origin.end);
        }
        mode.preferred_column = None;
        if let Some(mark) = &mut mode.mark {
            mark.active = anchor;
            mark.active_edges = None;
        } else {
            selection.set_keyboard_range(anchor, anchor);
        }
        self.select_mode = Some(mode);
        self.selection = Some(selection);
        self.sync_select_range(cells);
        self.reveal_select_cursor(cells);
    }
}

#[cfg(test)]
mod tests;
