// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Wrapping a styled line into display rows, each with the byte range of
//! the line's text it shows and the part of each link over it.

use std::ops::Range;

use ratatui::text::{Line, Span};
use textwrap::Options;
use unicode_segmentation::UnicodeSegmentation;

use crate::terminal_hyperlinks::{remap_source_wrapped_line, HyperlinkLine};
use crate::wrapping::{
    adaptive_wrap_line_with_source, display_width, line_width, wrap_ranges_trim, RtOptions,
    WrappedLine,
};

const TAB_STOP: usize = 4;

/// The line with each tab replaced by spaces up to the next tab stop, its
/// links moved with the text after each tab.
pub(super) fn expand_tabs(line: HyperlinkLine) -> HyperlinkLine {
    if !line
        .line
        .spans
        .iter()
        .any(|span| span.content.contains('\t'))
    {
        return line;
    }
    let mut column = 0;
    // The columns before and after expansion at each grapheme boundary.
    let mut boundaries = vec![(0, 0)];
    let mut source_column = 0;
    let spans = line
        .line
        .spans
        .into_iter()
        .map(|span| {
            let mut content = String::with_capacity(span.content.len());
            for grapheme in span.content.graphemes(true) {
                source_column += display_width(grapheme);
                if grapheme == "\t" {
                    let spaces = TAB_STOP - column % TAB_STOP;
                    content.extend(std::iter::repeat_n(' ', spaces));
                    column += spaces;
                } else {
                    content.push_str(grapheme);
                    column += display_width(grapheme);
                }
                boundaries.push((source_column, column));
            }
            Span::styled(content, span.style)
        })
        .collect::<Vec<_>>();
    // A zero-width tab shares its column with the text before it: a link
    // starts after it and ends before it.
    let start = |at: usize| {
        boundaries
            .iter()
            .rev()
            .find(|(source, _)| *source <= at)
            .map_or(0, |(_, column)| *column)
    };
    let end = |at: usize| {
        boundaries
            .iter()
            .find(|(source, _)| *source >= at)
            .map_or(column, |(_, column)| *column)
    };
    let mut hyperlinks = line.hyperlinks;
    for link in &mut hyperlinks {
        link.columns = start(link.columns.start)..end(link.columns.end);
    }
    HyperlinkLine {
        line: Line {
            spans,
            style: line.line.style,
            alignment: line.line.alignment,
        },
        hyperlinks,
    }
}

/// The rows of `line` at `columns`: wrapped at words, a word wider than
/// a row at graphemes; a line with links keeps a URL whole where it fits
/// and cuts it at the row's width, as a terminal would, where it does
/// not. The whitespace a wrap drops stays in the text, between the rows'
/// ranges.
pub(super) fn wrap_line(
    line: &HyperlinkLine,
    columns: usize,
) -> Vec<(Range<usize>, HyperlinkLine)> {
    let text: String = line
        .line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    if display_width(&text) <= columns {
        return vec![(0..text.len(), line.clone())];
    }
    let rows = if line.hyperlinks.is_empty() {
        word_rows(&line.line, &text, columns)
    } else {
        adaptive_wrap_line_with_source(&line.line, RtOptions::new(columns))
            .into_iter()
            .flat_map(|row| cut_to_width(&line.line, &text, row, columns))
            .collect()
    };
    let ranges = rows.iter().map(|row| row.range.clone()).collect::<Vec<_>>();
    ranges
        .into_iter()
        .zip(remap_source_wrapped_line(line, rows))
        .collect()
}

/// `row`, or its pieces at `columns` when it is wider, cut at graphemes.
fn cut_to_width<'a>(
    line: &'a Line<'static>,
    text: &str,
    row: WrappedLine<'a>,
    columns: usize,
) -> Vec<WrappedLine<'a>> {
    if line_width(&row.line) <= columns {
        return vec![row];
    }
    let mut pieces = Vec::new();
    let mut start = row.range.start;
    let mut used = 0;
    for (offset, grapheme) in text[row.range.clone()].grapheme_indices(true) {
        let width = display_width(grapheme);
        if used > 0 && used + width > columns {
            pieces.push(row.range.start + offset);
            used = 0;
        }
        used += width;
    }
    pieces.push(row.range.end);
    pieces
        .into_iter()
        .map(|end| {
            let range = start..end;
            start = end;
            WrappedLine {
                line: slice_line(line, range.clone()),
                range,
                prefix_bytes: 0,
            }
        })
        .collect()
}

/// Rows at words, a word wider than a row broken at graphemes: textwrap
/// breaks long words at characters, which would split a joined emoji.
fn word_rows<'a>(line: &'a Line<'static>, text: &str, columns: usize) -> Vec<WrappedLine<'a>> {
    let words = wrap_ranges_trim(text, Options::new(columns).break_words(false));
    let mut ranges = Vec::new();
    for word_range in words {
        let mut start = word_range.start;
        let mut used = 0;
        for (offset, grapheme) in text[word_range.clone()].grapheme_indices(true) {
            let width = display_width(grapheme);
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
        .map(|range| WrappedLine {
            line: slice_line(line, range.clone()),
            range,
            prefix_bytes: 0,
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
