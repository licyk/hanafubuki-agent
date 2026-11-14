// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! One diff row laid out at a width: its line number right-aligned in the
//! gutter, its sign, then its content hard-wrapped by display width with
//! every span's style kept across the cut, continuation rows indented past
//! the gutter and the sign. The row's tint is the line's style, so it
//! reaches the right edge of every row it takes.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;

use super::style::{
    style_add, style_context, style_del, style_gutter_for, style_line_bg_for, style_sign_add,
    style_sign_del, DiffLineType, DiffRenderStyleContext,
};
use crate::color::readable_color_on;
use crate::wrapping::display_width;

/// What a tab becomes in a row.
const TAB_REPLACEMENT: &str = "    ";
const TAB_WIDTH: usize = TAB_REPLACEMENT.len();

/// One diff row at `width` columns. `syntax_spans` are its content
/// highlighted, else `text` takes the row's colour; `gutter` is its line
/// number and the gutter's columns, when its number is known.
pub(crate) fn push_wrapped_diff_line(
    kind: DiffLineType,
    text: &str,
    syntax_spans: Option<&[Span<'static>]>,
    gutter: Option<(u64, usize)>,
    width: usize,
    style: DiffRenderStyleContext,
) -> Vec<Line<'static>> {
    let (sign, sign_style, content_style) = match kind {
        DiffLineType::Insert => ('+', style_sign_add(style), style_add(style)),
        DiffLineType::Delete => ('-', style_sign_del(style), style_del(style)),
        DiffLineType::Context => (' ', style_context(), style_context()),
    };
    let line_style = style_line_bg_for(kind, style.diff_backgrounds);
    let level = style.stdout_level();
    // Syntax colours are made readable on the row's tint; the tint and the
    // sign tell an addition from a removal, so nothing is dimmed.
    let content: Vec<Span<'static>> = match syntax_spans {
        Some(spans) => spans
            .iter()
            .map(|span| {
                let fg =
                    readable_color_on(span.style.fg.unwrap_or(Color::Reset), line_style.bg, level);
                Span::styled(
                    span.content.clone().into_owned(),
                    span.style.fg(fg).remove_modifier(Modifier::DIM),
                )
            })
            .collect(),
        None => {
            let fg = readable_color_on(
                content_style.fg.unwrap_or(Color::Reset),
                line_style.bg,
                level,
            );
            vec![Span::styled(text.to_string(), content_style.fg(fg))]
        }
    };
    let mut gutter_style = style_gutter_for(kind, style.theme, style.color_level);
    if line_style.bg.is_none() {
        gutter_style.bg = None;
    }
    let gutter_columns = gutter.map_or(0, |(_, columns)| columns.max(1) + 1);
    let continuation = Span::styled(" ".repeat(gutter_columns + 1), gutter_style);
    let available = width.saturating_sub(gutter_columns + 1).max(1);
    wrap_styled_spans(&content, available)
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            let mut spans = Vec::with_capacity(chunk.len() + 2);
            if index == 0 {
                if let Some((number, columns)) = gutter {
                    let columns = columns.max(1);
                    spans.push(Span::styled(format!("{number:>columns$} "), gutter_style));
                }
                spans.push(Span::styled(sign.to_string(), sign_style));
            } else {
                spans.push(continuation.clone());
            }
            spans.extend(chunk);
            Line::from(spans).style(line_style)
        })
        .collect()
}

/// `spans` cut into rows of at most `max_cols` columns, by grapheme and
/// display width (a tab four columns), each piece keeping its span's style.
/// A grapheme wider than the room left starts the next row; one wider than
/// a whole row takes a row of its own, so every row makes progress.
pub(crate) fn wrap_styled_spans(
    spans: &[Span<'static>],
    max_cols: usize,
) -> Vec<Vec<Span<'static>>> {
    let mut result: Vec<Vec<Span<'static>>> = Vec::new();
    let mut current_line: Vec<Span<'static>> = Vec::new();
    let mut col: usize = 0;

    for span in spans {
        let style = span.style;
        let mut remaining = span.content.as_ref();
        while !remaining.is_empty() {
            let mut byte_end = 0;
            let mut chars_col = 0;
            for grapheme in remaining.graphemes(true) {
                let w = grapheme_width(grapheme);
                if col + chars_col + w > max_cols {
                    break;
                }
                byte_end += grapheme.len();
                chars_col += w;
            }

            if byte_end == 0 {
                // Wider than the room left: onto a row of its own.
                if !current_line.is_empty() {
                    result.push(std::mem::take(&mut current_line));
                }
                let Some(grapheme) = remaining.graphemes(true).next() else {
                    break;
                };
                current_line.push(styled_piece(grapheme, style));
                // At least one column, so a zero-width grapheme still
                // advances.
                col = grapheme_width(grapheme).max(1);
                remaining = &remaining[grapheme.len()..];
                continue;
            }

            let (chunk, rest) = remaining.split_at(byte_end);
            current_line.push(styled_piece(chunk, style));
            col += chars_col;
            remaining = rest;
            // A full row ends here; the next span starts a new one.
            if col >= max_cols {
                result.push(std::mem::take(&mut current_line));
                col = 0;
            }
        }
    }

    if !current_line.is_empty() || result.is_empty() {
        result.push(current_line);
    }
    result
}

fn grapheme_width(grapheme: &str) -> usize {
    if grapheme == "\t" {
        TAB_WIDTH
    } else {
        display_width(grapheme)
    }
}

fn styled_piece(text: &str, style: Style) -> Span<'static> {
    Span::styled(text.replace('\t', TAB_REPLACEMENT), style)
}

/// The columns of a right-aligned line number up to `max_line_number`.
pub(crate) fn line_number_width(max_line_number: u64) -> usize {
    if max_line_number == 0 {
        1
    } else {
        max_line_number.to_string().len()
    }
}
