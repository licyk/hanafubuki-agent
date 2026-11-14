//! The renderer: a line addressed by cell like a terminal row, each cell
//! with the style it was written in, and the escape-sequence state machine
//! that writes it.

use unicode_width::UnicodeWidthChar;

use super::style::{apply_sgr, TextStyle, TextStyles};
use super::StyledRender;

/// One cell of a [`Line`].
#[derive(Clone)]
enum Cell {
    /// A character, the zero-width ones written after it, whether it takes
    /// two cells (the next one is then its [`Cell::Continuation`]), and its
    /// style.
    Glyph {
        character: char,
        marks: Vec<char>,
        wide: bool,
        style: TextStyle,
    },
    /// The second cell of a wide character.
    Continuation,
}

const BLANK: Cell = Cell::Glyph {
    character: ' ',
    marks: Vec::new(),
    wide: false,
    style: TextStyle {
        fg: None,
        bg: None,
        bold: false,
        dim: false,
        italic: false,
        underline: false,
        reverse: false,
    },
};

/// The line being written, addressed by cell like a terminal row.
#[derive(Default)]
struct Line {
    cells: Vec<Cell>,
    column: usize,
}

impl Line {
    fn put(&mut self, character: char, style: TextStyle) {
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
            style,
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

    fn finish(&mut self, out: &mut StyledRender) {
        for cell in self.cells.drain(..) {
            if let Cell::Glyph {
                character,
                marks,
                style,
                ..
            } = cell
            {
                let start = out.text.len();
                out.text.push(character);
                out.text.extend(marks);
                out.styles.push(start, out.text.len(), style);
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
    /// Inside `ESC [`, collecting the parameter bytes; `kept` is false once
    /// they ran past what is kept, or an intermediate byte came.
    Csi {
        parameters: String,
        kept: bool,
    },
    /// Inside an OSC, DCS, APC, PM, or SOS string.
    ControlString,
    /// After `ESC` inside a control string, which `\` completes as ST.
    ControlStringEscape,
}

impl State {
    fn csi() -> Self {
        State::Csi {
            parameters: String::new(),
            kept: true,
        }
    }
}

const ESCAPE: char = '\u{1b}';
const BELL: char = '\u{07}';
const BACKSPACE: char = '\u{08}';
const CANCEL: char = '\u{18}';
const SUBSTITUTE: char = '\u{1a}';
/// The CSI parameter text cursor movement and erasure read; a longer one is
/// still consumed.
const MAX_CSI_PARAMETER_CHARS: usize = 32;
/// The parameter text an SGR sequence may have; a longer one (or one with
/// an intermediate byte) is consumed and changes no style.
const MAX_SGR_PARAMETER_CHARS: usize = 128;

/// Render `bytes`, which start in style `pen` (see the module's docs).
pub(super) fn render(bytes: &[u8], pen: TextStyle) -> StyledRender {
    let text = String::from_utf8_lossy(bytes);
    let mut out = StyledRender {
        text: String::with_capacity(text.len()),
        styles: TextStyles::default(),
        pen,
    };
    let mut line = Line::default();
    let mut state = State::Ground;

    for character in text.chars() {
        state = match state {
            State::Ground => ground(character, &mut line, &mut out),
            State::Escape => match character {
                '[' => State::csi(),
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
            State::Csi {
                mut parameters,
                mut kept,
            } => match character {
                '0'..='?' => {
                    if parameters.len() < MAX_SGR_PARAMETER_CHARS {
                        parameters.push(character);
                    } else {
                        kept = false;
                    }
                    State::Csi { parameters, kept }
                }
                ' '..='/' => State::Csi {
                    parameters,
                    kept: false,
                },
                '@'..='~' => {
                    if character == 'm' {
                        if kept && !is_private(&parameters) {
                            apply_sgr(&mut out.pen, &parameters);
                        }
                    } else {
                        let read = parameters.len().min(MAX_CSI_PARAMETER_CHARS);
                        apply_csi(&parameters[..read], character, &mut line);
                    }
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
                '[' => State::csi(),
                ESCAPE => State::ControlStringEscape,
                _ => State::ControlString,
            },
        };
    }

    line.finish(&mut out);
    out.styles.text_bytes = out.text.len();
    out
}

fn ground(character: char, line: &mut Line, out: &mut StyledRender) -> State {
    match character {
        ESCAPE => return State::Escape,
        '\n' => {
            line.finish(out);
            out.text.push('\n');
        }
        '\r' => line.column = 0,
        BACKSPACE => line.column = line.column.saturating_sub(1),
        '\t' => line.put('\t', out.pen),
        character if character.is_control() => {}
        character => line.put(character, out.pen),
    }
    State::Ground
}

/// Private sequences (`ESC [ ? 25 l`, `ESC [ > 4 ; 2 m`) never move the
/// cursor, erase text or set a style.
fn is_private(parameters: &str) -> bool {
    parameters.starts_with(['<', '=', '>', '?'])
}

fn apply_csi(parameters: &str, final_byte: char, line: &mut Line) {
    if is_private(parameters) {
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
