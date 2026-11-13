// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The terminal handed to another program in the foreground (the external
//! editor) and taken back, as Codex's `Tui::with_restored`, in two steps so
//! a test can see the terminal in between. Released, the terminal is as the
//! shell left it: the input thread stopped, the alternate screen left, the
//! pointer, the keyboard enhancement and bracketed paste off, the cursor
//! shown and raw mode off. Taken back, each is as it was, the input typed
//! meanwhile dropped, and the screen drawn again in full.
//!
//! Codex keeps raw mode on and, on the alternate screen, repaints its last
//! frame there for an editor in another window; here the terminal is
//! released whole, the alternate screen included.

use std::io::{self, Write};

use ratatui::backend::Backend;
use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode};

use super::alternate_screen::enhancement_flags;
use super::Tui;

/// What the terminal had before [`Tui::release`].
#[must_use]
pub(crate) struct Released {
    /// The alternate screen was entered, reporting the pointer or not.
    alt_screen: Option<bool>,
    inline_mouse: bool,
}

impl<B: Backend<Error = io::Error> + Write> Tui<B> {
    pub(crate) fn release(&mut self) -> Released {
        if let Some(input) = self.input.as_mut() {
            input.pause();
        }
        let alt_screen = self
            .alt_saved_viewport
            .is_some()
            .then_some(self.alt_mouse_capture);
        if alt_screen.is_some() {
            let _ = self.leave_alt_screen();
        }
        let inline_mouse = std::mem::take(&mut self.inline_mouse_capture);
        let backend = self.terminal.backend_mut();
        // Off whatever the shared record of the pointer says.
        let _ = execute!(backend, DisableMouseCapture);
        // Refused by a terminal without the enhancement, as its push was.
        let _ = execute!(backend, PopKeyboardEnhancementFlags);
        let _ = execute!(backend, DisableBracketedPaste);
        let _ = self.terminal.show_cursor();
        if self.input.is_some() {
            let _ = disable_raw_mode();
        }
        Released {
            alt_screen,
            inline_mouse,
        }
    }

    /// The terminal as it was before `released`, to be drawn again in
    /// full. A step that fails does not keep the others from being taken.
    pub(crate) fn reclaim(&mut self, released: Released) -> io::Result<()> {
        let raw_mode = match self.input {
            Some(_) => enable_raw_mode(),
            None => Ok(()),
        };
        let backend = self.terminal.backend_mut();
        let paste = execute!(backend, EnableBracketedPaste);
        let _ = execute!(backend, PushKeyboardEnhancementFlags(enhancement_flags()));
        let input = match self.input.as_mut() {
            Some(input) => input.resume(),
            None => Ok(()),
        };
        let screen = match released.alt_screen {
            Some(capture_mouse) => self.enter_alt_screen(capture_mouse),
            None => self
                .terminal
                .clear()
                .and(self.set_inline_mouse_capture(released.inline_mouse)),
        };
        raw_mode.and(paste).and(input).and(screen)
    }
}

#[cfg(test)]
mod tests;
