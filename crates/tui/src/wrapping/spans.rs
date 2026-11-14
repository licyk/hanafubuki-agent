// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A styled line as one text and back: its spans flattened with their byte
//! ranges, a byte range of the text sliced back into styled spans, and the
//! columns a text takes in a terminal.

use std::borrow::Cow;
use std::ops::Range;

use ratatui::buffer::CellWidth;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// The columns `text` takes as ratatui draws it: Unicode's width, with the
/// halfwidth sound marks (`ﾞ`, `ﾟ`) a column each, as ratatui gives them.
pub(crate) fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
        + text
            .chars()
            .filter(|ch| matches!(ch, '\u{FF9E}' | '\u{FF9F}'))
            .count()
}

/// The columns a line takes as ratatui paints it, a grapheme at a time.
pub(crate) fn line_width(line: &Line<'_>) -> usize {
    line.iter()
        .flat_map(|span| span.styled_graphemes(Style::default()))
        .map(|grapheme| usize::from(grapheme.symbol.cell_width()))
        .sum()
}

pub(super) fn flatten_line(line: &Line<'_>) -> (String, Vec<(Range<usize>, Style)>) {
    let mut flat = String::new();
    let mut span_bounds = Vec::new();
    let mut acc = 0usize;
    for span in &line.spans {
        let text = span.content.as_ref();
        let start = acc;
        flat.push_str(text);
        acc += text.len();
        span_bounds.push((start..acc, span.style));
    }
    (flat, span_bounds)
}

pub(super) fn slice_line_spans<'a>(
    original: &'a Line<'a>,
    span_bounds: &[(Range<usize>, Style)],
    range: &Range<usize>,
) -> Line<'a> {
    let start_byte = range.start;
    let end_byte = range.end;
    let mut acc: Vec<Span<'a>> = Vec::new();
    for (i, (range, style)) in span_bounds.iter().enumerate() {
        let s = range.start;
        let e = range.end;
        if e <= start_byte {
            continue;
        }
        if s >= end_byte {
            break;
        }
        let seg_start = start_byte.max(s);
        let seg_end = end_byte.min(e);
        if seg_end > seg_start {
            let local_start = seg_start - s;
            let local_end = seg_end - s;
            let content = original.spans[i].content.as_ref();
            let slice = &content[local_start..local_end];
            acc.push(Span {
                style: *style,
                content: Cow::Borrowed(slice),
            });
        }
        if e >= end_byte {
            break;
        }
    }
    Line {
        style: original.style,
        alignment: original.alignment,
        spans: acc,
    }
}

/// Utilities to allow wrapping either borrowed or owned lines.
#[derive(Debug)]
pub(super) enum LineInput<'a> {
    Borrowed(&'a Line<'a>),
    Owned(Line<'a>),
}

impl<'a> LineInput<'a> {
    pub(super) fn as_ref(&self) -> &Line<'a> {
        match self {
            LineInput::Borrowed(line) => line,
            LineInput::Owned(line) => line,
        }
    }
}

/// This trait makes it easier to pass whatever we need into word_wrap_lines.
pub(super) trait IntoLineInput<'a> {
    fn into_line_input(self) -> LineInput<'a>;
}

impl<'a> IntoLineInput<'a> for &'a Line<'a> {
    fn into_line_input(self) -> LineInput<'a> {
        LineInput::Borrowed(self)
    }
}

impl<'a> IntoLineInput<'a> for &'a mut Line<'a> {
    fn into_line_input(self) -> LineInput<'a> {
        LineInput::Borrowed(self)
    }
}

impl<'a> IntoLineInput<'a> for Line<'a> {
    fn into_line_input(self) -> LineInput<'a> {
        LineInput::Owned(self)
    }
}

impl<'a> IntoLineInput<'a> for String {
    fn into_line_input(self) -> LineInput<'a> {
        LineInput::Owned(Line::from(self))
    }
}

impl<'a> IntoLineInput<'a> for &'a str {
    fn into_line_input(self) -> LineInput<'a> {
        LineInput::Owned(Line::from(self))
    }
}

impl<'a> IntoLineInput<'a> for Cow<'a, str> {
    fn into_line_input(self) -> LineInput<'a> {
        LineInput::Owned(Line::from(self))
    }
}

impl<'a> IntoLineInput<'a> for Span<'a> {
    fn into_line_input(self) -> LineInput<'a> {
        LineInput::Owned(Line::from(self))
    }
}

impl<'a> IntoLineInput<'a> for Vec<Span<'a>> {
    fn into_line_input(self) -> LineInput<'a> {
        LineInput::Owned(Line::from(self))
    }
}
