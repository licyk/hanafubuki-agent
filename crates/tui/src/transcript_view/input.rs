// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The view's keys and pointer. Typing and composer editing stay with the
//! caller's own input path ([`TranscriptView::owns_interaction_key`] says
//! which keys the view takes first). Dragging selects; Shift-click extends
//! the selection by its unit; a click on an entry's control or header
//! shows or hides its details; a copy hands the selected text back as a
//! [`ViewAction`]. Codex's link activation, copy-on-select and X11 primary
//! selection are not here.

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Position as ScreenPosition;

use super::*;
use crate::key_hint::normalize_key_parts;
use crate::keymap::{self, Action};
use crate::text_selection::{click_count, is_copy_key};

/// The rows one wheel step scrolls.
const WHEEL_ROWS: isize = 3;

/// What the caller does after the view took an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ViewAction {
    /// The view changed; draw it again.
    Changed,
    ExitSelectMode,
    /// Copy the text to the clipboard.
    Copy(String),
    /// Copy the text, then follow the latest output again.
    CopyAndFollow(String),
}

/// The jumps to the transcript's ends, which some terminals send only with
/// Alt (Alt+< and Alt+>) instead of Ctrl+Home and Ctrl+End.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum JumpTarget {
    Beginning,
    Latest,
}

impl JumpTarget {
    /// The keymap's `transcript.jump_top` and `jump_bottom`.
    pub(crate) fn from_key(key: KeyEvent) -> Option<Self> {
        if keymap::matches(Action::TranscriptTop, &key) {
            Some(Self::Beginning)
        } else if keymap::matches(Action::TranscriptBottom, &key) {
            Some(Self::Latest)
        } else {
            None
        }
    }
}

/// The keymap's `transcript.page_up` and `page_down`.
fn is_page_key(key: &KeyEvent) -> bool {
    keymap::matches(Action::TranscriptPageUp, key)
        || keymap::matches(Action::TranscriptPageDown, key)
}

impl TranscriptView {
    /// Whether the view takes `key` before the caller's own bindings: the
    /// keys of its selection, search and select mode while they are open.
    pub(crate) fn owns_interaction_key(&self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        let (code, modifiers) = normalize_key_parts(key.code, key.modifiers);
        if keymap::matches(Action::StartSelection, &key)
            || JumpTarget::from_key(key).is_some()
            || self.select_mode.is_some()
        {
            return true;
        }
        if self.selection.is_some() {
            return is_copy_key(key)
                || code == KeyCode::Esc
                || (code == KeyCode::Enter && modifiers == KeyModifiers::NONE)
                || (matches!(modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
                    && matches!(
                        code,
                        KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down
                    ))
                || is_page_key(&key);
        }
        self.search.is_active()
            && (code == KeyCode::Esc
                || (code == KeyCode::Enter
                    && (self.is_search_editing() || modifiers == KeyModifiers::NONE))
                || is_page_key(&key)
                || (modifiers == KeyModifiers::CONTROL
                    && matches!(code, KeyCode::Char('c' | 'n' | 'p'))))
    }

    pub(crate) fn handle_key(
        &mut self,
        key: KeyEvent,
        cells: &[Arc<dyn HistoryCell>],
    ) -> Option<ViewAction> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if self.select_mode.is_some() {
            return self.handle_select_mode_input(key, cells);
        }
        if let Some(action) = self.handle_disclosure_key(key, cells) {
            return Some(action);
        }
        if self.selection.is_some() {
            if let Some(action) = self.handle_selection_key(key, cells) {
                return Some(action);
            }
            self.end_selection(cells);
        }
        if keymap::matches(Action::StartSelection, &key) {
            self.begin_selection(cells, self.area.x, self.area.y, 1);
            if let Some(selection) = &mut self.selection {
                selection.dragging = false;
                selection.resume_on_empty = false;
            }
            return Some(ViewAction::Changed);
        }
        if keymap::matches(Action::FindTranscript, &key) && !self.is_search_editing() {
            self.begin_search();
            return Some(ViewAction::Changed);
        }
        if self.handle_search_key(key, cells) {
            return Some(ViewAction::Changed);
        }
        self.handle_scroll_key(key, cells)
    }

    fn handle_select_mode_input(
        &mut self,
        key: KeyEvent,
        cells: &[Arc<dyn HistoryCell>],
    ) -> Option<ViewAction> {
        if self.is_search_editing() {
            let (code, modifiers) = normalize_key_parts(key.code, key.modifiers);
            let accepting = code == KeyCode::Enter && modifiers == KeyModifiers::NONE;
            let cancelling = code == KeyCode::Esc
                || (code == KeyCode::Char('c') && modifiers == KeyModifiers::CONTROL);
            if self.handle_search_key(key, cells) {
                if accepting && self.search.is_reading() {
                    self.accept_search_into_select_mode(cells);
                } else if cancelling {
                    self.restore_select_search_origin();
                }
                return Some(ViewAction::Changed);
            }
        }
        if keymap::matches(Action::FindTranscript, &key) {
            self.begin_search();
            return Some(ViewAction::Changed);
        }
        self.handle_select_mode_key(key, cells)
            .or(Some(ViewAction::Changed))
    }

    pub(crate) fn handle_mouse(
        &mut self,
        event: MouseEvent,
        cells: &[Arc<dyn HistoryCell>],
    ) -> Option<ViewAction> {
        let endpoint_changed = self.select_mode.is_some()
            && matches!(
                event.kind,
                MouseEventKind::Down(MouseButton::Left)
                    | MouseEventKind::Drag(MouseButton::Left)
                    | MouseEventKind::Up(MouseButton::Left)
            );
        let before = self.selection_endpoints();
        if self.select_mode.is_none() {
            if let Some(action) = self.handle_follow_control_mouse(event) {
                return Some(action);
            }
        }
        let action = self.handle_pointer(event, cells);
        if endpoint_changed && before != self.selection_endpoints() {
            self.adopt_select_mouse_range();
        }
        action
    }

    fn selection_endpoints(&self) -> Option<(Anchor, Anchor)> {
        self.selection
            .as_ref()
            .map(|selection| (selection.start, selection.end))
    }

    fn handle_pointer(
        &mut self,
        event: MouseEvent,
        cells: &[Arc<dyn HistoryCell>],
    ) -> Option<ViewAction> {
        let point = ScreenPosition::new(event.column, event.row);
        let inside = self.area.contains(point);
        let dragging = self
            .selection
            .as_ref()
            .is_some_and(|selection| selection.dragging);
        if !inside && !dragging {
            return None;
        }
        // The wheel supersedes the last drag position; a real drag event
        // resumes it.
        if matches!(
            event.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) {
            if let Some(selection) = &mut self.selection {
                selection.pointer = None;
            }
        }
        match event.kind {
            MouseEventKind::ScrollUp => self.scroll(cells, -WHEEL_ROWS),
            MouseEventKind::ScrollDown => self.scroll(cells, WHEEL_ROWS),
            MouseEventKind::Down(MouseButton::Right) if inside => {
                return self
                    .selected_text(cells)
                    .filter(|text| !text.is_empty())
                    .map(ViewAction::Copy);
            }
            MouseEventKind::Down(MouseButton::Left) => return self.pointer_down(event, cells),
            MouseEventKind::Drag(MouseButton::Left) if dragging => {
                self.extend_selection(event.column, event.row);
            }
            MouseEventKind::Up(MouseButton::Left) if dragging => {
                if self
                    .selection
                    .as_ref()
                    .is_some_and(|selection| selection.moved && selection.pointer.is_some())
                {
                    self.extend_selection(event.column, event.row);
                }
                self.end_drag();
                if self.selected_text(cells).is_none() {
                    self.end_selection(cells);
                    // A header is a button: a quick second click toggles
                    // it again rather than select a word.
                    if self.release_header_at(cells, event.column, event.row) {
                        self.last_click = None;
                    }
                } else {
                    self.cancel_header_press();
                }
            }
            _ => return None,
        }
        Some(ViewAction::Changed)
    }

    fn handle_selection_key(
        &mut self,
        key: KeyEvent,
        cells: &[Arc<dyn HistoryCell>],
    ) -> Option<ViewAction> {
        if is_copy_key(key) || (key.modifiers == KeyModifiers::NONE && key.code == KeyCode::Enter) {
            return Some(
                self.selected_text(cells)
                    .filter(|text| !text.is_empty())
                    .map_or(ViewAction::Changed, |text| {
                        if key.code == KeyCode::Enter {
                            ViewAction::CopyAndFollow(text)
                        } else {
                            ViewAction::Copy(text)
                        }
                    }),
            );
        }
        if key.code == KeyCode::Esc {
            self.end_selection(cells);
            return Some(ViewAction::Changed);
        }
        if matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
            && self.selection_key(cells, key.code)
        {
            return Some(ViewAction::Changed);
        }
        self.handle_scroll_key(key, cells)
    }

    pub(super) fn handle_scroll_key(
        &mut self,
        key: KeyEvent,
        cells: &[Arc<dyn HistoryCell>],
    ) -> Option<ViewAction> {
        let jump = JumpTarget::from_key(key);
        // A page keeps one row of the last one.
        let page = (self.area.height.max(1) as isize - 1).max(1);
        match (key.code, key.modifiers) {
            _ if keymap::matches(Action::TranscriptPageUp, &key) => self.scroll(cells, -page),
            _ if keymap::matches(Action::TranscriptPageDown, &key) => self.scroll(cells, page),
            (KeyCode::Esc, KeyModifiers::NONE) if self.can_return_to_latest() => {
                self.jump_to_latest();
            }
            _ if jump == Some(JumpTarget::Beginning) => self.jump_to_beginning(cells),
            _ if jump == Some(JumpTarget::Latest) => self.jump_to_latest(),
            _ => return None,
        }
        Some(ViewAction::Changed)
    }

    fn pointer_down(
        &mut self,
        event: MouseEvent,
        cells: &[Arc<dyn HistoryCell>],
    ) -> Option<ViewAction> {
        self.cancel_header_press();
        let plain = !self.is_select_mode() && event.modifiers.is_empty();
        if plain && self.toggle_disclosure_at(cells, event.column, event.row) {
            return Some(ViewAction::Changed);
        }
        self.disclosure.focused = None;
        let visible = self
            .visible
            .get(usize::from(event.row.checked_sub(self.area.y)?))?;
        if event.modifiers == KeyModifiers::SHIFT && self.has_selection_range() {
            self.last_click = None;
            self.extend_selection(event.column, event.row);
            if let Some(selection) = &mut self.selection {
                selection.dragging = true;
            }
            return Some(ViewAction::Changed);
        }
        let clicks = click_count(&mut self.last_click, event.column, event.row);
        // A double click past a row's end selects nothing.
        if clicks >= 2
            && visible
                .layout
                .position_at(visible.row, event.column.saturating_sub(self.area.x))
                == visible.layout.position_at(visible.row, u16::MAX)
        {
            return None;
        }
        if plain && clicks == 1 {
            self.press_header_at(event.column, event.row);
        }
        self.begin_selection(cells, event.column, event.row, clicks);
        Some(ViewAction::Changed)
    }
}

#[cfg(test)]
mod tests;
