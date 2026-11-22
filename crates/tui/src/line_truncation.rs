// Modified from OpenAI Codex 57a808a392 (Apache-2.0).
// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

/// Paragraph paints each visible grapheme separately, even when the whole string
/// has a narrower Unicode width (for example, Arabic lam followed by alef).
pub(crate) fn span_width(span: &Span<'_>) -> usize {
    span.styled_graphemes(Style::default())
        .map(|g| UnicodeWidthStr::width(g.symbol))
        .sum()
}

pub(crate) fn line_width(line: &Line<'_>) -> usize {
    line.iter().map(span_width).sum()
}

pub(crate) fn truncate_line_to_width(line: Line<'static>, max_width: usize) -> Line<'static> {
    if max_width == 0 {
        return Line::from(Vec::<Span<'static>>::new());
    }

    let Line {
        style,
        alignment,
        spans,
    } = line;
    let mut used = 0usize;
    let mut spans_out: Vec<Span<'static>> = Vec::with_capacity(spans.len());

    for span in spans {
        let span_width = span_width(&span);

        if span_width == 0 {
            spans_out.push(span);
            continue;
        }

        if used >= max_width {
            break;
        }

        if used + span_width <= max_width {
            used += span_width;
            spans_out.push(span);
            continue;
        }

        let mut prefix = String::new();
        for grapheme in span.styled_graphemes(Style::default()) {
            let grapheme_width = UnicodeWidthStr::width(grapheme.symbol);
            if used + grapheme_width > max_width {
                break;
            }
            prefix.push_str(grapheme.symbol);
            used += grapheme_width;
        }

        if !prefix.is_empty() {
            spans_out.push(Span::styled(prefix, span.style));
        }

        break;
    }

    Line {
        style,
        alignment,
        spans: spans_out,
    }
}

/// Truncate a styled line to `max_width` and append an ellipsis on overflow.
///
/// Intended for short UI rows. This preserves a fast no-overflow path (width
/// pre-scan + return original line unchanged) and uses `truncate_line_to_width`
/// for the overflow case.
/// Performance should be reevaluated if using this method in loops/over larger content in the future.
pub(crate) fn truncate_line_with_ellipsis_if_overflow(
    line: Line<'static>,
    max_width: usize,
) -> Line<'static> {
    if max_width == 0 {
        return Line::from(Vec::<Span<'static>>::new());
    }

    if line_width(&line) <= max_width {
        return line;
    }

    truncate_line_with_ellipsis(line, max_width)
}

/// Fit an already overflowing line into a nonempty area, including the ellipsis.
pub(crate) fn truncate_line_with_ellipsis(line: Line<'static>, max_width: usize) -> Line<'static> {
    debug_assert!(max_width > 0);
    let truncated = truncate_line_to_width(line, max_width.saturating_sub(1));
    let Line {
        style,
        alignment,
        mut spans,
    } = truncated;
    let ellipsis_style = spans.last().map(|span| span.style).unwrap_or_default();
    spans.push(Span::styled("…", ellipsis_style));
    Line {
        style,
        alignment,
        spans,
    }
}

/// Shorten the path components (filename first) chosen to distinguish two documents.
pub(crate) fn document_label(parts: &[&str], width: usize) -> String {
    use crate::wrapping::display_width;
    let name = parts.iter().rev().copied().collect::<Vec<_>>().join("/");
    if display_width(&name) <= width {
        return name;
    }
    if parts.len() < 2 || width < 4 {
        return filename_label(&name, width);
    }
    let directory = parts[parts.len() - 1];
    let budget = (width / 2).min(display_width(directory));
    let directory = middle_label(directory, budget);
    let filename = filename_label(
        parts[0],
        width.saturating_sub(display_width(&directory) + 1),
    );
    format!("{directory}/{filename}")
}

pub(crate) fn filename_label(name: &str, width: usize) -> String {
    let extension = name
        .rsplit_once('.')
        .filter(|(_, ext)| !ext.contains(['/', '\\']))
        .map_or(0, |(_, ext)| crate::wrapping::display_width(ext) + 1);
    middle_label_with_suffix(name, width, extension)
}

pub(crate) fn middle_label(text: &str, width: usize) -> String {
    middle_label_with_suffix(text, width, 0)
}

fn middle_label_with_suffix(text: &str, width: usize, suffix_width: usize) -> String {
    use crate::wrapping::display_width;
    use unicode_segmentation::UnicodeSegmentation;
    if width == 0 {
        return String::new();
    }
    if display_width(text) <= width {
        return text.to_owned();
    }
    let mut prefix = (width - 1) / 2;
    if suffix_width < width - 1 {
        prefix = prefix.min(width - 1 - suffix_width);
    }
    let mut used = 0;
    let start = text
        .grapheme_indices(true)
        .find_map(|(index, g)| {
            let cells = display_width(g);
            if used + cells > prefix {
                Some(index)
            } else {
                used += cells;
                None
            }
        })
        .unwrap_or(text.len());
    let mut remaining = width - used - 1;
    let mut end = text.len();
    for (index, g) in text[start..].grapheme_indices(true).rev() {
        let cells = display_width(g);
        if cells > remaining {
            break;
        }
        remaining -= cells;
        end = start + index;
    }
    format!("{}…{}", &text[..start], &text[end..])
}
