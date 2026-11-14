// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Word-wrapping with URL-aware heuristics.
//!
//! The interface renders text that often holds URLs: answers, plans,
//! command output. Standard `textwrap` hyphenation treats `/` and `-` as
//! split points, which breaks a URL across rows, where neither a person nor
//! a terminal's own link detection can take it whole.
//!
//! - **Standard** (`word_wrap_line`, `word_wrap_lines`): textwrap with the
//!   caller's options unchanged, for text known to be prose.
//! - **Adaptive** (`adaptive_wrap_line_with_source`): a line with a
//!   URL-like token keeps the token whole; prose around it still wraps at
//!   words, a non-URL token is split only when it is wider than a row. A
//!   URL wider than its row stays wider: the transcript cuts it at the
//!   row's width where it is shown (`transcript_view::text`).
//!
//! Every path measures with the columns ratatui paints, and keeps compound
//! graphemes (a joined emoji, a halfwidth sound mark) whole (`ranges`).

mod ranges;
mod spans;
mod urls;

use std::ops::Range;

use ratatui::text::Line;
use textwrap::{Options, WordSeparator};

pub(crate) use ranges::{wrap_ranges, wrap_ranges_trim};
pub(crate) use spans::{display_width, line_width};

use crate::render::line_utils::push_owned_lines;
use spans::{flatten_line, slice_line_spans, IntoLineInput};

/// Reconfigures wrapping options so that URL-like tokens are never split.
///
/// Sets `AsciiSpace` word separation (so `/` and `-` inside URLs are
/// not treated as break points), disables `break_words`, and prevents
/// per-word hyphenation. Mixed URL/prose lines use a dedicated wrapper
/// so normal prose can still wrap cleanly around the preserved URL token.
fn url_preserving_wrap_options(opts: RtOptions<'_>) -> RtOptions<'_> {
    opts.word_separator(WordSeparator::AsciiSpace)
        .word_splitter(textwrap::WordSplitter::NoHyphenation)
        .break_words(false)
}

/// A display row and the exact source fragment used to build it.
#[derive(Debug)]
pub(crate) struct WrappedLine<'a> {
    pub(crate) line: Line<'a>,
    /// The byte range of the source line's text the row shows.
    pub(crate) range: Range<usize>,
    /// The bytes of the indent before the row's source text.
    pub(crate) prefix_bytes: usize,
}

/// A line's rows, URL-like tokens kept whole: when the line has a URL,
/// URL-only lines wrap at spaces without breaking a word, and lines
/// mixing URLs and prose move the URL whole and split only a prose token
/// wider than a row. A URL wider than its row stays wider.
pub(crate) fn adaptive_wrap_line_with_source<'a>(
    line: &'a Line<'a>,
    base: RtOptions<'a>,
) -> Vec<WrappedLine<'a>> {
    let (flat, span_bounds) = flatten_line(line);
    match urls::url_and_prose_tokens(&flat) {
        (false, _) => word_wrap_flattened_line(line, &flat, &span_bounds, base),
        (true, true) => mixed_url_wrap_line(line, &flat, &span_bounds, base),
        (true, false) => {
            let initial_indent = base.initial_indent.clone();
            let width = base.width;
            let rows = word_wrap_flattened_line(
                line,
                &flat,
                &span_bounds,
                url_preserving_wrap_options(base),
            );
            keep_url_with_its_prefix(rows, line, &flat, &span_bounds, &initial_indent, width)
        }
    }
}

/// A URL-only line's rows, the rows before its first URL (its leading
/// whitespace, a list marker or a quote's `>`) put back on the URL's row
/// when they are blank, or when the URL is wider than its row anyway: the
/// row is then cut where it is shown, as a terminal wraps it, rather than
/// leave a marker or an indent on a row of its own. Textwrap gives them
/// one when the URL after them does not fit.
fn keep_url_with_its_prefix<'a>(
    mut rows: Vec<WrappedLine<'a>>,
    line: &'a Line<'a>,
    flat: &str,
    span_bounds: &[(Range<usize>, ratatui::style::Style)],
    initial_indent: &Line<'a>,
    width: usize,
) -> Vec<WrappedLine<'a>> {
    let Some(url_row) = rows
        .iter()
        .position(|row| urls::url_and_prose_tokens(&flat[row.range.clone()]).0)
        .filter(|row| *row > 0)
    else {
        return rows;
    };
    let prefix_is_blank = flat[..rows[url_row].range.start].trim().is_empty();
    if !prefix_is_blank && line_width(&rows[url_row].line) <= width {
        return rows;
    }
    let range = 0..rows[url_row].range.end;
    rows[url_row] = WrappedLine {
        line: indented_slice(line, span_bounds, &range, initial_indent),
        range,
        prefix_bytes: line_bytes(initial_indent),
    };
    rows.drain(..url_row);
    rows
}

#[derive(Debug, Clone)]
pub struct RtOptions<'a> {
    /// The width in columns at which the text will be wrapped.
    pub width: usize,
    /// Line ending used for breaking lines.
    pub line_ending: textwrap::LineEnding,
    /// Indentation used for the first line of output. See the
    /// [`Options::initial_indent`] method.
    pub initial_indent: Line<'a>,
    /// Indentation used for subsequent lines of output. See the
    /// [`Options::subsequent_indent`] method.
    pub subsequent_indent: Line<'a>,
    /// Allow long words to be broken if they cannot fit on a line.
    /// When set to `false`, some lines may be longer than
    /// `self.width`. See the [`Options::break_words`] method.
    pub break_words: bool,
    /// Wrapping algorithm to use, see the implementations of the
    /// [`WrapAlgorithm`] trait for details.
    pub wrap_algorithm: textwrap::WrapAlgorithm,
    /// The line breaking algorithm to use, see the [`WordSeparator`]
    /// trait for an overview and possible implementations.
    pub word_separator: textwrap::WordSeparator,
    /// The method for splitting words. This can be used to prohibit
    /// splitting words on hyphens, or it can be used to implement
    /// language-aware machine hyphenation.
    pub word_splitter: textwrap::WordSplitter,
}

impl From<usize> for RtOptions<'_> {
    fn from(width: usize) -> Self {
        RtOptions::new(width)
    }
}

impl<'a> RtOptions<'a> {
    pub fn new(width: usize) -> Self {
        RtOptions {
            width,
            line_ending: textwrap::LineEnding::LF,
            initial_indent: Line::default(),
            subsequent_indent: Line::default(),
            break_words: true,
            word_separator: textwrap::WordSeparator::new(),
            wrap_algorithm: textwrap::WrapAlgorithm::FirstFit,
            word_splitter: textwrap::WordSplitter::HyphenSplitter,
        }
    }

    pub fn initial_indent(self, initial_indent: Line<'a>) -> Self {
        RtOptions {
            initial_indent,
            ..self
        }
    }

    pub fn subsequent_indent(self, subsequent_indent: Line<'a>) -> Self {
        RtOptions {
            subsequent_indent,
            ..self
        }
    }

    pub fn break_words(self, break_words: bool) -> Self {
        RtOptions {
            break_words,
            ..self
        }
    }

    pub fn word_separator(self, word_separator: textwrap::WordSeparator) -> RtOptions<'a> {
        RtOptions {
            word_separator,
            ..self
        }
    }

    pub fn wrap_algorithm(self, wrap_algorithm: textwrap::WrapAlgorithm) -> RtOptions<'a> {
        RtOptions {
            wrap_algorithm,
            ..self
        }
    }

    pub fn word_splitter(self, word_splitter: textwrap::WordSplitter) -> RtOptions<'a> {
        RtOptions {
            word_splitter,
            ..self
        }
    }
}

#[must_use]
pub(crate) fn word_wrap_line<'a, O>(line: &'a Line<'a>, width_or_options: O) -> Vec<Line<'a>>
where
    O: Into<RtOptions<'a>>,
{
    word_wrap_line_with_source(line, width_or_options)
        .into_iter()
        .map(|wrapped| wrapped.line)
        .collect()
}

/// Standard wrapping with the same source ranges used to slice styled spans.
pub(crate) fn word_wrap_line_with_source<'a, O>(
    line: &'a Line<'a>,
    width_or_options: O,
) -> Vec<WrappedLine<'a>>
where
    O: Into<RtOptions<'a>>,
{
    let (flat, span_bounds) = flatten_line(line);
    word_wrap_flattened_line(line, &flat, &span_bounds, width_or_options.into())
}

fn word_wrap_flattened_line<'a>(
    line: &'a Line<'a>,
    flat: &str,
    span_bounds: &[(Range<usize>, ratatui::style::Style)],
    rt_opts: RtOptions<'a>,
) -> Vec<WrappedLine<'a>> {
    let opts = Options::new(rt_opts.width)
        .line_ending(rt_opts.line_ending)
        .break_words(rt_opts.break_words)
        .wrap_algorithm(rt_opts.wrap_algorithm)
        .word_separator(rt_opts.word_separator)
        .word_splitter(rt_opts.word_splitter);

    let mut out: Vec<WrappedLine<'a>> = Vec::new();

    // Compute first line range with reduced width due to initial indent.
    let initial_width_available = opts
        .width
        .saturating_sub(line_width(&rt_opts.initial_indent))
        .max(1);
    let initial_wrapped = wrap_ranges_trim(flat, opts.clone().width(initial_width_available));
    let Some(first_line_range) = initial_wrapped.first() else {
        return vec![WrappedLine {
            line: rt_opts.initial_indent.clone().style(line.style),
            range: 0..0,
            prefix_bytes: line_bytes(&rt_opts.initial_indent),
        }];
    };

    out.push(WrappedLine {
        line: indented_slice(line, span_bounds, first_line_range, &rt_opts.initial_indent),
        range: first_line_range.clone(),
        prefix_bytes: line_bytes(&rt_opts.initial_indent),
    });

    let base = first_line_range.end;
    let skip_leading_spaces = flat[base..].chars().take_while(|c| *c == ' ').count();
    let base = base + skip_leading_spaces;
    let subsequent_width_available = opts
        .width
        .saturating_sub(line_width(&rt_opts.subsequent_indent))
        .max(1);
    // First-fit decisions do not depend on later rows. Reuse the full first pass when
    // both indents leave the same width and the remainder starts after a complete word.
    // Splitting inside a word can change hyphenation when the remainder is tokenized again.
    // Custom tokenizers can also repartition the remainder, so they retain the second pass.
    let remaining_wrapped = if initial_width_available == subsequent_width_available
        && matches!(rt_opts.wrap_algorithm, textwrap::WrapAlgorithm::FirstFit)
        && matches!(
            rt_opts.word_separator,
            WordSeparator::AsciiSpace | WordSeparator::UnicodeBreakProperties
        )
        && (skip_leading_spaces > 0 || base == flat.len())
        // The projected and plain text wrappers interpret control characters differently.
        && !flat.as_bytes().iter().any(u8::is_ascii_control)
        && initial_wrapped
            .get(1)
            .map_or(base == flat.len(), |range| range.start == base)
    {
        initial_wrapped.into_iter().skip(1).collect()
    } else {
        wrap_ranges_trim(&flat[base..], opts.width(subsequent_width_available))
            .into_iter()
            .map(|range| (range.start + base)..(range.end + base))
            .collect::<Vec<_>>()
    };
    for offset_range in remaining_wrapped {
        if offset_range.is_empty() {
            continue;
        }
        out.push(WrappedLine {
            line: indented_slice(line, span_bounds, &offset_range, &rt_opts.subsequent_indent),
            range: offset_range,
            prefix_bytes: line_bytes(&rt_opts.subsequent_indent),
        });
    }

    out
}

fn mixed_url_wrap_line<'a>(
    line: &'a Line<'a>,
    flat: &str,
    span_bounds: &[(Range<usize>, ratatui::style::Style)],
    rt_opts: RtOptions<'a>,
) -> Vec<WrappedLine<'a>> {
    let initial_width_available = rt_opts
        .width
        .saturating_sub(line_width(&rt_opts.initial_indent))
        .max(1);
    let subsequent_width_available = rt_opts
        .width
        .saturating_sub(line_width(&rt_opts.subsequent_indent))
        .max(1);
    let ranges =
        urls::mixed_url_wrap_ranges(flat, initial_width_available, subsequent_width_available);

    let out = ranges
        .iter()
        .enumerate()
        .map(|(idx, range)| {
            let indent = if idx == 0 {
                &rt_opts.initial_indent
            } else {
                &rt_opts.subsequent_indent
            };
            WrappedLine {
                line: indented_slice(line, span_bounds, range, indent),
                range: range.clone(),
                prefix_bytes: line_bytes(indent),
            }
        })
        .collect::<Vec<_>>();

    if out.is_empty() {
        vec![WrappedLine {
            line: rt_opts.initial_indent.clone().style(line.style),
            range: 0..0,
            prefix_bytes: line_bytes(&rt_opts.initial_indent),
        }]
    } else {
        out
    }
}

/// `indent`, then the part of `line` in `range`, the line's style over
/// each span's, as the scrollback writer merges them (Codex lets the
/// span's win here).
fn indented_slice<'a>(
    line: &'a Line<'a>,
    span_bounds: &[(Range<usize>, ratatui::style::Style)],
    range: &Range<usize>,
    indent: &Line<'a>,
) -> Line<'a> {
    let mut wrapped = indent.clone().style(line.style);
    let sliced = slice_line_spans(line, span_bounds, range);
    wrapped.spans.extend(
        sliced
            .spans
            .into_iter()
            .map(|span| span.patch_style(line.style)),
    );
    wrapped
}

fn line_bytes(line: &Line<'_>) -> usize {
    line.spans.iter().map(|span| span.content.len()).sum()
}

/// Wrap a sequence of lines, applying the initial indent only to the very first
/// output line, and using the subsequent indent for all later wrapped pieces.
#[allow(private_bounds, reason = "the input conversions are the wrapper's own")]
pub(crate) fn word_wrap_lines<'a, I, O, L>(lines: I, width_or_options: O) -> Vec<Line<'static>>
where
    I: IntoIterator<Item = L>,
    L: IntoLineInput<'a>,
    O: Into<RtOptions<'a>>,
{
    let base_opts: RtOptions<'a> = width_or_options.into();
    let mut out: Vec<Line<'static>> = Vec::new();

    for (idx, line) in lines.into_iter().enumerate() {
        let line_input = line.into_line_input();
        let opts = if idx == 0 {
            base_opts.clone()
        } else {
            let sub = base_opts.subsequent_indent.clone();
            base_opts.clone().initial_indent(sub)
        };
        let wrapped = word_wrap_line(line_input.as_ref(), opts);
        push_owned_lines(&wrapped, &mut out);
    }

    out
}

#[cfg(test)]
mod tests;
