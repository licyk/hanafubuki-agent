// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Scrollback after a resize (Codex's `app/resize_reflow.rs`,
//! `transcript_reflow.rs` and `resize_reflow_cap.rs`). The rows written
//! into the terminal's scrollback are the terminal's: a resize leaves them
//! wrapped for the old width, or rewrapped by the terminal. Once the size
//! has held for [`REFLOW_DEBOUNCE`], the terminal's scrollback and screen
//! are cleared and the transcript before the tail is written again from its
//! cells, laid out for the new width as the tail writes them (an expanded
//! entry stays expanded), its latest rows up to a cap of about what the
//! terminal keeps, under a notice when earlier ones are left out. The tail
//! stays in the viewport as it is.
//!
//! The clear takes everything above the viewport: what the shell printed
//! before the interface started, an earlier session's rows, the other
//! conversation's and the banners between them.
//!
//! A stream's rows are rendered at the width it started at. A resize while
//! one is in the transcript asks for one more rewrite once the stream is
//! consolidated into the cell of its Markdown (`message_consolidation`).
//!
//! Fullscreen lays its view out for each frame's width, so it needs none of
//! this. Nothing runs while the alternate screen is open: Scrollback's frame
//! is not drawn then, and its first frame after notices the size.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use ratatui::backend::Backend;
use ratatui::layout::Size;

use super::App;
use crate::terminal_detection::{TerminalInfo, TerminalName};
use crate::tui::Tui;

/// How long the size holds before the scrollback is written again, so a
/// window dragged to size is written once, at the size it ends at.
pub(super) const REFLOW_DEBOUNCE: Duration = Duration::from_millis(75);

/// A pending rewrite of the scrollback, and whether a stream's end owes one.
#[derive(Default)]
pub(super) struct TranscriptReflow {
    pending_until: Option<Instant>,
    /// The terminal was resized, or the scrollback written again, while a
    /// stream's rows were in the transcript.
    stream_repair: bool,
}

impl TranscriptReflow {
    /// A rewrite after the quiet period, which a later resize pushes out;
    /// when it is due.
    fn schedule_debounced(&mut self, now: Instant) -> Instant {
        let at = now + REFLOW_DEBOUNCE;
        self.pending_until = Some(at);
        at
    }

    /// A rewrite at the next frame.
    pub(super) fn schedule_immediate(&mut self, now: Instant) {
        self.pending_until = Some(now);
    }

    /// Whether a stream's end owes a rewrite; asked once per stream end.
    pub(super) fn take_stream_repair(&mut self) -> bool {
        std::mem::take(&mut self.stream_repair)
    }

    #[cfg(test)]
    pub(super) fn set_due_for_test(&mut self) {
        if self.pending_until.is_some() {
            self.pending_until = Some(Instant::now() - Duration::from_millis(1));
        }
    }

    #[cfg(test)]
    pub(super) fn is_pending(&self) -> bool {
        self.pending_until.is_some()
    }
}

impl App {
    /// Scrollback's frame at `size`: a change of size schedules a rewrite
    /// of the scrollback, and a due one runs.
    pub(super) fn reflow_scrollback_if_due<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        size: Size,
    ) -> io::Result<()> {
        let now = Instant::now();
        if self.tail.note_size(size) {
            if self.has_stream_rows() {
                self.reflow.stream_repair = true;
            }
            let at = self.reflow.schedule_debounced(now);
            self.schedule_frame(at);
        }
        match self.reflow.pending_until {
            Some(at) if at <= now => self.reflow_scrollback(tui, size.width),
            Some(at) => {
                // An earlier frame than the quiet period's end asks again.
                self.schedule_frame(at);
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// Clears the terminal's scrollback and screen and writes the
    /// transcript before the tail again at `width`; the tail keeps the rest.
    fn reflow_scrollback<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        width: u16,
    ) -> io::Result<()> {
        self.reflow.pending_until = None;
        if self.has_stream_rows() {
            self.reflow.stream_repair = true;
        }
        let cells = &self.transcript_cells;
        let start = self.tail.first_unwritten(cells);
        let lines = self
            .transcript_view
            .scrollback_lines(&cells[..start], width, row_cap());
        tui.clear_scrollback()?;
        self.deferred_history_lines.clear();
        self.tail.restart(&cells[start..], !lines.is_empty());
        tui.insert_history_lines(lines);
        // Some terminals report their last size after the frame that
        // handled the resize: a frame after the quiet period looks again.
        self.schedule_frame(Instant::now() + REFLOW_DEBOUNCE);
        Ok(())
    }

    /// A stream runs, or its rows wait for their consolidation.
    fn has_stream_rows(&self) -> bool {
        self.chat_widget.has_active_stream()
            || self
                .transcript_cells
                .iter()
                .rev()
                .any(|cell| cell.stream_id().is_some())
    }
}

/// The most rows a rewrite writes, about what the terminal's scrollback
/// keeps: Codex's for the terminals it knows, 1,000 elsewhere.
fn row_cap() -> usize {
    row_cap_for(&crate::terminal_detection::terminal_info())
}

const DEFAULT_ROW_CAP: usize = 1_000;
const WINDOWS_TERMINAL_ROW_CAP: usize = 9_001;
const WEZTERM_ROW_CAP: usize = 3_500;
const ALACRITTY_ROW_CAP: usize = 10_000;

fn row_cap_for(terminal: &TerminalInfo) -> usize {
    match terminal.name {
        TerminalName::WezTerm => WEZTERM_ROW_CAP,
        TerminalName::Alacritty => ALACRITTY_ROW_CAP,
        TerminalName::WindowsTerminal => WINDOWS_TERMINAL_ROW_CAP,
        _ => DEFAULT_ROW_CAP,
    }
}

#[cfg(test)]
mod tests;
