//! Scrollback's tail (the plan's §4.11): the newest entries not yet written
//! into the terminal's scrollback stay in the inline viewport above the
//! chat, where they change and expand in place as in Fullscreen, laid out by
//! the conversation's transcript view (`transcript_view::tail`).
//!
//! What leaves the tail is written from its top, in order, as it is shown:
//! - the oldest completed entries, once the tail would grow past its limit
//!   (60% of the screen, at least 8 rows). An entry without a control (an
//!   answer's lines) leaves in part, as far as needed; a live one never;
//! - the whole tail as it was, once a message of yours enters it, when a
//!   session is opened in its place or the other conversation is shown, and
//!   when the terminal is resized, which reflows what the viewport drew
//!   (what has completed, then the rest as it completes);
//! - everything, live or not, when the interface quits.
//!
//! During a turn the viewport does not get shorter, so the terminal is not
//! cleared for a shorter one: a collapsed entry leaves blank rows. The mouse
//! is taken while the tail shows a control (`tui.scrollbackMouse`), and let
//! go at the first turn of the wheel until the next key or entry, so the
//! terminal's own scrolling works from the second.

use std::io::{self, Write};
use std::sync::Arc;
use std::time::Instant;

use hanafubuki_agent::AgentScrollbackMouse;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Rect, Size};
use ratatui::text::Line;

use super::{App, LIVE_TICK};
use crate::history_cell::{plain_lines, HistoryCell, UserHistoryCell};
use crate::render::renderable::Renderable;
use crate::transcript_view::{TailEntry, TranscriptView};
use crate::tui::Tui;

/// The tail's share of the screen at most, and its least.
const TAIL_SHARE_PERCENT: u16 = 60;
const TAIL_MIN_ROWS: u16 = 8;

/// One conversation's tail.
#[derive(Default)]
pub(super) struct Tail {
    cells: Vec<Arc<dyn HistoryCell>>,
    /// The entries that still change, which only leave at exit.
    live: Vec<Arc<dyn HistoryCell>>,
    /// How many leading entries leave whatever the limit.
    flush: usize,
    /// Something was written above the tail, which its first entry is
    /// separated from.
    written: bool,
    /// The viewport's height so far this turn, which it does not go below.
    floor: u16,
    /// The screen's size at the last frame.
    size: Option<Size>,
}

impl Tail {
    pub(super) fn push(&mut self, cell: Arc<dyn HistoryCell>, live: bool) {
        if cell.as_any().is::<UserHistoryCell>() {
            self.flush = self.cells.len();
        }
        if live {
            self.live.push(Arc::clone(&cell));
        }
        self.cells.push(cell);
    }

    /// `cell` completed: it may leave.
    pub(super) fn complete(&mut self, cell: &Arc<dyn HistoryCell>) {
        self.live.retain(|live| !Arc::ptr_eq(live, cell));
    }

    /// Everything shown leaves at the next frame, as far as it has
    /// completed.
    pub(super) fn flush_all(&mut self) {
        self.flush = self.cells.len();
    }

    /// A session is opened in its place: what was live is not any more, and
    /// everything leaves.
    pub(super) fn finish(&mut self) {
        self.live.clear();
        self.flush_all();
    }

    /// The screen was cleared: nothing is shown, nothing was written.
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(super) fn cells(&self) -> &[Arc<dyn HistoryCell>] {
        &self.cells
    }

    /// Takes what leaves the tail at `width` with at most `limit` rows
    /// (`None`: everything, live or not), as the lines to write; and lays
    /// out the entries that stay.
    pub(super) fn take(
        &mut self,
        view: &mut TranscriptView,
        width: u16,
        limit: Option<u16>,
    ) -> (Vec<Line<'static>>, Vec<TailEntry>) {
        let mut entries = view.measure_tail(&self.cells, width, self.written);
        let mut rows: usize = entries.iter().map(|entry| entry.rows).sum();
        let mut lines = Vec::new();
        while let Some(first) = entries.first().copied() {
            let flushing = limit.is_none() || self.flush > 0;
            let excess = limit.map_or(0, |limit| rows.saturating_sub(usize::from(limit)));
            if (!flushing && excess == 0) || (limit.is_some() && self.is_live(0)) {
                break;
            }
            let mut written = view.tail_lines(&self.cells, 0, self.written);
            self.written |= first.rows > 0;
            if !flushing && !first.control && excess < first.rows {
                let rest = written.split_off(excess);
                lines.extend(written);
                self.cells[0] = Arc::new(Remainder(rest));
                entries[0].rows -= excess;
                break;
            }
            lines.extend(written);
            let cell = self.cells.remove(0);
            self.complete(&cell);
            entries.remove(0);
            rows -= first.rows;
            self.flush = self.flush.saturating_sub(1);
        }
        (lines, entries)
    }

    fn is_live(&self, index: usize) -> bool {
        self.cells
            .get(index)
            .is_some_and(|cell| self.live.iter().any(|live| Arc::ptr_eq(live, cell)))
    }
}

/// The rows of an entry the tail wrote in part that it still shows, laid
/// out as they were.
#[derive(Debug)]
struct Remainder(Vec<Line<'static>>);

impl HistoryCell for Remainder {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.0.clone()
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        plain_lines(self.0.clone())
    }

    fn is_stream_continuation(&self) -> bool {
        true
    }
}

/// Whether the mouse is the tail's.
#[derive(Default)]
pub(super) struct TailMouse {
    pub(super) setting: AgentScrollbackMouse,
    /// Let go at a turn of the wheel, until the next key or entry.
    pub(super) released: bool,
}

impl App {
    /// Scrollback's frame: writes what leaves the tail, then paints the
    /// tail and the chat below it.
    pub(super) fn draw_scrollback<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) -> io::Result<()> {
        let size = tui.terminal.size()?;
        if self
            .tail
            .size
            .replace(size)
            .is_some_and(|last| last != size)
        {
            self.tail.flush_all();
            self.tail.floor = 0;
        }
        let chat_height = self.chat_widget.desired_height(size.width);
        let limit = tail_limit(size.height, chat_height);
        let (lines, entries) = self
            .tail
            .take(&mut self.transcript_view, size.width, Some(limit));
        if !lines.is_empty() {
            tui.insert_history_lines(lines);
        }
        let rows: usize = entries.iter().map(|entry| entry.rows).sum();
        let tail_height = u16::try_from(rows).unwrap_or(u16::MAX).min(limit);
        let mut height = tail_height.saturating_add(chat_height);
        if self.chat_widget.is_task_running() {
            height = height.max(self.tail.floor.min(size.height.saturating_sub(1)));
            self.tail.floor = height;
        } else {
            self.tail.floor = 0;
        }
        let capture = self.tail_mouse.setting == AgentScrollbackMouse::Tail
            && !self.tail_mouse.released
            && entries.iter().any(|entry| entry.control);
        let _ = tui.set_inline_mouse_capture(capture);
        let cells = self.tail.cells();
        let view = &mut self.transcript_view;
        let chat_widget = &self.chat_widget;
        tui.draw(height, |frame| {
            let area = frame.area();
            let chat_height = chat_height.min(area.height);
            let tail_area = Rect {
                height: area.height - chat_height,
                ..area
            };
            let chat_area = Rect::new(area.x, tail_area.bottom(), area.width, chat_height);
            view.render_tail(tail_area, frame.buffer_mut(), cells);
            chat_widget.render(chat_area, frame.buffer_mut());
            if let Some((x, y)) = chat_widget.cursor_pos(chat_area) {
                frame.set_cursor_position((x, y));
            }
        })?;
        if self.chat_widget.has_live_entries() {
            self.schedule_frame(Instant::now() + LIVE_TICK);
        }
        // A command's view that panicked while drawn is gone from the next
        // frame, which its error cell schedules.
        self.chat_widget.reap_failed_views();
        Ok(())
    }

    /// The wheel lets the mouse go; a click toggles the control or the
    /// header under it.
    pub(super) fn handle_tail_mouse(&mut self, mouse: MouseEvent) {
        let changed = match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                self.tail_mouse.released = true;
                true
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.transcript_view
                    .press_tail(self.tail.cells(), mouse.column, mouse.row)
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.transcript_view
                    .release_tail(self.tail.cells(), mouse.column, mouse.row)
            }
            _ => false,
        };
        if changed {
            self.schedule_frame(Instant::now());
        }
    }

    /// At exit, the tail is written whole, as it is shown.
    pub(super) fn write_tail_at_exit<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) -> io::Result<()> {
        if self.mode.is_owned() || self.alt_screen_open() {
            return Ok(());
        }
        let width = tui.terminal.size()?.width;
        let (lines, _) = self.tail.take(&mut self.transcript_view, width, None);
        if lines.is_empty() {
            return Ok(());
        }
        tui.insert_history_lines(lines);
        self.draw_scrollback(tui)
    }
}

/// The most rows the tail shows before its oldest entries are written: a
/// share of the screen, at least a few rows, and always one row above the
/// viewport for the lines written into the scrollback.
fn tail_limit(screen_height: u16, chat_height: u16) -> u16 {
    let share = u16::try_from(u32::from(screen_height) * u32::from(TAIL_SHARE_PERCENT) / 100)
        .unwrap_or(u16::MAX);
    share
        .max(TAIL_MIN_ROWS)
        .min(screen_height.saturating_sub(1).saturating_sub(chat_height))
}

#[cfg(test)]
mod tests;
