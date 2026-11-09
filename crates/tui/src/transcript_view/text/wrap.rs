// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Wrapping a styled line into display rows, each with the byte range of
//! the line's text it shows.

use std::ops::Range;

use ratatui::text::{Line, Span};
use textwrap::Options;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::wrapping::wrap_ranges_trim;

const TAB_STOP: usize = 4;

/// The line with each tab replaced by spaces up to the next tab stop.
pub(super) fn expand_tabs(line: Line<'static>) -> Line<'static> {
    if !line.spans.iter().any(|span| span.content.contains('\t')) {
        return line;
    }
    let mut column = 0;
    let spans = line
        .spans
        .into_iter()
        .map(|span| {
            let mut content = String::with_capacity(span.content.len());
            for grapheme in span.content.graphemes(true) {
                if grapheme == "\t" {
                    let spaces = TAB_STOP - column % TAB_STOP;
                    content.extend(std::iter::repeat_n(' ', spaces));
                    column += spaces;
                } else {
                    content.push_str(grapheme);
                    column += grapheme.width();
                }
            }
            Span::styled(content, span.style)
        })
        .collect::<Vec<_>>();
    Line {
        spans,
        style: line.style,
        alignment: line.alignment,
    }
}

/// The rows of `line` at `columns`: wrapped at words, a word wider than
/// a row at graphemes. The whitespace a wrap drops stays in the text,
/// between the rows' ranges.
pub(super) fn wrap_line(
    line: &Line<'static>,
    columns: usize,
) -> Vec<(Range<usize>, Line<'static>)> {
    let text: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    if text.width() <= columns {
        return vec![(0..text.len(), line.clone())];
    }
    // textwrap breaks long words at characters, which would split a
    // joined emoji; words are broken here at graphemes instead.
    let words = wrap_ranges_trim(&text, Options::new(columns).break_words(false));
    let mut ranges = Vec::new();
    for word_range in words {
        let mut start = word_range.start;
        let mut used = 0;
        for (offset, grapheme) in text[word_range.clone()].grapheme_indices(true) {
            let width = grapheme.width();
            if used > 0 && used + width > columns {
                ranges.push(start..word_range.start + offset);
                start = word_range.start + offset;
                used = 0;
            }
            used += width;
        }
        ranges.push(start..word_range.end);
    }
    ranges
        .into_iter()
        .map(|range| {
            let row = slice_line(line, range.clone());
            (range, row)
        })
        .collect()
}

/// The part of `line` in a byte range of its text, with its styles.
pub(super) fn slice_line(line: &Line<'_>, range: Range<usize>) -> Line<'static> {
    let mut offset = 0;
    let spans = line
        .spans
        .iter()
        .filter_map(|span| {
            let start = range.start.saturating_sub(offset).min(span.content.len());
            let end = range.end.saturating_sub(offset).min(span.content.len());
            offset += span.content.len();
            (start < end).then(|| Span::styled(span.content[start..end].to_owned(), span.style))
        })
        .collect::<Vec<_>>();
    Line {
        spans,
        style: line.style,
        alignment: line.alignment,
    }
}
