// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The OSC 8 sequences, where text reaches the terminal: around a row's
//! linked text for the scrollback writer, and inside each linked cell of a
//! frame's buffer, whose drawing joins a run of them into one link (see
//! `custom_terminal`). Every link opened is closed on the same row.

use std::num::NonZeroU16;

use ratatui::buffer::{Buffer, CellDiffOption, CellWidth};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;

use super::{HyperlinkLine, TerminalHyperlink};
use crate::wrapping::display_width;

pub(crate) const OSC8_CLOSE: &str = "\x1b]8;;\x07";

fn is_terminal_destination(destination: &str) -> bool {
    destination.starts_with("https://") || destination.starts_with("http://")
}

pub(crate) fn osc8_open(destination: &str) -> String {
    format!("\x1b]8;;{destination}\x07")
}

/// The row's spans, ready to print: control characters left out of the
/// text, as ratatui leaves them out of a buffer, and each link's text
/// between its opening and closing sequences, the only escapes added.
pub(crate) fn decorate_spans(line: &HyperlinkLine) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut column = 0usize;
    let mut link_index = 0usize;
    let mut active: Option<usize> = None;
    for span in &line.line.spans {
        for grapheme in span.content.graphemes(true) {
            let width = display_width(grapheme);
            if grapheme.contains(char::is_control) {
                column += width;
                continue;
            }
            while line
                .hyperlinks
                .get(link_index)
                .is_some_and(|link| link.columns.end <= column)
            {
                link_index += 1;
            }
            let selected = line.hyperlinks.get(link_index).and_then(|link| {
                (link.columns.contains(&column) && is_terminal_destination(&link.destination))
                    .then_some(link_index)
            });
            if active != selected {
                if active.is_some() {
                    append_to_last_span(&mut out, OSC8_CLOSE);
                }
                if let Some(index) = selected {
                    push_styled_content(
                        &mut out,
                        &osc8_open(&line.hyperlinks[index].destination),
                        span.style,
                    );
                }
                active = selected;
            }
            push_styled_content(&mut out, grapheme, span.style);
            column += width;
        }
    }
    if active.is_some() {
        append_to_last_span(&mut out, OSC8_CLOSE);
    }
    out
}

fn push_styled_content(out: &mut Vec<Span<'static>>, content: &str, style: Style) {
    if let Some(last) = out.last_mut() {
        if last.style == style {
            last.content.to_mut().push_str(content);
            return;
        }
    }
    out.push(Span::styled(content.to_string(), style));
}

fn append_to_last_span(out: &mut [Span<'static>], content: &str) {
    if let Some(last) = out.last_mut() {
        last.content.to_mut().push_str(content);
    }
}

/// Each painted grapheme of one row's `row` cells linked to `destination`,
/// which is sanitised already: the cell keeps its width for the diff, its
/// symbol carries the link.
pub(crate) fn mark_hyperlink_cells(buf: &mut Buffer, row: Rect, destination: &str) {
    if !is_terminal_destination(destination) {
        return;
    }
    let row = row.intersection(buf.area);
    let mut x = row.left();
    while x < row.right() {
        let cell = &mut buf[(x, row.y)];
        if cell.diff_option == CellDiffOption::Skip {
            x += 1;
            continue;
        }
        let width = NonZeroU16::new(cell.cell_width()).unwrap_or(NonZeroU16::MIN);
        x = x.saturating_add(width.get());
        let symbol = format!("{}{}{OSC8_CLOSE}", osc8_open(destination), cell.symbol());
        cell.set_symbol(&symbol)
            .set_diff_option(CellDiffOption::ForcedWidth(width));
    }
}

/// The destination and the visible symbol of a linked cell's symbol.
/// The destination and the visible symbol of a linked cell's symbol.
/// Paints one row of a line, a grapheme at a time, so every grapheme takes
/// the columns [`display_width`] gives it in one cell that says so
/// (`ForcedWidth`).
///
/// ratatui's own `set_line` measures a grapheme by `CellWidth`, which counts a
/// halfwidth sound mark twice under `unicode-width` 0.2.3 (as the mark's own
/// column and as ratatui's compensation for when it had none), and then blanks
/// the cells after it — the row and its links would disagree about where each
/// character sits. Painted here, the layout, the links and the terminal agree.
fn paint(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    line: &Line<'_>,
    max_width: u16,
    hyperlinks: &[TerminalHyperlink],
) {
    let right = (x.saturating_add(max_width)).min(buf.area.right());
    let mut column = 0usize;
    let mut link_index = 0usize;
    for span in line {
        let style = line.style.patch(span.style);
        for grapheme in span.content.graphemes(true) {
            let width = display_width(grapheme);
            let start = column;
            column += width;
            if width == 0 || grapheme.contains(char::is_control) {
                continue;
            }
            let at = x.saturating_add(u16::try_from(start).unwrap_or(u16::MAX));
            if at >= right {
                return;
            }
            while hyperlinks
                .get(link_index)
                .is_some_and(|link| link.columns.end <= start)
            {
                link_index += 1;
            }
            let cell = &mut buf[(at, y)];
            cell.set_symbol(grapheme).set_style(style);
            if let Some(link) = hyperlinks.get(link_index) {
                if link.columns.contains(&start) && is_terminal_destination(&link.destination) {
                    let symbol = format!(
                        "{}{}{OSC8_CLOSE}",
                        osc8_open(&link.destination),
                        cell.symbol()
                    );
                    cell.set_symbol(&symbol);
                }
            }
            cell.set_diff_option(CellDiffOption::ForcedWidth(
                NonZeroU16::new(u16::try_from(width).unwrap_or(u16::MAX))
                    .unwrap_or(NonZeroU16::MIN),
            ));
        }
    }
}

/// Paints one row of a line as [`paint`] does, with its links.
pub(crate) fn paint_row(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    line: &Line<'_>,
    max_width: u16,
    hyperlinks: &[TerminalHyperlink],
) {
    paint(buf, x, y, line, max_width, hyperlinks);
}

/// Paints one row of a line that carries no links of its own, so that the
/// columns a grapheme takes are the same everywhere on screen.
pub(crate) fn paint_line(buf: &mut Buffer, x: u16, y: u16, line: &Line<'_>, max_width: u16) {
    paint(buf, x, y, line, max_width, &[]);
}
pub(crate) fn osc8_parts(symbol: &str) -> Option<(&str, &str)> {
    let rest = symbol.strip_prefix("\x1b]8;;")?;
    let (destination, rest) = rest.split_once('\x07')?;
    let visible = rest.strip_suffix(OSC8_CLOSE)?;
    Some((destination, visible))
}
