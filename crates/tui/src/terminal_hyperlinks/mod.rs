// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Terminal hyperlinks (OSC 8) carried beside the visible text.
//!
//! Layout measures and wraps ordinary lines; a [`HyperlinkLine`] keeps,
//! beside its line, the columns each web destination covers. The escape
//! sequences are added only where text reaches the terminal — the
//! scrollback writer and the buffer's cells (`emission`) — so they never
//! count in a width. Only an `http` or `https` URL with a host, at most
//! 8 KiB, its control characters removed, becomes a destination.

mod emission;

use std::ops::Range;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use url::Url;

pub(crate) use emission::{
    decorate_spans, mark_hyperlink_cells, osc8_open, osc8_parts, OSC8_CLOSE,
};

use crate::render::line_utils::line_to_static;
use crate::wrapping::{adaptive_wrap_line_with_source, display_width, RtOptions, WrappedLine};

/// Destinations are repeated in every linked buffer cell. Leave oversized URLs as plain text.
const MAX_HYPERLINK_DESTINATION_BYTES: usize = 8 * 1024;

/// A web destination over a range of a line's display columns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TerminalHyperlink {
    pub(crate) columns: Range<usize>,
    /// Sanitised already ([`web_destination`]).
    pub(crate) destination: String,
}

impl TerminalHyperlink {
    fn with_columns(&self, columns: Range<usize>) -> Self {
        Self {
            columns,
            destination: self.destination.clone(),
        }
    }
}

/// A line and the hyperlinks over its columns, in column order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct HyperlinkLine {
    pub(crate) line: Line<'static>,
    pub(crate) hyperlinks: Vec<TerminalHyperlink>,
}

impl HyperlinkLine {
    pub(crate) fn new(line: Line<'static>) -> Self {
        Self {
            line,
            hyperlinks: Vec::new(),
        }
    }

    pub(crate) fn width(&self) -> usize {
        self.line.spans.iter().map(span_width).sum()
    }

    /// `span` appended, linked to `destination` when it is a web one.
    pub(crate) fn push_span(&mut self, span: Span<'static>, destination: Option<&str>) {
        let start = self.width();
        let end = start + span_width(&span);
        self.line.push_span(span);
        if let Some(destination) = destination.and_then(web_destination) {
            if end > start {
                self.push_link(start..end, destination);
            }
        }
    }

    /// A link over `columns`, joined to the last one when it continues it.
    fn push_link(&mut self, columns: Range<usize>, destination: String) {
        if let Some(previous) = self.hyperlinks.last_mut() {
            if previous.destination == destination && previous.columns.end == columns.start {
                previous.columns.end = columns.end;
                return;
            }
        }
        self.hyperlinks.push(TerminalHyperlink {
            columns,
            destination,
        });
    }

    /// `other`'s spans and links after this line's.
    pub(crate) fn append(&mut self, other: HyperlinkLine) {
        let shift = self.width();
        self.line.spans.extend(other.line.spans);
        for link in other.hyperlinks {
            self.push_link(
                link.columns.start + shift..link.columns.end + shift,
                link.destination,
            );
        }
    }

    pub(crate) fn style(mut self, style: Style) -> Self {
        self.line = self.line.style(style);
        self
    }

    /// `spans` put before the line, its links moved after them.
    pub(crate) fn prefixed(mut self, spans: Vec<Span<'static>>) -> Self {
        let shift: usize = spans.iter().map(span_width).sum();
        let mut line_spans = spans;
        line_spans.append(&mut self.line.spans);
        self.line.spans = line_spans;
        for link in &mut self.hyperlinks {
            link.columns = link.columns.start + shift..link.columns.end + shift;
        }
        self
    }
}

impl From<Line<'static>> for HyperlinkLine {
    fn from(line: Line<'static>) -> Self {
        Self::new(line)
    }
}

/// The columns a span takes, a grapheme at a time, as ratatui paints it.
pub(crate) fn span_width(span: &Span<'_>) -> usize {
    span.content.graphemes(true).map(display_width).sum()
}

pub(crate) fn plain_hyperlink_lines(lines: Vec<Line<'static>>) -> Vec<HyperlinkLine> {
    lines.into_iter().map(HyperlinkLine::new).collect()
}

pub(crate) fn visible_lines(lines: Vec<HyperlinkLine>) -> Vec<Line<'static>> {
    lines.into_iter().map(|line| line.line).collect()
}

/// Each line behind `initial` (the first) or `subsequent`, its links moved.
pub(crate) fn prefix_hyperlink_lines(
    lines: Vec<HyperlinkLine>,
    initial: Span<'static>,
    subsequent: Span<'static>,
) -> Vec<HyperlinkLine> {
    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let prefix = if index == 0 {
                initial.clone()
            } else {
                subsequent.clone()
            };
            let style = line.line.style;
            line.prefixed(vec![prefix]).style(style)
        })
        .collect()
}

/// The rows `wrap` makes of `source`'s line, each with the part of each
/// link it shows: a link a wrap splits is linked whole on every row.
pub(crate) fn remap_source_wrapped_line(
    source: &HyperlinkLine,
    wrapped: Vec<WrappedLine<'_>>,
) -> Vec<HyperlinkLine> {
    let text: String = source
        .line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    let mut source_byte = 0;
    let mut source_column = 0;
    wrapped
        .into_iter()
        .map(|wrapped| {
            let line = line_to_static(&wrapped.line);
            let prefix_columns = line_prefix_width(&line, wrapped.prefix_bytes);
            let range = wrapped.range.start.min(text.len())..wrapped.range.end.min(text.len());
            source_column += display_width(&text[source_byte.min(range.start)..range.start]);
            let start = source_column;
            let end = start + display_width(&text[range.clone()]);
            source_byte = range.end;
            source_column = end;
            let hyperlinks = source
                .hyperlinks
                .iter()
                .filter_map(|link| {
                    let first = link.columns.start.max(start);
                    let last = link.columns.end.min(end);
                    (first < last).then(|| {
                        link.with_columns(
                            prefix_columns + first - start..prefix_columns + last - start,
                        )
                    })
                })
                .collect();
            HyperlinkLine { line, hyperlinks }
        })
        .collect()
}

/// The columns of the first `bytes` of `line`'s text.
fn line_prefix_width(line: &Line<'_>, bytes: usize) -> usize {
    let mut left = bytes;
    let mut width = 0;
    for span in &line.spans {
        let take = left.min(span.content.len());
        width += display_width(&span.content[..take]);
        left -= take;
        if left == 0 {
            break;
        }
    }
    width
}

/// `lines` wrapped URL-aware (`wrapping`), the initial indent before the
/// first row only, their links following the text.
pub(crate) fn adaptive_wrap_hyperlink_lines(
    lines: &[HyperlinkLine],
    options: RtOptions<'static>,
) -> Vec<HyperlinkLine> {
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let options = if index == 0 {
            options.clone()
        } else {
            options
                .clone()
                .initial_indent(options.subsequent_indent.clone())
        };
        out.extend(remap_source_wrapped_line(
            line,
            adaptive_wrap_line_with_source(&line.line, options),
        ));
    }
    out
}

/// The line with each bare web URL in its text linked.
pub(crate) fn annotate_web_urls_in_line(line: Line<'static>) -> HyperlinkLine {
    let text = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    let mut out = HyperlinkLine::new(line);
    out.hyperlinks = web_links_in_text(&text);
    out
}

/// The web URLs written in `text`, each over its columns: a
/// whitespace-separated token, without the punctuation around it (a
/// closing bracket only when unbalanced).
pub(crate) fn web_links_in_text(text: &str) -> Vec<TerminalHyperlink> {
    let mut links = Vec::new();
    let mut search_from = 0usize;
    let mut source_byte = 0usize;
    let mut source_column = 0usize;
    for raw_token in text.split_whitespace() {
        let Some(relative_start) = text[search_from..].find(raw_token) else {
            continue;
        };
        let raw_start = search_from + relative_start;
        search_from = raw_start + raw_token.len();
        let trimmed_start = raw_token
            .find(|ch: char| !is_leading_punctuation(ch))
            .unwrap_or(raw_token.len());
        let trimmed_end = trailing_url_end(&raw_token[trimmed_start..]) + trimmed_start;
        if trimmed_start >= trimmed_end {
            continue;
        }
        let candidate = &raw_token[trimmed_start..trimmed_end];
        let Some(destination) = web_destination(candidate) else {
            continue;
        };
        let candidate_start = raw_start + trimmed_start;
        // Measure disjoint prefixes so scanning a text with many URLs stays linear.
        source_column += display_width(&text[source_byte..candidate_start]);
        source_byte = candidate_start;
        let end = source_column + display_width(candidate);
        links.push(TerminalHyperlink {
            columns: source_column..end,
            destination,
        });
    }
    links
}

fn is_leading_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | '.' | ';' | '!' | '\'' | '"'
    )
}

fn trailing_url_end(candidate: &str) -> usize {
    // Count delimiter balances once rather than rescanning for every trailing closer.
    let mut balances = [0isize; 4];
    for ch in candidate.chars() {
        match ch {
            '(' => balances[0] += 1,
            ')' => balances[0] -= 1,
            '[' => balances[1] += 1,
            ']' => balances[1] -= 1,
            '{' => balances[2] += 1,
            '}' => balances[2] -= 1,
            '<' => balances[3] += 1,
            '>' => balances[3] -= 1,
            _ => {}
        }
    }
    let mut end = candidate.len();
    while end > 0 {
        let remaining = &candidate[..end];
        let Some(ch) = remaining.chars().next_back() else {
            break;
        };
        let balance = match ch {
            ')' => Some(&mut balances[0]),
            ']' => Some(&mut balances[1]),
            '}' => Some(&mut balances[2]),
            '>' => Some(&mut balances[3]),
            _ => None,
        };
        let trim = if let Some(balance) = balance {
            let unmatched = *balance < 0;
            *balance += 1;
            unmatched
        } else {
            matches!(ch, ',' | '.' | ';' | '!' | '\'' | '"')
                || ch == '?'
                    && match remaining.chars().rev().nth(1) {
                        Some(')') => balances[0] < 0,
                        Some(']') => balances[1] < 0,
                        Some('}') => balances[2] < 0,
                        Some('>') => balances[3] < 0,
                        _ => false,
                    }
        };
        if !trim {
            break;
        }
        end -= ch.len_utf8();
    }
    end
}

/// `destination` as a terminal may receive it: at most 8 KiB, its control
/// characters (ESC, BEL, C1 such as `\u{9c}`, newlines) removed, and then
/// an `http` or `https` URL with a host; else none.
pub(crate) fn web_destination(destination: &str) -> Option<String> {
    if destination.len() > MAX_HYPERLINK_DESTINATION_BYTES {
        return None;
    }
    let safe_destination: String = destination.chars().filter(|ch| !ch.is_control()).collect();
    let parsed = Url::parse(&safe_destination).ok()?;
    matches!(parsed.scheme(), "http" | "https")
        .then(|| parsed.host_str())
        .flatten()?;
    Some(safe_destination)
}

#[cfg(test)]
mod tests;
