// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The terminal: its modes, set on entry and restored on exit and on a
//! fatal panic; the inline viewport at the bottom of the screen, with the
//! transcript written into the scrollback above it; the alternate screen
//! the overlays take over; and the scheduling of frames.
//!
//! Codex reads the terminal with crossterm's async event stream behind a
//! broker it can pause; here one thread reads it (`input`) and the app loop
//! is a plain receive loop, since nothing pauses the input.

mod input;

use std::fmt;
use std::io::{self, stdout, Result, Stdout, Write};
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement,
};
use ratatui::crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use ratatui::crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::crossterm::{execute, queue, Command};
use ratatui::layout::{Offset, Rect};
use ratatui::text::Line;

use crate::app_event::{AppEvent, AppEventSender};
use crate::custom_terminal;
use crate::custom_terminal::Terminal as CustomTerminal;
pub(crate) use input::InputReader;

/// The terminal type the application draws on.
pub(crate) type Terminal = CustomTerminal<CrosstermBackend<Stdout>>;

/// Whether an overlay holds the alternate screen, so a restore leaves it:
/// leaving it when it was not entered would move the cursor.
static ALT_SCREEN_ACTIVE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EnableAlternateScroll;

impl Command for EnableAlternateScroll {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[?1007h")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> Result<()> {
        Err(std::io::Error::other(
            "tried to execute EnableAlternateScroll using WinAPI; use ANSI instead",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DisableAlternateScroll;

impl Command for DisableAlternateScroll {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[?1007l")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> Result<()> {
        Err(std::io::Error::other(
            "tried to execute DisableAlternateScroll using WinAPI; use ANSI instead",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

fn set_modes() -> Result<()> {
    execute!(stdout(), EnableBracketedPaste)?;
    enable_raw_mode()?;
    // Modified keys (Shift+Enter for a newline) are only told apart with
    // the keyboard enhancement flags; terminals without them (legacy
    // Windows consoles) still work, without the distinction.
    let _ = execute!(
        stdout(),
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
        )
    );
    Ok(())
}

/// Restores the terminal's modes; the inverse of [`set_modes`].
pub(crate) fn restore() -> Result<()> {
    if ALT_SCREEN_ACTIVE.swap(false, Ordering::Relaxed) {
        let _ = execute!(stdout(), DisableAlternateScroll, LeaveAlternateScreen);
    }
    // Pop may fail on platforms that didn't support the push; ignore errors.
    let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
    execute!(stdout(), DisableBracketedPaste)?;
    disable_raw_mode()?;
    let _ = execute!(stdout(), ratatui::crossterm::cursor::Show);
    Ok(())
}

/// Enters the terminal's modes and opens the inline viewport at the
/// cursor. Call it after the process's panic hook is installed: the hook
/// this installs restores the terminal before it runs that one.
pub(crate) fn init() -> Result<Terminal> {
    set_modes()?;
    set_panic_hook();
    let backend = CrosstermBackend::new(stdout());
    // The viewport opens at the cursor, which a terminal that does not
    // answer the position query never gives; the modes go with the error.
    CustomTerminal::with_options(backend).inspect_err(|_| {
        let _ = restore();
    })
}

/// Restores the terminal on a panic that ends the process, before the
/// previous hook reports it (the Agent's aborts, so nothing unwinds to a
/// guard that could restore it later). A contained panic, on an Agent
/// thread the Agent recovers from, leaves the interface running.
fn set_panic_hook() {
    let hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        if !hanafubuki_agent::panic::is_panic_contained() {
            let _ = restore();
        }
        hook(panic_info);
    }));
}

/// Whether the terminal reports modified keys apart. Ask it before the
/// input thread starts, so its reply is not read as input.
pub(crate) fn enhanced_keys_supported() -> bool {
    supports_keyboard_enhancement().unwrap_or(false)
}

/// The terminal with its inline viewport and the transcript lines waiting
/// to be written above it. Tests draw it on a vt100 screen.
pub(crate) struct Tui<B: Backend<Error = io::Error> + Write = CrosstermBackend<Stdout>> {
    pub(crate) terminal: CustomTerminal<B>,
    pending_history_lines: Vec<Line<'static>>,
    /// The inline viewport, kept while an overlay holds the alternate screen.
    alt_saved_viewport: Option<Rect>,
}

impl<B: Backend<Error = io::Error> + Write> Tui<B> {
    pub(crate) fn new(terminal: CustomTerminal<B>) -> Self {
        Self {
            terminal,
            pending_history_lines: Vec::new(),
            alt_saved_viewport: None,
        }
    }

    /// Enter alternate screen and expand the viewport to full terminal size, saving the current
    /// inline viewport for restoration when leaving.
    pub(crate) fn enter_alt_screen(&mut self) -> Result<()> {
        let _ = execute!(self.terminal.backend_mut(), EnterAlternateScreen);
        // Enable "alternate scroll" so terminals may translate wheel to arrows
        let _ = execute!(self.terminal.backend_mut(), EnableAlternateScroll);
        ALT_SCREEN_ACTIVE.store(true, Ordering::Relaxed);
        if let Ok(size) = self.terminal.size() {
            self.alt_saved_viewport = Some(self.terminal.viewport_area);
            self.terminal
                .set_viewport_area(Rect::new(0, 0, size.width, size.height));
            let _ = self.terminal.clear();
        }
        Ok(())
    }

    /// Leave alternate screen and restore the previously saved inline viewport, if any.
    pub(crate) fn leave_alt_screen(&mut self) -> Result<()> {
        // Disable alternate scroll when leaving alt-screen
        let _ = execute!(self.terminal.backend_mut(), DisableAlternateScroll);
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        ALT_SCREEN_ACTIVE.store(false, Ordering::Relaxed);
        if let Some(saved) = self.alt_saved_viewport.take() {
            self.terminal.set_viewport_area(saved);
        }
        Ok(())
    }

    pub(crate) fn insert_history_lines(&mut self, lines: Vec<Line<'static>>) {
        self.pending_history_lines.extend(lines);
    }

    /// Draws the viewport at `height` rows, after writing the pending
    /// transcript lines into the scrollback above it.
    pub(crate) fn draw(
        &mut self,
        height: u16,
        draw_fn: impl FnOnce(&mut custom_terminal::Frame),
    ) -> Result<()> {
        // A cursor-position query cannot run inside the synchronized
        // update, so the viewport's move after a resize is found first.
        let mut pending_viewport_area = self.pending_viewport_area()?;

        queue!(self.terminal.backend_mut(), BeginSynchronizedUpdate)?;
        let drawn = (|| {
            let terminal = &mut self.terminal;
            if let Some(new_area) = pending_viewport_area.take() {
                terminal.set_viewport_area(new_area);
                terminal.clear()?;
            }

            let size = terminal.size()?;

            let mut area = terminal.viewport_area;
            area.height = height.min(size.height);
            area.width = size.width;
            // If the viewport has expanded, scroll everything else up to make room.
            if area.bottom() > size.height {
                terminal
                    .backend_mut()
                    .scroll_region_up(0..area.top(), area.bottom() - size.height)?;
                area.y = size.height - area.height;
            }
            if area != terminal.viewport_area {
                terminal.clear()?;
                terminal.set_viewport_area(area);
            }

            if !self.pending_history_lines.is_empty() {
                crate::insert_history::insert_history_lines(
                    terminal,
                    std::mem::take(&mut self.pending_history_lines),
                )?;
            }

            terminal.draw(|frame| {
                draw_fn(frame);
            })
        })();
        execute!(self.terminal.backend_mut(), EndSynchronizedUpdate)?;
        drawn
    }

    /// After a resize that moved the cursor, the viewport moves with it,
    /// so it stays at the same place relative to the transcript (a
    /// heuristic that works at least in iTerm2).
    fn pending_viewport_area(&mut self) -> Result<Option<Rect>> {
        let terminal = &mut self.terminal;
        let screen_size = terminal.size()?;
        if screen_size == terminal.last_known_screen_size {
            return Ok(None);
        }
        let Ok(cursor_pos) = terminal.get_cursor_position() else {
            return Ok(None);
        };
        let last_known_cursor_pos = terminal.last_known_cursor_pos;
        if cursor_pos.y == last_known_cursor_pos.y {
            return Ok(None);
        }
        let offset = Offset {
            x: 0,
            y: cursor_pos.y as i32 - last_known_cursor_pos.y as i32,
        };
        Ok(Some(terminal.viewport_area.offset(offset)))
    }

    /// Clears the viewport, so what the shell prints next follows the
    /// transcript.
    pub(crate) fn leave(&mut self) -> Result<()> {
        self.terminal.clear()
    }
}

/// A handle the widgets hold to have a frame drawn, now or later.
#[derive(Clone, Debug)]
pub(crate) struct FrameRequester {
    app_event_tx: AppEventSender,
}

impl FrameRequester {
    pub(crate) fn new(app_event_tx: AppEventSender) -> Self {
        Self { app_event_tx }
    }

    pub(crate) fn schedule_frame(&self) {
        self.app_event_tx
            .send(AppEvent::ScheduleFrame(Instant::now()));
    }

    pub(crate) fn schedule_frame_in(&self, dur: Duration) {
        self.app_event_tx
            .send(AppEvent::ScheduleFrame(Instant::now() + dur));
    }

    #[cfg(test)]
    pub(crate) fn test_dummy() -> Self {
        let (tx, _rx) = std::sync::mpsc::channel();
        Self::new(AppEventSender::new(tx))
    }
}

#[cfg(test)]
mod tests;
