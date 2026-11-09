// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The terminal-mode transcript (`Ctrl+T`): the session's committed cells
//! in their detailed presentation, with the live cells after them, in a
//! transcript view on the alternate screen. The view does the scrolling,
//! selection, search and copying; this adds the pager's chrome and keys.
//! The app syncs the live tail each frame and holds back the scrollback
//! until the overlay closes.
//!
//! While the app browses the prompts to edit an earlier message
//! (`app::backtrack`), it takes the overlay's browsing keys, highlights the
//! chosen prompt and puts its own hints in place of the overlay's. Codex's
//! named bookmarks and select-mode entry key are not here.

use std::sync::Arc;

use ratatui::crossterm::event::{KeyEventKind, MouseEvent};

use super::*;
use crate::chatwidget::ActiveCellTranscriptKey;
use crate::history_cell::{HistoryCell, HistoryRenderMode};
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
        view.set_presentation(true, mode);
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

    pub(crate) fn set_render_mode(&mut self, mode: HistoryRenderMode) {
        self.view.set_presentation(true, mode);
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

    /// Lays out the live cells after the committed ones at `width`; `true`
    /// when they changed.
    pub(crate) fn sync_live_tail(
        &mut self,
        width: u16,
        key: Option<ActiveCellTranscriptKey>,
        active: &[&dyn HistoryCell],
    ) -> bool {
        self.view.sync_live_cells(&self.cells, width, key, active)
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
        render_key_hints(
            hints,
            buf,
            &[
                (&[KEY_UP, KEY_DOWN], "scroll"),
                (&[KEY_PAGE_UP, KEY_PAGE_DOWN], "page"),
                (&[KEY_HOME, KEY_END], "jump"),
                (&[KEY_Q, KEY_ESC], "close"),
            ],
        );
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
        let rows = match key {
            e if KEY_Q.is_press(e)
                || KEY_ESC.is_press(e)
                || KEY_CTRL_C.is_press(e)
                || KEY_CTRL_T.is_press(e) =>
            {
                self.is_done = true;
                return Some(ViewAction::Changed);
            }
            e if KEY_UP.is_press(e) || KEY_K.is_press(e) => -1,
            e if KEY_DOWN.is_press(e) || KEY_J.is_press(e) => 1,
            e if KEY_SPACE.is_press(e) || KEY_CTRL_F.is_press(e) => page,
            e if KEY_SHIFT_SPACE.is_press(e) || KEY_CTRL_B.is_press(e) => -page,
            e if KEY_CTRL_D.is_press(e) => page / 2,
            e if KEY_CTRL_U.is_press(e) => -page / 2,
            e if KEY_HOME.is_press(e) => {
                self.view.jump_to_beginning(&self.cells);
                return Some(ViewAction::Changed);
            }
            e if KEY_END.is_press(e) => {
                self.view.jump_to_latest();
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
