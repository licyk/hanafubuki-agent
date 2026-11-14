//! A PTY session's output as a terminal would show it.
//!
//! A program in a PTY draws on a grid: ConPTY repaints, progress bars, cursor
//! moves, erases and wide characters put text at a row and a column, which a
//! renderer of the byte stream in order ([`render_plain_text`]) cannot follow.
//! [`TerminalScreen`] replays the output through a `vt100` screen of the PTY's
//! size and gives each reader the rows that scrolled off the top since its
//! last read, then the screen's rows from the first one that changed.
//!
//! Each row comes with its cells' styles (colours, bold, dim, italic,
//! underline, inverse) for a display. The text is the row's whatever its
//! styles, and a row whose styles alone changed is not taken again.
//!
//! `vt100` keeps scrolled-off rows in a bounded scrollback and drops the
//! oldest silently, so the output is fed in slices that cannot overflow it,
//! the new rows are collected after each, and the scrollback is emptied, by
//! replaying the screen's state into a fresh parser, before it fills.
//!
//! [`render_plain_text`]: hanafubuki_agent_foundation::terminal_text::render_plain_text

use hanafubuki_agent_foundation::terminal_text::{
    complete_prefix_len, TextColor, TextStyle, TextStyles, MAX_HELD_SEQUENCE_BYTES,
};
use hanafubuki_agent_process::terminal_size::{TerminalResize, TerminalSize};

use crate::model::AgentExecutionOutput;

/// Bytes fed between two collections of scrolled-off rows. One byte scrolls
/// at most one row, so a slice adds at most this many to the scrollback.
const SLICE_BYTES: usize = 128;
/// Rows collected before the scrollback is emptied.
const EMPTY_AT_ROWS: usize = 256;
/// Room for the rows a slice adds on top of [`EMPTY_AT_ROWS`], and for the
/// slices fed while an unfinished sequence postpones the emptying.
const SCROLLBACK_ROWS: usize = 1024;

pub struct TerminalScreen {
    parser: vt100::Parser,
    /// Rows the scrollback holds that were already collected.
    collected: usize,
    /// Rows scrolled off since the last [`Self::take`], oldest first, each
    /// with whether it continues on the next row.
    scrolled: Vec<(StyledRow, bool)>,
    /// Rows ever scrolled off: the absolute index of the screen's top row.
    top: u64,
    /// The screen's rows at the last take, from its top row then.
    shown_top: u64,
    shown: Vec<String>,
    /// The absolute row the last take ended on without a line break.
    unfinished_row: Option<u64>,
    /// The end of the output fed, while it is inside an escape sequence.
    held: Vec<u8>,
}

/// What a [`TerminalScreen::take`] returns.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ScreenText {
    /// The text begins with the row the previous take ended on without a line
    /// break, as it is now: a reader that joins takes replaces that line.
    pub rewrites_last_line: bool,
    /// Rows, each ended by a line break unless the next row continues it or
    /// the cursor is still on it.
    pub text: String,
    /// The styles of `text`'s cells.
    pub styles: TextStyles,
}

/// A row's text, its trailing blanks trimmed, and its cells' styles.
struct StyledRow {
    text: String,
    styles: TextStyles,
}

impl TerminalScreen {
    pub fn new(size: TerminalSize) -> Self {
        Self {
            parser: vt100::Parser::new(size.rows.max(1), size.cols.max(1), SCROLLBACK_ROWS),
            collected: 0,
            scrolled: Vec::new(),
            top: 0,
            shown_top: 0,
            shown: Vec::new(),
            unfinished_row: None,
            held: Vec::new(),
        }
    }

    /// A screen for `output` when it is a PTY's.
    pub fn for_output(output: &AgentExecutionOutput) -> Option<Self> {
        output.terminal_size.map(Self::new)
    }

    /// Feed `output` at the sizes it was produced at.
    pub fn push_output(&mut self, output: &AgentExecutionOutput) {
        let bytes = output
            .chunks
            .iter()
            .flat_map(|chunk| chunk.text.as_bytes())
            .copied()
            .collect::<Vec<_>>();
        self.push_sized(
            &bytes,
            output.start_offset,
            output.terminal_size,
            &output.resizes,
        );
    }

    /// Feed `bytes`, the output from `start_offset` on, produced at `size`
    /// (when known) and at each of `resizes` from its offset on.
    pub fn push_sized(
        &mut self,
        bytes: &[u8],
        start_offset: u64,
        size: Option<TerminalSize>,
        resizes: &[TerminalResize],
    ) {
        if let Some(size) = size {
            self.resize(size);
        }
        let mut fed = 0;
        for resize in resizes {
            let at = resize.offset.saturating_sub(start_offset) as usize;
            if at > bytes.len() {
                break;
            }
            if at > fed {
                self.push(&bytes[fed..at]);
                fed = at;
            }
            self.resize(resize.size);
        }
        self.push(&bytes[fed..]);
    }

    pub fn resize(&mut self, size: TerminalSize) {
        let screen = self.parser.screen_mut();
        if screen.size() != (size.rows.max(1), size.cols.max(1)) {
            screen.set_size(size.rows.max(1), size.cols.max(1));
        }
    }

    /// Feed `bytes`. An escape sequence they end inside is held until the
    /// rest of it arrives, so the parser is between sequences after every
    /// push.
    pub fn push(&mut self, bytes: &[u8]) {
        let mut data = std::mem::take(&mut self.held);
        data.extend_from_slice(bytes);
        let complete = complete_prefix_len(&data);
        self.held = data.split_off(complete);
        let mut fed = 0;
        while fed < data.len() {
            let end = (fed + SLICE_BYTES).min(data.len());
            self.parser.process(&data[fed..end]);
            fed = end;
            self.collect_scrolled();
            if self.collected >= EMPTY_AT_ROWS
                && !self.parser.screen().alternate_screen()
                && between_sequences(&data, fed)
            {
                self.empty_scrollback();
            }
        }
    }

    /// The rows scrolled off since the last take, then the screen's rows from
    /// the first one that changed through the last one with text.
    pub fn take(&mut self) -> ScreenText {
        let screen = self.parser.screen();
        let (_, cols) = screen.size();
        let scrolled = self.scrolled.len();
        let mut rows = std::mem::take(&mut self.scrolled);
        rows.extend(screen.rows(0, cols).enumerate().map(|(index, text)| {
            let row = StyledRow {
                text: text.trim_end_matches(' ').to_owned(),
                styles: TextStyles::default(),
            };
            (row, screen.row_wrapped(index as u16))
        }));
        let cursor = scrolled + usize::from(screen.cursor_position().0);
        let end = rows
            .iter()
            .rposition(|(row, _)| !row.text.is_empty())
            .map_or(0, |last| last + 1);
        // Compared by text: a row whose styles alone changed is not taken.
        let mut start = rows
            .iter()
            .zip(&self.shown)
            .position(|((row, _), shown)| row.text != *shown)
            .unwrap_or(self.shown.len().min(rows.len()));

        let mut taken = ScreenText::default();
        if start < end {
            if let Some(row) = self.unfinished_row {
                start = start.min(row.saturating_sub(self.shown_top) as usize);
                taken.rewrites_last_line = true;
            }
            for (index, (row, wrapped)) in rows.iter_mut().enumerate().take(end).skip(start) {
                if index >= scrolled {
                    row.styles = row_styles(screen, (index - scrolled) as u16, &row.text);
                }
                taken.text.push_str(&row.text);
                taken.styles.append(&row.styles);
                let ended = index + 1 < end || cursor > index;
                if ended && !*wrapped {
                    taken.text.push('\n');
                    taken.styles.extend_plain(1);
                }
            }
            let last = end - 1;
            self.unfinished_row = (cursor <= last).then_some(self.shown_top + last as u64);
        }
        self.shown_top = self.top;
        self.shown = rows
            .drain(scrolled.min(end)..end)
            .map(|(row, _)| row.text)
            .collect();
        taken
    }

    /// Collect the rows the scrollback gained.
    fn collect_scrolled(&mut self) {
        let screen = self.parser.screen_mut();
        // The alternate screen has no scrollback of its own, and the main
        // one's is left as it was.
        if screen.alternate_screen() {
            return;
        }
        screen.set_scrollback(usize::MAX);
        let held = screen.scrollback();
        let (_, cols) = screen.size();
        for index in self.collected..held {
            screen.set_scrollback(held - index);
            let text = screen.rows(0, cols).next().unwrap_or_default();
            let text = text.trim_end_matches(' ').to_owned();
            let styles = row_styles(screen, 0, &text);
            let wrapped = screen.row_wrapped(0);
            self.scrolled.push((StyledRow { text, styles }, wrapped));
        }
        screen.set_scrollback(0);
        self.top += held.saturating_sub(self.collected) as u64;
        self.collected = held;
    }

    fn empty_scrollback(&mut self) {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        let mut parser = vt100::Parser::new(rows, cols, SCROLLBACK_ROWS);
        parser.process(&screen.state_formatted());
        parser.screen_mut().set_scrollback(usize::MAX);
        self.collected = parser.screen().scrollback();
        parser.screen_mut().set_scrollback(0);
        self.parser = parser;
    }
}

/// The styles of the cells of the screen's row `row`, whose text (as
/// `vt100` writes a row, its trailing blanks trimmed) is `text`. Cells that
/// spell another text give none, so a style never lands on the wrong text.
fn row_styles(screen: &vt100::Screen, row: u16, text: &str) -> TextStyles {
    let (_, cols) = screen.size();
    let mut spelled = String::with_capacity(text.len());
    let mut styles = TextStyles::default();
    // The column after the last cell written: empty cells before the next
    // one with contents show as blanks.
    let mut written = 0;
    let mut column = 0;
    while column < cols {
        let Some(cell) = screen.cell(row, column) else {
            break;
        };
        if cell.has_contents() {
            spelled.extend(std::iter::repeat_n(' ', usize::from(column - written)));
            let start = spelled.len();
            spelled.push_str(cell.contents());
            styles.push(start, spelled.len(), cell_style(cell));
            written = column + if cell.is_wide() { 2 } else { 1 };
        }
        column += if cell.is_wide() { 2 } else { 1 };
    }
    styles.text_bytes = text.len();
    if spelled.trim_end_matches(' ') != text {
        styles.spans.clear();
        return styles;
    }
    // Styled blanks past the text were trimmed with it.
    styles.spans.retain_mut(|span| {
        span.end = span.end.min(text.len());
        span.start < span.end
    });
    styles
}

fn cell_style(cell: &vt100::Cell) -> TextStyle {
    TextStyle {
        fg: color(cell.fgcolor()),
        bg: color(cell.bgcolor()),
        bold: cell.bold(),
        dim: cell.dim(),
        italic: cell.italic(),
        underline: cell.underline(),
        reverse: cell.inverse(),
    }
}

fn color(color: vt100::Color) -> Option<TextColor> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Idx(index) => Some(TextColor::Indexed(index)),
        vt100::Color::Rgb(r, g, b) => Some(TextColor::Rgb(r, g, b)),
    }
}

/// Whether the parser, fed `data[..end]` from a point between sequences, is
/// between sequences again: `end` is a character boundary outside any escape
/// sequence.
fn between_sequences(data: &[u8], end: usize) -> bool {
    // A sequence held longer than this counts as complete anyway.
    let fed = &data[end.saturating_sub(MAX_HELD_SEQUENCE_BYTES + 1)..end];
    data.get(end).is_none_or(|byte| byte & 0xC0 != 0x80) && complete_prefix_len(fed) == fed.len()
}

#[cfg(test)]
mod tests;
