// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The terminal-mode transcript (`Ctrl+T`): the session's cells in the
//! compact presentation, whose controls show an entry's details (a click on
//! the control or the header, F4, Ctrl-O for every thinking), the live ones
//! changing in place among them, in a transcript view on
//! the alternate screen; a second `Ctrl+T` shows them all in detail, a third
//! closes it. The view does the scrolling, selection, search and copying;
//! this adds the pager's chrome and keys. The app holds back the scrollback
//! until the overlay closes.
//!
//! While the app browses the prompts to edit an earlier message
//! (`app::backtrack`), it takes the overlay's browsing keys, highlights the
//! chosen prompt and puts its own hints in place of the overlay's. Codex's
//! named bookmarks and select-mode entry key are not here.

use std::sync::Arc;

use ratatui::crossterm::event::{KeyEventKind, MouseEvent};

use super::*;
use crate::history_cell::{
    insert_before, splice_stream, stream_span, HistoryCell, HistoryRenderMode, StreamId,
};
use crate::transcript_view::{TranscriptView, ViewAction};

const TITLE: &str = "/ T R A N S C R I P T";
/// The rows under the content: the view's footer or the status line, and
/// the key hints.
const BOTTOM_ROWS: u16 = 2;

pub(crate) struct TranscriptOverlay {
    view: Box<TranscriptView>,
    cells: Vec<Arc<dyn HistoryCell>>,
    content_area: Rect,
    /// What the last copy said, until the next action.
    notice: Option<String>,
    /// The browsing hints, in place of the overlay's.
    browsing_footer: Option<Line<'static>>,
    is_done: bool,
}

impl TranscriptOverlay {
    pub(crate) fn new(cells: Vec<Arc<dyn HistoryCell>>, mode: HistoryRenderMode) -> Self {
        let mut view = Box::<TranscriptView>::default();
        view.set_presentation(false, mode);
        Self {
            view,
            cells,
            content_area: Rect::default(),
            notice: None,
            browsing_footer: None,
            is_done: false,
        }
    }

    pub(crate) fn insert_cell(&mut self, cell: Arc<dyn HistoryCell>) {
        self.cells.push(cell);
    }

    /// A stream's committed rows, before its tail entry.
    pub(crate) fn insert_cell_before(
        &mut self,
        cell: Arc<dyn HistoryCell>,
        tail: &Arc<dyn HistoryCell>,
    ) {
        insert_before(&mut self.cells, cell, tail);
    }

    /// `stream`'s cells give way to `cell`, the one of its Markdown.
    pub(crate) fn consolidate(&mut self, stream: StreamId, cell: Arc<dyn HistoryCell>) {
        if let Some(span) = stream_span(&self.cells, stream) {
            self.view.replace_entries(&self.cells, span.clone(), &cell);
            splice_stream(&mut self.cells, span, stream, cell);
        }
    }

    pub(crate) fn set_render_mode(&mut self, mode: HistoryRenderMode) {
        self.view.set_presentation(self.view.is_detailed(), mode);
    }

    pub(crate) fn set_presentation(&mut self, detailed: bool, mode: HistoryRenderMode) {
        self.view.set_presentation(detailed, mode);
    }

    pub(crate) fn is_detailed(&self) -> bool {
        self.view.is_detailed()
    }

    /// Highlights the entry at `index` and brings it into view; `jump`
    /// shows it from its top even when some of it is in view.
    pub(crate) fn highlight(&mut self, index: usize, jump: bool) {
        self.view.set_highlight(Some(index));
        if jump {
            self.view.jump_to_entry(&self.cells, index);
        } else {
            self.view.ensure_entry_visible(&self.cells, index);
        }
    }

    pub(crate) fn scroll(&mut self, rows: isize) {
        self.view.scroll(&self.cells, rows);
    }

    pub(crate) fn has_active_interaction(&self) -> bool {
        self.view.has_active_interaction()
    }

    pub(crate) fn set_browsing_footer(&mut self, footer: Line<'static>) {
        self.browsing_footer = Some(footer);
    }

    /// Advances the view's search and an edge-scrolling drag; `true` when
    /// another frame is due soon.
    pub(crate) fn tick(&mut self, width: u16) -> bool {
        self.view.prepare_width(width);
        let scanning = self.view.advance_search(&self.cells);
        let dragging = self.view.tick_selection(&self.cells);
        scanning || dragging
    }

    pub(crate) fn set_notice(&mut self, notice: String) {
        self.notice = Some(notice);
    }

    /// Follows the latest output again, after a copy that asked for it.
    pub(crate) fn follow_latest(&mut self) {
        self.view.jump_to_latest();
    }

    pub(crate) fn is_done(&self) -> bool {
        self.is_done
    }

    pub(crate) fn render(&mut self, area: Rect, buf: &mut Buffer) -> Option<(u16, u16)> {
        Clear.render(area, buf);
        let header = Rect::new(area.x, area.y, area.width, area.height.min(1));
        Span::from("/ ".repeat(area.width as usize / 2))
            .dim()
            .render(header, buf);
        TITLE.dim().render(header, buf);
        let content_height = area.height.saturating_sub(1 + BOTTOM_ROWS);
        self.content_area = Rect::new(area.x, area.y + header.height, area.width, content_height);
        self.view.render(self.content_area, buf, &self.cells);
        let bottom = Rect::new(
            area.x,
            self.content_area.bottom(),
            area.width,
            area.bottom().saturating_sub(self.content_area.bottom()),
        );
        let status = Rect::new(bottom.x, bottom.y, bottom.width, bottom.height.min(1));
        let hints = Rect::new(
            bottom.x,
            status.bottom(),
            bottom.width,
            bottom.height.saturating_sub(1),
        );
        if let Some(footer) = self.view.footer(area.width) {
            Paragraph::new(footer.lines).render(bottom, buf);
            return footer
                .cursor_column
                .map(|column| (bottom.x + column, bottom.y));
        }
        if let Some(footer) = &self.browsing_footer {
            footer.clone().render(status, buf);
            return None;
        }
        let status_line = self
            .notice
            .clone()
            .unwrap_or_else(|| "ctrl + space select · F3 find".to_string());
        Line::from(status_line).dim().render(status, buf);
        let mut keys: Vec<(&[Action], &str)> = vec![
            (&[Action::ScrollUp, Action::ScrollDown], "scroll"),
            (&[Action::PageUp, Action::PageDown], "page"),
            (&[Action::JumpTop, Action::JumpBottom], "jump"),
        ];
        if !self.view.is_detailed() {
            keys.push((&[Action::CloseTranscript], "details"));
        }
        keys.push((&[Action::Close], "close"));
        render_key_hints(hints, buf, &keys);
        None
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Option<ViewAction> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        self.notice = None;
        if self.view.has_active_interaction() {
            if let Some(action) = self.view.handle_key(key, &self.cells) {
                return Some(action);
            }
        }
        let page = self.content_area.height.max(2) as isize - 1;
        if is_ctrl_c(&key) {
            self.is_done = true;
            return Some(ViewAction::Changed);
        }
        let rows = match pager_action(&key) {
            Some(Action::CloseTranscript) if !self.view.is_detailed() => {
                let mode = self.view.mode();
                self.view.set_presentation(true, mode);
                return Some(ViewAction::Changed);
            }
            Some(Action::Close | Action::CloseTranscript) => {
                self.is_done = true;
                return Some(ViewAction::Changed);
            }
            Some(Action::ScrollUp) => -1,
            Some(Action::ScrollDown) => 1,
            Some(Action::PageDown) => page,
            Some(Action::PageUp) => -page,
            Some(Action::HalfPageDown) => page / 2,
            Some(Action::HalfPageUp) => -page / 2,
            Some(Action::JumpTop) => {
                self.view.jump_to_beginning(&self.cells);
                return Some(ViewAction::Changed);
            }
            Some(Action::JumpBottom) => {
                self.view.jump_to_latest();
                return Some(ViewAction::Changed);
            }
            _ if keymap::matches(Action::ToggleThinking, &key) => {
                self.view.toggle_thinking(&self.cells);
                return Some(ViewAction::Changed);
            }
            _ => return self.view.handle_key(key, &self.cells),
        };
        self.view.scroll(&self.cells, rows);
        Some(ViewAction::Changed)
    }

    pub(crate) fn handle_mouse(&mut self, event: MouseEvent) -> Option<ViewAction> {
        self.view.handle_mouse(event, &self.cells)
    }

    /// A paste goes into the search query while it is edited.
    pub(crate) fn handle_paste(&mut self, text: &str) -> bool {
        self.view.paste_search(text)
    }
}

#[cfg(test)]
mod tests;
