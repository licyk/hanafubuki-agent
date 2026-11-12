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

use unicode_width::UnicodeWidthChar;

/// One cell of a [`Line`].
#[derive(Clone)]
enum Cell {
    /// A character, the zero-width ones written after it, and whether it takes
    /// two cells (the next one is then its [`Cell::Continuation`]).
    Glyph {
        character: char,
        marks: Vec<char>,
        wide: bool,
    },
    /// The second cell of a wide character.
    Continuation,
}

const BLANK: Cell = Cell::Glyph {
    character: ' ',
    marks: Vec::new(),
    wide: false,
};

/// The line being written, addressed by cell like a terminal row.
#[derive(Default)]
struct Line {
    cells: Vec<Cell>,
    column: usize,
}

impl Line {
    fn put(&mut self, character: char) {
        let width = if character == '\t' {
            1
        } else {
            character.width().unwrap_or(0)
        };
        if width == 0 {
            self.mark(character);
            return;
        }
        let wide = width > 1;
        self.overwrite(self.column);
        if wide {
            self.overwrite(self.column + 1);
        }
        let end = self.column + width;
        if self.cells.len() < end {
            self.cells.resize(end, BLANK);
        }
        self.cells[self.column] = Cell::Glyph {
            character,
            marks: Vec::new(),
            wide,
        };
        if wide {
            self.cells[self.column + 1] = Cell::Continuation;
        }
        self.column = end;
    }

    /// Attach a zero-width character to the one before the cursor.
    fn mark(&mut self, character: char) {
        let mut index = self.column.min(self.cells.len());
        while index > 0 {
            index -= 1;
            if let Cell::Glyph { marks, .. } = &mut self.cells[index] {
                marks.push(character);
                return;
            }
        }
    }

    /// Blank the other half of a wide character the cell at `column` is part
    /// of, before that cell is written or erased.
    fn overwrite(&mut self, column: usize) {
        match self.cells.get(column) {
            Some(Cell::Continuation) => self.cells[column - 1] = BLANK,
            Some(Cell::Glyph { wide: true, .. }) => {
                if let Some(next) = self.cells.get_mut(column + 1) {
                    *next = BLANK;
                }
            }
            _ => {}
        }
    }

    fn erase(&mut self, mode: usize) {
        match mode {
            0 => {
                self.overwrite(self.column);
                self.cells.truncate(self.column);
            }
            1 => {
                self.overwrite(self.column);
                let end = self.column.saturating_add(1).min(self.cells.len());
                self.cells[..end].fill(BLANK);
            }
            2 => self.cells.clear(),
            _ => {}
        }
    }

    fn finish(&mut self, out: &mut String) {
        for cell in self.cells.drain(..) {
            if let Cell::Glyph {
                character, marks, ..
            } = cell
            {
                out.push(character);
                out.extend(marks);
            }
        }
        self.column = 0;
    }
}

enum State {
    Ground,
    /// After `ESC`.
    Escape,
    /// After `ESC` and an intermediate byte, awaiting the final byte.
    EscapeIntermediate,
    /// Inside `ESC [`, collecting the parameter bytes.
    Csi {
        parameters: String,
    },
    /// Inside an OSC, DCS, APC, PM, or SOS string.
    ControlString,
    /// After `ESC` inside a control string, which `\` completes as ST.
    ControlStringEscape,
}

const ESCAPE: char = '\u{1b}';
const BELL: char = '\u{07}';
const BACKSPACE: char = '\u{08}';
const CANCEL: char = '\u{18}';
const SUBSTITUTE: char = '\u{1a}';
/// Longest CSI parameter text retained; a longer one is still consumed.
const MAX_CSI_PARAMETER_CHARS: usize = 32;

/// Render captured terminal output as the plain text it displayed.
///
/// Invalid UTF-8 is replaced, `\r\n` becomes `\n`, tabs are kept, and every
/// other control character and escape sequence is removed. A sequence cut off
/// by the end of the input is dropped.
pub fn render_plain_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(text.len());
    let mut line = Line::default();
    let mut state = State::Ground;

    for character in text.chars() {
        state = match state {
            State::Ground => ground(character, &mut line, &mut out),
            State::Escape => match character {
                '[' => State::Csi {
                    parameters: String::new(),
                },
                ']' | 'P' | 'X' | '^' | '_' => State::ControlString,
                ' '..='/' => State::EscapeIntermediate,
                ESCAPE => State::Escape,
                _ => State::Ground,
            },
            State::EscapeIntermediate => match character {
                ' '..='/' => State::EscapeIntermediate,
                ESCAPE => State::Escape,
                _ => State::Ground,
            },
            State::Csi { mut parameters } => match character {
                '0'..='?' => {
                    if parameters.len() < MAX_CSI_PARAMETER_CHARS {
                        parameters.push(character);
                    }
                    State::Csi { parameters }
                }
                ' '..='/' => State::Csi { parameters },
                '@'..='~' => {
                    apply_csi(&parameters, character, &mut line);
                    State::Ground
                }
                // Anything else ends the malformed sequence and is handled as
                // ordinary output, so a stray ESC [ cannot swallow a newline.
                _ => ground(character, &mut line, &mut out),
            },
            State::ControlString => match character {
                BELL | CANCEL | SUBSTITUTE => State::Ground,
                ESCAPE => State::ControlStringEscape,
                _ => State::ControlString,
            },
            State::ControlStringEscape => match character {
                '\\' => State::Ground,
                '[' => State::Csi {
                    parameters: String::new(),
                },
                ESCAPE => State::ControlStringEscape,
                _ => State::ControlString,
            },
        };
    }

    line.finish(&mut out);
    out
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

fn ground(character: char, line: &mut Line, out: &mut String) -> State {
    match character {
        ESCAPE => return State::Escape,
        '\n' => {
            line.finish(out);
            out.push('\n');
        }
        '\r' => line.column = 0,
        BACKSPACE => line.column = line.column.saturating_sub(1),
        '\t' => line.put('\t'),
        character if character.is_control() => {}
        character => line.put(character),
    }
    State::Ground
}

fn apply_csi(parameters: &str, final_byte: char, line: &mut Line) {
    // Private sequences (`ESC [ ? 25 l`) never move the cursor or erase text.
    if parameters.starts_with(['<', '=', '>', '?']) {
        return;
    }
    let first = parameters.split([';', ':']).next().unwrap_or("");
    let Ok(value) = (if first.is_empty() {
        Ok(None)
    } else {
        first.parse::<usize>().map(Some)
    }) else {
        return;
    };
    let count = value.filter(|value| *value > 0).unwrap_or(1);
    match final_byte {
        'K' => line.erase(value.unwrap_or(0)),
        'C' => line.column = line.column.saturating_add(count).min(MAX_COLUMN),
        'D' => line.column = line.column.saturating_sub(count),
        'G' | '`' => line.column = (count - 1).min(MAX_COLUMN),
        _ => {}
    }
}

/// Bound on forward cursor movement, so a forged count cannot pad a line
/// without limit.
const MAX_COLUMN: usize = 4096;

#[cfg(test)]
mod tests;
