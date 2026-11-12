// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The alternate screen, with the input modes that belong to it: the
//! keyboard enhancement flags (the alternate screen has its own stack of
//! them, so they are pushed on entry and popped before leaving) and either
//! the pointer capture a transcript view reads the mouse with or the
//! terminal's alternate scroll, which turns the wheel into arrow keys.
//!
//! A restore on a fatal panic cannot borrow the `Tui`, so the state of the
//! actual screen is kept apart from it. Codex's tmux and Windows console
//! handling and its link pointer shape are not ported.

use std::fmt;
use std::io::{Result, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use ratatui::crossterm::event::{
    DisableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::crossterm::{execute, queue, Command};

pub(super) static ALTERNATE_SCREEN: AlternateScreen = AlternateScreen::new();

pub(super) struct AlternateScreen {
    active: AtomicBool,
    /// Set before the capture is requested: a partial write may already
    /// have turned reporting on, and the cleanup must still turn it off.
    mouse_active: AtomicBool,
    keyboard_active: AtomicBool,
}

impl AlternateScreen {
    pub(super) const fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            mouse_active: AtomicBool::new(false),
            keyboard_active: AtomicBool::new(false),
        }
    }

    pub(super) fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    pub(super) fn is_capturing_mouse(&self) -> bool {
        self.mouse_active.load(Ordering::Relaxed)
    }

    pub(super) fn enter(&self, writer: &mut impl Write, capture_mouse: bool) -> Result<()> {
        queue!(writer, EnterAlternateScreen)?;
        // The queued entry may still reach the terminal after a failed
        // flush; the cleanup must follow it.
        self.active.store(true, Ordering::Relaxed);
        writer.flush()?;
        // Terminals without the enhancement (legacy Windows consoles) refuse
        // the push; keys then arrive without the distinction.
        let pushed = execute!(writer, PushKeyboardEnhancementFlags(enhancement_flags())).is_ok();
        self.keyboard_active.store(pushed, Ordering::Relaxed);
        self.configure_input(writer, capture_mouse)
    }

    /// Reports the pointer to a transcript view, or leaves the wheel to the
    /// terminal's alternate scroll.
    pub(super) fn configure_input(
        &self,
        writer: &mut impl Write,
        capture_mouse: bool,
    ) -> Result<()> {
        if capture_mouse {
            self.mouse_active.store(true, Ordering::Relaxed);
            execute!(writer, DisableAlternateScroll, EnablePointerCapture)
        } else {
            let mouse = self.disable_mouse(writer);
            let scroll = execute!(writer, EnableAlternateScroll);
            mouse.and(scroll)
        }
    }

    pub(super) fn leave(&self, writer: &mut impl Write) -> Result<()> {
        let mouse = self.disable_mouse(writer);
        let keyboard = if self.keyboard_active.swap(false, Ordering::Relaxed) {
            execute!(writer, PopKeyboardEnhancementFlags)
        } else {
            Ok(())
        };
        let scroll = execute!(writer, DisableAlternateScroll);
        // A failed reset must not keep the screen from being left.
        let screen = execute!(writer, LeaveAlternateScreen);
        if screen.is_ok() {
            self.active.store(false, Ordering::Relaxed);
        }
        mouse.and(keyboard).and(scroll).and(screen)
    }

    /// Leaves the screen if it is entered, and stops pointer reporting in
    /// any case.
    pub(super) fn restore(&self, writer: &mut impl Write) -> Result<()> {
        if self.is_active() {
            self.leave(writer)
        } else {
            self.disable_mouse(writer)
        }
    }

    /// Off the alternate screen: reports clicks and the wheel, which the
    /// cleanup turns off as it does a transcript view's capture.
    pub(super) fn capture_clicks(&self, writer: &mut impl Write) -> Result<()> {
        self.mouse_active.store(true, Ordering::Relaxed);
        execute!(writer, EnableClickCapture)
    }

    pub(super) fn disable_mouse(&self, writer: &mut impl Write) -> Result<()> {
        if !self.mouse_active.load(Ordering::Relaxed) {
            return Ok(());
        }
        execute!(writer, DisableMouseCapture)?;
        self.mouse_active.store(false, Ordering::Relaxed);
        Ok(())
    }
}

pub(super) fn enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
}

/// Clicks, drags and pointer motion (the transcript's return-to-bottom
/// control shows its hover), reported in SGR encoding. Crossterm's
/// `EnableMouseCapture` does not report motion without a button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EnablePointerCapture;

impl Command for EnablePointerCapture {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        f.write_str("\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> Result<()> {
        ratatui::crossterm::event::EnableMouseCapture.execute_winapi()
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        false
    }
}

/// Presses, releases and the wheel, without motion, so a drag with Shift
/// still selects in terminals that pass it through. On Windows the console
/// mode crossterm sets for the mouse also turns Quick Edit off while held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EnableClickCapture;

impl Command for EnableClickCapture {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        f.write_str("\x1b[?1000h\x1b[?1006h")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> Result<()> {
        ratatui::crossterm::event::EnableMouseCapture.execute_winapi()
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct EnableAlternateScroll;

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
pub(super) struct DisableAlternateScroll;

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

#[cfg(test)]
mod tests;
