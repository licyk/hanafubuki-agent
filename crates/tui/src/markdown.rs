// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Markdown rendered into lines: [`append_markdown`] for any Markdown, and
//! [`append_markdown_agent`] for a model's answer, which first unwraps the
//! ```` ```md ```` fences that hold a table.
//!
//! Models often put a table in a ```` ```markdown ```` fence, as code; the
//! unwrapper gives pulldown-cmark the table instead. It is conservative: it
//! reads a fence whole before deciding, unwraps only an `md` or `markdown`
//! fence whose body has a header row followed by a delimiter row, and keeps
//! an unclosed fence as code.

use std::borrow::Cow;
use std::ops::Range;

use ratatui::text::Line;

use crate::markdown_render::{ListSpacing, StreamingMarkdownRender};
use crate::table_detect;
use crate::terminal_hyperlinks::HyperlinkLine;

/// `markdown_source` rendered and wrapped to `width`, local file links
/// relative to `cwd`, appended to `lines`: no hyperlinks, so a web link's
/// destination stays in its text.
pub(crate) fn append_markdown(
    markdown_source: &str,
    width: Option<usize>,
    cwd: Option<&str>,
    lines: &mut Vec<Line<'static>>,
) {
    lines.extend(crate::markdown_render::render_markdown_unlinked(
        markdown_source,
        width,
        cwd,
    ));
}

/// A model's answer: [`append_markdown`] after the table fences are
/// unwrapped, when tables are on.
#[cfg(test)]
pub(crate) fn append_markdown_agent(
    markdown_source: &str,
    width: Option<usize>,
    cwd: Option<&str>,
    lines: &mut Vec<Line<'static>>,
) {
    lines.extend(crate::terminal_hyperlinks::visible_lines(
        render_markdown_agent_with_list_spacing(
            markdown_source,
            width,
            cwd,
            ListSpacing::AfterMultiline,
        ),
    ));
}

/// [`append_markdown`]'s lines, with their links, lists spaced by
/// `list_spacing`.
pub(crate) fn render_markdown_with_list_spacing(
    markdown_source: &str,
    width: Option<usize>,
    cwd: Option<&str>,
    list_spacing: ListSpacing,
) -> Vec<HyperlinkLine> {
    crate::markdown_render::render_markdown_lines(markdown_source, width, cwd, list_spacing)
}

/// [`append_markdown_agent`]'s lines, lists spaced by `list_spacing`.
pub(crate) fn render_markdown_agent_with_list_spacing(
    markdown_source: &str,
    width: Option<usize>,
    cwd: Option<&str>,
    list_spacing: ListSpacing,
) -> Vec<HyperlinkLine> {
    let normalized = if crate::markdown_render::preferences::current().tables {
        unwrap_markdown_fences(markdown_source)
    } else {
        Cow::Borrowed(markdown_source)
    };
    render_markdown_with_list_spacing(&normalized, width, cwd, list_spacing)
}

/// [`render_markdown_agent_with_list_spacing`] with what a stream needs of
/// its blocks. The offsets are the source's: after a fence was unwrapped, a
/// boundary is kept when the rest from it is the source's own end (it
/// comes after the lines taken out); otherwise the block stays the
/// stream's to change.
pub(crate) fn render_streaming_markdown_agent(
    markdown_source: &str,
    width: Option<usize>,
    cwd: Option<&str>,
    list_spacing: ListSpacing,
) -> StreamingMarkdownRender {
    let normalized = if crate::markdown_render::preferences::current().tables {
        unwrap_markdown_fences(markdown_source)
    } else {
        Cow::Borrowed(markdown_source)
    };
    let mut rendered =
        crate::markdown_render::render_streaming_markdown(&normalized, width, cwd, list_spacing);
    if normalized != markdown_source {
        let in_source = |boundary: usize| {
            markdown_source
                .strip_suffix(&normalized[boundary..])
                .map(str::len)
        };
        rendered.pending_math_start = rendered
            .pending_math_start
            .map(|boundary| in_source(boundary).unwrap_or(0));
        rendered.last_top_level_block_start =
            rendered.last_top_level_block_start.and_then(in_source);
        rendered.mutable_fence_start = rendered
            .mutable_fence_start
            .map(|boundary| in_source(boundary).unwrap_or(0));
    }
    rendered
}

/// `markdown_source` with each ```` ```md ````/```` ```markdown ```` fence
/// whose body holds a table replaced by its body. Other fences, Markdown
/// fences without a table, and an unclosed fence stay as they are.
pub(crate) fn unwrap_markdown_fences(markdown_source: &str) -> Cow<'_, str> {
    if !markdown_source.contains("```") && !markdown_source.contains("~~~") {
        return Cow::Borrowed(markdown_source);
    }

    struct MarkdownCandidate {
        fence: Fence,
        opening_range: Range<usize>,
        content_ranges: Vec<Range<usize>>,
    }

    enum ActiveFence {
        Passthrough(Fence),
        MarkdownCandidate(Box<MarkdownCandidate>),
    }

    let mut out = String::with_capacity(markdown_source.len());
    let mut active_fence: Option<ActiveFence> = None;
    let mut source_offset = 0usize;
    let push = |out: &mut String, range: Range<usize>| out.push_str(&markdown_source[range]);

    for line in markdown_source.split_inclusive('\n') {
        let line_range = source_offset..source_offset + line.len();
        source_offset = line_range.end;

        match active_fence.take() {
            Some(ActiveFence::Passthrough(fence)) => {
                push(&mut out, line_range);
                if !is_close_fence(line, fence) {
                    active_fence = Some(ActiveFence::Passthrough(fence));
                }
            }
            Some(ActiveFence::MarkdownCandidate(mut candidate)) => {
                if !is_close_fence(line, candidate.fence) {
                    candidate.content_ranges.push(line_range);
                    active_fence = Some(ActiveFence::MarkdownCandidate(candidate));
                    continue;
                }
                let content: String = candidate
                    .content_ranges
                    .iter()
                    .map(|range| &markdown_source[range.clone()])
                    .collect();
                if markdown_fence_contains_table(&content, candidate.fence.is_blockquoted) {
                    out.push_str(&content);
                } else {
                    push(&mut out, candidate.opening_range);
                    out.push_str(&content);
                    push(&mut out, line_range);
                }
            }
            None => match parse_open_fence(line) {
                Some((fence, true)) => {
                    active_fence = Some(ActiveFence::MarkdownCandidate(Box::new(
                        MarkdownCandidate {
                            fence,
                            opening_range: line_range,
                            content_ranges: Vec::new(),
                        },
                    )));
                }
                Some((fence, false)) => {
                    push(&mut out, line_range);
                    active_fence = Some(ActiveFence::Passthrough(fence));
                }
                None => push(&mut out, line_range),
            },
        }
    }

    if let Some(ActiveFence::MarkdownCandidate(candidate)) = active_fence {
        push(&mut out, candidate.opening_range);
        for range in candidate.content_ranges {
            push(&mut out, range);
        }
    }
    Cow::Owned(out)
}

#[derive(Clone, Copy)]
struct Fence {
    marker: char,
    len: usize,
    is_blockquoted: bool,
}

/// `line` without its newline and up to three spaces of indentation; `None`
/// when indented four columns or more (an indented code line).
fn strip_line_indent(line: &str) -> Option<&str> {
    let without_newline = line.strip_suffix('\n').unwrap_or(line);
    let mut byte_idx = 0usize;
    let mut column = 0usize;
    for byte in without_newline.as_bytes() {
        match byte {
            b' ' => column += 1,
            b'\t' => column += 4,
            _ => break,
        }
        byte_idx += 1;
        if column >= 4 {
            return None;
        }
    }
    Some(&without_newline[byte_idx..])
}

/// An opening fence, and whether its info string says Markdown.
fn parse_open_fence(line: &str) -> Option<(Fence, bool)> {
    let trimmed = strip_line_indent(line)?;
    let is_blockquoted = trimmed.trim_start().starts_with('>');
    let fence_scan_text = table_detect::strip_blockquote_prefix(trimmed);
    let (marker, len) = table_detect::parse_fence_marker(fence_scan_text)?;
    let is_markdown = table_detect::is_markdown_fence_info(fence_scan_text, len);
    Some((
        Fence {
            marker,
            len,
            is_blockquoted,
        },
        is_markdown,
    ))
}

fn is_close_fence(line: &str, fence: Fence) -> bool {
    let Some(trimmed) = strip_line_indent(line) else {
        return false;
    };
    let fence_scan_text = if fence.is_blockquoted {
        if !trimmed.trim_start().starts_with('>') {
            return false;
        }
        table_detect::strip_blockquote_prefix(trimmed)
    } else {
        trimmed
    };
    table_detect::parse_fence_marker(fence_scan_text).is_some_and(|(marker, len)| {
        marker == fence.marker && len >= fence.len && fence_scan_text[len..].trim().is_empty()
    })
}

/// Whether a header line is followed directly by a delimiter line.
fn markdown_fence_contains_table(content: &str, is_blockquoted_fence: bool) -> bool {
    let mut previous_line: Option<&str> = None;
    for line in content.lines() {
        let text = if is_blockquoted_fence {
            table_detect::strip_blockquote_prefix(line)
        } else {
            line
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            previous_line = None;
            continue;
        }
        if previous_line.is_some_and(|previous| {
            table_detect::is_table_header_line(previous)
                && !table_detect::is_table_delimiter_line(previous)
        }) && table_detect::is_table_delimiter_line(trimmed)
        {
            return true;
        }
        previous_line = Some(trimmed);
    }
    false
}

#[cfg(test)]
mod tests;
