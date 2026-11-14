//! Plain-text rendering of captured terminal output.
//!
//! Captured PTY and pipe output carries colour, title, hyperlink, and cursor
//! control sequences meant for a terminal emulator. A saved log, a clipboard
//! copy, or a command-output excerpt wants the text a person would have read
//! instead. This renders one line at a time: control sequences are dropped,
//! while carriage return, backspace, horizontal cursor movement, and line
//! erasure are applied by cell, a wide character taking two, so a progress bar
//! that redrew itself leaves only its final state. It is not a screen
//! emulator; vertical cursor movement is ignored and full-screen programs
//! render as the text they wrote, in order.

mod render;
mod style;

pub use style::{
    sgr_sequence, styled_terminal_text, StyleSpan, TextColor, TextStyle, TextStyles,
    MAX_STYLE_SPANS,
};

/// Render captured terminal output as the plain text it displayed.
///
/// Invalid UTF-8 is replaced, `\r\n` becomes `\n`, tabs are kept, and every
/// other control character and escape sequence is removed. A sequence cut off
/// by the end of the input is dropped.
pub fn render_plain_text(bytes: &[u8]) -> String {
    render::render(bytes, TextStyle::default()).text
}

/// What [`render_styled_text`] made of terminal output.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StyledRender {
    /// The plain text, exactly as [`render_plain_text`] renders it.
    pub text: String,
    /// The styles `text` was written in.
    pub styles: TextStyles,
    /// The style in force at the end, which the output after it starts in.
    pub pen: TextStyle,
}

/// [`render_plain_text`] with the styles its SGR sequences gave the text,
/// for output that starts in style `pen` (the default at a stream's start).
/// A style lives on across lines, as in a terminal; each run ends with its
/// line.
pub fn render_styled_text(bytes: &[u8], pen: TextStyle) -> StyledRender {
    render::render(bytes, pen)
}

/// Longest unfinished sequence [`complete_prefix_len`] holds back; a longer
/// one is treated as complete and rendered (dropped) as it is.
pub const MAX_HELD_SEQUENCE_BYTES: usize = 4096;

/// Length of the prefix of `bytes` that does not end inside an escape
/// sequence.
///
/// Output read in pieces can stop partway through a sequence, and rendering
/// that piece on its own would print the rest of it as text in the next one. A
/// reader passes only this prefix to [`render_plain_text`] and reads the held
/// suffix again with what follows. Every byte that classifies a sequence is
/// ASCII, so this works on output that is not UTF-8, and the length is always
/// a character boundary of output that is.
pub fn complete_prefix_len(bytes: &[u8]) -> usize {
    let Some(start) = bytes.iter().rposition(|byte| *byte == ESCAPE_BYTE) else {
        return bytes.len();
    };
    if bytes.len() - start > MAX_HELD_SEQUENCE_BYTES {
        return bytes.len();
    }
    if sequence_len(&bytes[start..]).is_some() {
        bytes.len()
    } else {
        start
    }
}

/// Where a reader of `bytes` that starts at `cut`, skipping what precedes it,
/// begins so that it renders as the stream would: at `cut` when a line starts
/// there; else just after the first line break among the
/// [`ALIGN_SEARCH_BYTES`] from `cut`; else just after the first complete
/// escape sequence there, since what precedes it may be the rest of a cut one;
/// else at the first character boundary.
pub fn aligned_start(bytes: &[u8], cut: usize) -> usize {
    if cut == 0 || bytes.get(cut - 1) == Some(&b'\n') {
        return cut;
    }
    cut + resume_point(&bytes[cut..])
}

/// [`aligned_start`] for `bytes` whose start is a cut, with nothing before it
/// left to look at.
pub fn resume_point(bytes: &[u8]) -> usize {
    let window = &bytes[..bytes.len().min(ALIGN_SEARCH_BYTES)];
    if let Some(line_break) = window.iter().position(|byte| *byte == b'\n') {
        return line_break + 1;
    }
    let mut index = 0;
    while let Some(found) = window[index..].iter().position(|byte| *byte == ESCAPE_BYTE) {
        let start = index + found;
        match sequence_len(&bytes[start..]) {
            Some(len) if start + len <= window.len() => {
                return char_boundary_from(bytes, start + len);
            }
            _ => index = start + 1,
        }
    }
    char_boundary_from(bytes, 0)
}

/// The first character boundary at or after `index` (at most three bytes on).
fn char_boundary_from(bytes: &[u8], index: usize) -> usize {
    index
        + bytes[index..]
            .iter()
            .take(char::MAX.len_utf8() - 1)
            .take_while(|byte| **byte & 0xC0 == 0x80)
            .count()
}

/// Where a reader that may return at most `max` bytes of `bytes` stops, so it
/// ends as the stream would render up to there: just after the last line
/// break among the [`ALIGN_SEARCH_BYTES`] before `max`; else before an
/// escape sequence `max` would cut, on a character boundary.
pub fn end_point(bytes: &[u8], max: usize) -> usize {
    if max >= bytes.len() {
        return bytes.len();
    }
    let window_start = max.saturating_sub(ALIGN_SEARCH_BYTES);
    if let Some(line_break) = bytes[window_start..max]
        .iter()
        .rposition(|byte| *byte == b'\n')
    {
        return window_start + line_break + 1;
    }
    let mut end = max;
    while end > 0 && bytes[end] & 0xC0 == 0x80 {
        end -= 1;
    }
    complete_prefix_len(&bytes[..end])
}

/// How far a reader looks for a line break, or a sequence, to align on.
pub const ALIGN_SEARCH_BYTES: usize = 1024;

const ESCAPE_BYTE: u8 = 0x1b;

/// The length of the escape sequence `bytes` starts with (at its `ESC`), or
/// `None` while it is unfinished.
fn sequence_len(bytes: &[u8]) -> Option<usize> {
    let rest = &bytes[1..];
    let body = match rest.first()? {
        b'[' => rest[1..]
            .iter()
            .position(|byte| !matches!(byte, b'0'..=b'?' | b' '..=b'/')),
        b']' | b'P' | b'X' | b'^' | b'_' => {
            return control_string_len(&rest[1..]).map(|len| len + 2);
        }
        b' '..=b'/' => rest[1..]
            .iter()
            .position(|byte| !matches!(byte, b' '..=b'/')),
        _ => return Some(2),
    };
    body.map(|len| len + 3)
}

/// The length of a control string's body and terminator: up to `BEL`, `CAN`,
/// `SUB` or `ESC \\`.
fn control_string_len(bytes: &[u8]) -> Option<usize> {
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            0x07 | 0x18 | 0x1a => return Some(index + 1),
            ESCAPE_BYTE if bytes.get(index + 1) == Some(&b'\\') => return Some(index + 2),
            _ => index += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests;
