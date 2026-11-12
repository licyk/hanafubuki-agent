// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The disclosure of an activity's details in the compact presentation,
//! keyed by the activities' stable identities and apart from selection.
//! F4 focuses the latest visible activity; the list keys then move between
//! activities and Enter shows or hides the focused one's details. A click
//! on an activity's control row toggles it too. Copying and search read
//! the full text whatever is disclosed. A live entry keeps its disclosure
//! as it changes, through its completion.

use std::collections::HashSet;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Modifier, Style};

use super::*;
use crate::key_hint::normalize_key_parts;
use crate::keymap::{self, Action};
use crate::style::accent_color;

/// How many activities a page key moves the focus.
const PAGE_ACTIVITIES: usize = 5;

#[derive(Default)]
pub(super) struct Disclosure {
    pub(super) expanded: HashSet<String>,
    pub(super) focused: Option<Vec<String>>,
}

impl Disclosure {
    pub(super) fn is_expanded(&self, ids: &[String]) -> bool {
        ids.iter().any(|id| self.expanded.contains(id))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListAction {
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    PageUp,
    PageDown,
    JumpTop,
    JumpBottom,
    Accept,
    Cancel,
}

impl ListAction {
    /// Codex's default list keys.
    fn for_key(key: KeyEvent) -> Option<Self> {
        let action =
            match normalize_key_parts(key.code, key.modifiers) {
                (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE)
                | (KeyCode::Char('p' | 'k'), KeyModifiers::CONTROL) => Self::MoveUp,
                (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE)
                | (KeyCode::Char('n' | 'j'), KeyModifiers::CONTROL) => Self::MoveDown,
                (KeyCode::Left, KeyModifiers::NONE)
                | (KeyCode::Char('h'), KeyModifiers::CONTROL) => Self::MoveLeft,
                (KeyCode::Right, KeyModifiers::NONE)
                | (KeyCode::Char('l'), KeyModifiers::CONTROL) => Self::MoveRight,
                (KeyCode::PageUp, KeyModifiers::NONE)
                | (KeyCode::Char('b'), KeyModifiers::CONTROL) => Self::PageUp,
                (KeyCode::PageDown, KeyModifiers::NONE)
                | (KeyCode::Char('f'), KeyModifiers::CONTROL) => Self::PageDown,
                (KeyCode::Home, KeyModifiers::NONE) => Self::JumpTop,
                (KeyCode::End, KeyModifiers::NONE) => Self::JumpBottom,
                (KeyCode::Enter, KeyModifiers::NONE) => Self::Accept,
                (KeyCode::Esc, KeyModifiers::NONE) => Self::Cancel,
                _ => return None,
            };
        Some(action)
    }
}

impl TranscriptView {
    pub(crate) fn is_activity_focused(&self) -> bool {
        self.disclosure.focused.is_some() && !self.detailed && self.mode == HistoryRenderMode::Rich
    }

    pub(crate) fn clear_activity_focus(&mut self) {
        self.disclosure.focused = None;
    }

    fn activity_ids(&self, cells: &[Arc<dyn HistoryCell>], index: usize) -> Vec<String> {
        cells
            .get(index)
            .map(|cell| cell.activity_ids())
            .unwrap_or_default()
    }

    /// The identities of the revision on display, which a pinned revision
    /// keeps.
    pub(super) fn displayed_activity_ids(
        &self,
        cells: &[Arc<dyn HistoryCell>],
        index: usize,
    ) -> Arc<[String]> {
        let key = cells.get(index).map_or(EntryKey::End, EntryKey::cell);
        self.snapshot()
            .and_then(|snapshot| snapshot.activities.get(&key))
            .cloned()
            .unwrap_or_else(|| self.activity_ids(cells, index).into())
    }

    /// Emphasises the focused activity: its control row, else its first
    /// row.
    pub(super) fn render_disclosure(
        &self,
        ids: &[String],
        layout: &TextLayout,
        row: usize,
        area: Rect,
        buf: &mut Buffer,
    ) {
        let focused = self
            .disclosure
            .focused
            .as_ref()
            .is_some_and(|focused| ids.iter().any(|id| focused.contains(id)));
        if !focused || !layout.disclosure {
            return;
        }
        if layout.disclosure_row() == Some(row) {
            let columns = layout.disclosure_columns();
            buf.set_style(
                Rect::new(
                    area.x + columns.start,
                    area.y,
                    columns.end - columns.start,
                    1,
                ),
                Style::default()
                    .fg(accent_color())
                    .bold()
                    .remove_modifier(Modifier::DIM),
            );
        } else if row == usize::from(layout.separated) {
            buf.set_style(area, Style::default().underlined());
        }
    }

    /// Toggles the activity whose control row is under the pointer;
    /// `false` when there is none.
    pub(super) fn toggle_disclosure_at(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        column: u16,
        row: u16,
    ) -> bool {
        if self.detailed || self.mode != HistoryRenderMode::Rich {
            return false;
        }
        let Some(visible) = row
            .checked_sub(self.area.y)
            .and_then(|row| self.visible.get(usize::from(row)))
        else {
            return false;
        };
        let on_control = visible.layout.disclosure_row() == Some(visible.row)
            && column
                .checked_sub(self.area.x)
                .is_some_and(|column| visible.layout.disclosure_columns().contains(&column));
        if !on_control {
            return false;
        }
        let Some(index) = self.canonical_activity_index(cells, &visible.activity_ids) else {
            return false;
        };
        self.toggle_activity(cells, index);
        true
    }

    pub(super) fn handle_disclosure_key(
        &mut self,
        key: KeyEvent,
        cells: &[Arc<dyn HistoryCell>],
    ) -> Option<ViewAction> {
        if self.detailed || self.mode != HistoryRenderMode::Rich || self.is_search_editing() {
            return None;
        }
        if keymap::matches(Action::FocusActivity, &key) {
            if self.is_activity_focused() {
                self.disclosure.focused = None;
            } else {
                self.focus_latest_activity(cells);
            }
            return Some(ViewAction::Changed);
        }
        let focused = self
            .disclosure
            .focused
            .as_ref()
            .and_then(|ids| self.canonical_activity_index(cells, ids));
        let Some(focused) = focused else {
            self.disclosure.focused = None;
            return None;
        };
        let Some(action) = ListAction::for_key(key) else {
            // Typing goes back to the composer: activity navigation never
            // edits text.
            self.disclosure.focused = None;
            return None;
        };
        match action {
            ListAction::Cancel => self.disclosure.focused = None,
            ListAction::Accept => self.toggle_activity(cells, focused),
            ListAction::MoveLeft | ListAction::MoveRight => {
                let ids = self.activity_ids(cells, focused);
                let expanded = self.disclosure.is_expanded(&ids)
                    || self.is_search_expanded(self.entry_key(cells, focused));
                if expanded != (action == ListAction::MoveRight) {
                    self.toggle_activity(cells, focused);
                }
            }
            _ => self.move_activity_focus(cells, focused, action),
        }
        Some(ViewAction::Changed)
    }

    fn focus_latest_activity(&mut self, cells: &[Arc<dyn HistoryCell>]) {
        self.end_selection(cells);
        let visible = self
            .visible
            .iter()
            .rev()
            .find_map(|row| self.canonical_activity_index(cells, &row.activity_ids));
        for index in visible.into_iter().chain((0..cells.len()).rev()) {
            if !self.activity_ids(cells, index).is_empty() && self.focus_activity(cells, index) {
                break;
            }
        }
    }

    fn move_activity_focus(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        focused: usize,
        action: ListAction,
    ) {
        let (mut range, backwards, steps) = match action {
            ListAction::MoveUp => (0..focused, true, 1),
            ListAction::PageUp => (0..focused, true, PAGE_ACTIVITIES),
            ListAction::JumpTop => (0..cells.len(), false, 1),
            ListAction::MoveDown => (focused + 1..cells.len(), false, 1),
            ListAction::PageDown => (focused + 1..cells.len(), false, PAGE_ACTIVITIES),
            _ => (0..cells.len(), true, 1),
        };
        let mut next = focused;
        for _ in 0..steps {
            let candidate = if backwards {
                range
                    .by_ref()
                    .rev()
                    .find(|index| !self.activity_ids(cells, *index).is_empty())
            } else {
                range
                    .by_ref()
                    .find(|index| !self.activity_ids(cells, *index).is_empty())
            };
            let Some(index) = candidate else { break };
            next = index;
        }
        while !self.focus_activity(cells, next) {
            let candidate = if backwards {
                range.next_back()
            } else {
                range.next()
            };
            let Some(index) = candidate else { break };
            next = index;
        }
    }

    /// The entry showing one of the activities: a visible unchanged one
    /// first, else the first in the transcript.
    pub(super) fn canonical_activity_index(
        &self,
        cells: &[Arc<dyn HistoryCell>],
        ids: &[String],
    ) -> Option<usize> {
        if ids.is_empty() {
            return None;
        }
        let shares_id = |index: usize| {
            self.activity_ids(cells, index)
                .iter()
                .any(|id| ids.contains(id))
        };
        if let Some(row) = self.visible.iter().find(|row| {
            row.activity_ids.iter().any(|id| ids.contains(id))
                && cells.get(row.index).map(EntryKey::cell) == Some(row.key)
                && shares_id(row.index)
        }) {
            return Some(row.index);
        }
        (0..cells.len()).find(|index| shares_id(*index))
    }

    fn focus_activity(&mut self, cells: &[Arc<dyn HistoryCell>], index: usize) -> bool {
        if self
            .current_layout(cells, index)
            .is_none_or(|layout| !layout.disclosure)
        {
            return false;
        }
        let ids = self.activity_ids(cells, index);
        let header_visible = self.visible.iter().any(|row| {
            row.layout.disclosure
                && row.activity_ids.iter().any(|id| ids.contains(id))
                && row.row == usize::from(row.layout.separated)
        });
        self.disclosure.focused = Some(ids);
        if !header_visible {
            self.jump_to_entry(cells, index);
        }
        true
    }

    fn toggle_activity(&mut self, cells: &[Arc<dyn HistoryCell>], index: usize) {
        let ids = self.activity_ids(cells, index);
        if ids.is_empty() {
            return;
        }
        let key = self.entry_key(cells, index);
        let matched = self.is_search_expanded(key);
        let previous_top = self.visible.first().map(|visible| {
            let offset = visible.layout.position_at(visible.row, 0);
            (
                visible.key,
                Arc::clone(&visible.activity_ids),
                offset,
                visible.layout.row_for_offset(offset) as isize - visible.row as isize,
            )
        });
        self.end_selection(cells);
        if self.search.has_active_query() && !matched {
            if let Some(snapshot) = &mut self.held_reading {
                snapshot.pinned.remove(&key);
            }
        } else {
            self.release_live_reading();
        }
        // Offsets hold only in an unchanged cell; an entry found by its
        // activities is anchored at its first row.
        let anchor = previous_top
            .and_then(|(top_key, top_ids, offset, row_bias)| {
                let unchanged = cells
                    .iter()
                    .position(|cell| EntryKey::cell(cell) == top_key);
                let top = unchanged.or_else(|| self.canonical_activity_index(cells, &top_ids))?;
                let preserve_offset = unchanged.is_some() && top != index;
                Some(Anchor {
                    key: cells.get(top).map_or(EntryKey::End, EntryKey::cell),
                    index: top,
                    offset: if preserve_offset { offset } else { 0 },
                    row_bias: if preserve_offset { row_bias } else { 0 },
                })
            })
            .unwrap_or(Anchor {
                key: cells.get(index).map_or(EntryKey::End, EntryKey::cell),
                index,
                offset: 0,
                row_bias: 0,
            });
        if matched || self.disclosure.is_expanded(&ids) {
            self.disclosure.expanded.retain(|id| !ids.contains(id));
        } else {
            self.disclosure.expanded.extend(ids.iter().cloned());
        }
        self.disclosure.focused = Some(ids);
        self.position = Position::Reading(anchor);
        self.cache.clear();
        if matched {
            self.invalidate_held_search();
        }
    }
}

#[cfg(test)]
mod tests;
