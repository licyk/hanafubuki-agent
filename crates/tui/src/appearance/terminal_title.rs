// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The terminal's title (OSC 0), written through the interface's terminal
//! when it changes. Its text comes from the session (titles, paths, model
//! names), so it is sanitized first: control characters, which could end
//! or reshape the sequence, and invisible and bidi formatting characters,
//! which could hide or reorder text, are dropped, whitespace runs collapse,
//! and it is bounded. Under GNU Screen, which cuts Unicode in a title to a
//! byte, the whole title is printable ASCII.
//!
//! Codex does not restore the title it set. Here the terminal's title is
//! pushed on its title stack (XTWINOPS 22) before the first write and
//! popped (23) when the interface stops setting it or exits; a terminal
//! without the stack is left with an empty title.

use std::io::{self, Write};

/// Most terminals cut titles a few hundred characters long.
const MAX_TERMINAL_TITLE_CHARS: usize = 240;

const PUSH_TITLE: &str = "\x1b[22;0t";
const POP_TITLE: &str = "\x1b[23;0t";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TitleEncoding {
    Unicode,
    Screen,
}

impl TitleEncoding {
    /// Screen stays an output hop when another multiplexer runs inside it,
    /// and `TERM=screen` is tmux's too: `STY` tells.
    pub(crate) fn detect() -> Self {
        if std::env::var_os("STY").is_some() {
            Self::Screen
        } else {
            Self::Unicode
        }
    }
}

/// The title the interface wrote last, so a title is written only when it
/// changes and the terminal's is put back.
pub(crate) struct TerminalTitle {
    encoding: TitleEncoding,
    written: Option<String>,
}

impl TerminalTitle {
    pub(crate) fn new(encoding: TitleEncoding) -> Self {
        Self {
            encoding,
            written: None,
        }
    }

    /// Writes `title` to `out` when it is not the one written last; a title
    /// with nothing to show gives the terminal its own back.
    pub(crate) fn sync(&mut self, out: &mut impl Write, title: &str) -> io::Result<()> {
        let title = sanitize_terminal_title(title, self.encoding);
        if title.is_empty() {
            return self.restore(out);
        }
        if self.written.as_deref() == Some(title.as_str()) {
            return Ok(());
        }
        if self.written.is_none() {
            out.write_all(PUSH_TITLE.as_bytes())?;
        }
        write!(out, "\x1b]0;{title}\x07")?;
        out.flush()?;
        self.written = Some(title);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn written(&self) -> Option<&str> {
        self.written.as_deref()
    }

    /// Clears the title written and pops the terminal's own back.
    pub(crate) fn restore(&mut self, out: &mut impl Write) -> io::Result<()> {
        if self.written.take().is_none() {
            return Ok(());
        }
        write!(out, "\x1b]0;\x07{POP_TITLE}")?;
        out.flush()
    }
}

/// `title` as one bounded line of visible text.
pub(crate) fn sanitize_terminal_title(title: &str, encoding: TitleEncoding) -> String {
    let mut sanitized = String::new();
    let mut written = 0;
    let mut pending_space = false;
    for ch in title.chars() {
        if ch.is_whitespace() {
            // Leading whitespace is dropped.
            pending_space = !sanitized.is_empty();
            continue;
        }
        if is_disallowed_terminal_title_char(ch) {
            continue;
        }
        let ch = match (encoding, ch) {
            // The spinner keeps turning in ASCII.
            (TitleEncoding::Screen, '⠋' | '⠼' | '⠇') => '|',
            (TitleEncoding::Screen, '⠙' | '⠴' | '⠏') => '/',
            (TitleEncoding::Screen, '⠹' | '⠦') => '-',
            (TitleEncoding::Screen, '⠸' | '⠧') => '\\',
            (TitleEncoding::Screen, ch) if !ch.is_ascii() => '?',
            (_, ch) => ch,
        };
        if pending_space && MAX_TERMINAL_TITLE_CHARS.saturating_sub(written) > 1 {
            sanitized.push(' ');
            written += 1;
            pending_space = false;
        }
        if written >= MAX_TERMINAL_TITLE_CHARS {
            break;
        }
        sanitized.push(ch);
        written += 1;
    }
    sanitized
}

/// Control characters, and the invisible formatting characters that can
/// reorder or hide text (the Trojan Source family).
fn is_disallowed_terminal_title_char(ch: char) -> bool {
    ch.is_control()
        || matches!(
            ch,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{E0100}'..='\u{E01EF}'
        )
}

#[cfg(test)]
mod tests;
