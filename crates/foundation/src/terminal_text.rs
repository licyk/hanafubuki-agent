//! Plain-text rendering of captured terminal output.
//!
//! Captured PTY and pipe output carries colour, title, hyperlink, and cursor
//! control sequences meant for a terminal emulator. A saved log, a clipboard
//! copy, or a command-output excerpt wants the text a person would have read
//! instead. This renders one line at a time: control sequences are dropped,
//! while carriage return, backspace, horizontal cursor movement, and line
//! erasure are applied, so a progress bar that redrew itself leaves only its
//! final state. It is not a screen emulator; vertical cursor movement is
//! ignored and full-screen programs render as the text they wrote, in order.

/// The line being written, addressed by cell like a terminal row.
#[derive(Default)]
struct Line {
    cells: Vec<char>,
    column: usize,
}

impl Line {
    fn put(&mut self, character: char) {
        if self.column < self.cells.len() {
            self.cells[self.column] = character;
        } else {
            self.cells.resize(self.column, ' ');
            self.cells.push(character);
        }
        self.column += 1;
    }

    fn erase(&mut self, mode: usize) {
        match mode {
            0 => self.cells.truncate(self.column),
            1 => {
                let end = self.column.saturating_add(1).min(self.cells.len());
                self.cells[..end].fill(' ');
            }
            2 => self.cells.clear(),
            _ => {}
        }
    }

    fn finish(&mut self, out: &mut String) {
        out.extend(self.cells.drain(..));
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
const MAX_HELD_SEQUENCE_BYTES: usize = 4096;

/// Length of the prefix of `text` that does not end inside an escape sequence.
///
/// Output read in pieces can stop partway through a sequence, and rendering
/// that piece on its own would print the rest of it as text in the next one. A
/// reader passes only this prefix to [`render_plain_text`] and reads the held
/// suffix again with what follows.
pub fn complete_prefix_len(text: &str) -> usize {
    let Some(start) = text.rfind(ESCAPE) else {
        return text.len();
    };
    if text.len() - start > MAX_HELD_SEQUENCE_BYTES {
        return text.len();
    }
    let mut rest = text[start + ESCAPE.len_utf8()..].chars();
    let finished = match rest.next() {
        None => false,
        Some('[') => rest.any(|character| !matches!(character, '0'..='?' | ' '..='/')),
        Some(']' | 'P' | 'X' | '^' | '_') => {
            rest.any(|character| matches!(character, BELL | CANCEL | SUBSTITUTE))
        }
        Some(' '..='/') => rest.any(|character| !matches!(character, ' '..='/')),
        Some(_) => true,
    };
    if finished {
        text.len()
    } else {
        start
    }
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
