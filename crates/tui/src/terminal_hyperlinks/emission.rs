// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The OSC 8 sequences, where text reaches the terminal: around a row's
//! linked text for the scrollback writer, and inside each linked cell of a
//! frame's buffer, whose drawing joins a run of them into one link (see
//! `custom_terminal`). Every link opened is closed on the same row.

use std::num::NonZeroU16;

use ratatui::buffer::{Buffer, CellDiffOption, CellWidth};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use unicode_segmentation::UnicodeSegmentation;

use super::HyperlinkLine;
use crate::wrapping::display_width;

pub(crate) const OSC8_CLOSE: &str = "\x1b]8;;\x07";

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
            let selected = line
                .hyperlinks
                .get(link_index)
                .and_then(|link| link.columns.contains(&column).then_some(link_index));
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
pub(crate) fn osc8_parts(symbol: &str) -> Option<(&str, &str)> {
    let rest = symbol.strip_prefix("\x1b]8;;")?;
    let (destination, rest) = rest.split_once('\x07')?;
    let visible = rest.strip_suffix(OSC8_CLOSE)?;
    Some((destination, visible))
}
