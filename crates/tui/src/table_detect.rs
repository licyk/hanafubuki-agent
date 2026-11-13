// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Pipe-table structure and fenced-code-block tracking on raw Markdown
//! source, shared by the ```` ```md ```` fence unwrapper (`markdown.rs`) and
//! the stream's holdback (`markdown_stream.rs`).
//!
//! A GFM pipe table is a header line (pipe-separated segments, one of them
//! not empty), a delimiter line right after it (only `---`, `:---`, `---:`
//! or `:---:` segments, each with at least three dashes), then body rows.
//! The functions here look at one line at a time; their callers pair
//! consecutive lines to confirm a table.

/// The trimmed segments of a pipe-delimited line; `None` when it is empty
/// or has no unescaped separator. A leading and a trailing pipe are
/// stripped first. Escaped pipes stay inside their segment: this tells
/// whether a line can be part of a table, not how a cell reads.
pub(crate) fn parse_table_segments(line: &str) -> Option<Vec<&str>> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let has_outer_pipe = trimmed.starts_with('|') || trimmed.ends_with('|');
    let content = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let content = content.strip_suffix('|').unwrap_or(content);
    let raw_segments = split_unescaped_pipe(content);
    if !has_outer_pipe && raw_segments.len() <= 1 {
        return None;
    }

    let segments: Vec<&str> = raw_segments.into_iter().map(str::trim).collect();
    (!segments.is_empty()).then_some(segments)
}

/// `content` split on its unescaped `|`; a `\|` stays in its segment,
/// backslash included.
fn split_unescaped_pipe(content: &str) -> Vec<&str> {
    let mut segments = Vec::with_capacity(8);
    let mut start = 0;
    let bytes = content.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
        } else if bytes[i] == b'|' {
            segments.push(&content[start..i]);
            start = i + 1;
            i += 1;
        } else {
            i += 1;
        }
    }
    segments.push(&content[start..]);
    segments
}

/// Whether `line` can be a table's header: pipe-separated segments, one of
/// them not empty.
pub(crate) fn is_table_header_line(line: &str) -> bool {
    parse_table_segments(line).is_some_and(|segments| segments.iter().any(|s| !s.is_empty()))
}

/// Whether one segment is `---`, `:---`, `---:` or `:---:` (any number of
/// dashes from three).
fn is_table_delimiter_segment(segment: &str) -> bool {
    let trimmed = segment.trim();
    if trimmed.is_empty() {
        return false;
    }
    let without_leading = trimmed.strip_prefix(':').unwrap_or(trimmed);
    let without_ends = without_leading.strip_suffix(':').unwrap_or(without_leading);
    without_ends.len() >= 3 && without_ends.chars().all(|c| c == '-')
}

/// Whether every segment of `line` is a delimiter segment.
pub(crate) fn is_table_delimiter_line(line: &str) -> bool {
    parse_table_segments(line)
        .is_some_and(|segments| segments.into_iter().all(is_table_delimiter_segment))
}

/// Where a source line sits relative to fenced code blocks: pipes inside a
/// fence that is not Markdown (`sh`, `rust`) are code, not a table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FenceKind {
    Outside,
    /// Inside a ```` ```md ```` or ```` ```markdown ```` fence.
    Markdown,
    /// Inside a fence with another info string, or none.
    Other,
}

/// The fence a line is in, fed one line at a time. A line indented four
/// spaces or more is no fence; blockquote markers are stripped first.
/// [`Self::kind`] is the context of the line about to be advanced over.
pub(crate) struct FenceTracker {
    state: Option<(char, usize, FenceKind)>,
}

impl FenceTracker {
    pub(crate) fn new() -> Self {
        Self { state: None }
    }

    pub(crate) fn advance(&mut self, raw_line: &str) {
        let leading_spaces = raw_line
            .as_bytes()
            .iter()
            .take_while(|byte| **byte == b' ')
            .count();
        if leading_spaces > 3 {
            return;
        }

        let trimmed = &raw_line[leading_spaces..];
        let fence_scan_text = strip_blockquote_prefix(trimmed);
        if let Some((marker, len)) = parse_fence_marker(fence_scan_text) {
            if let Some((open_char, open_len, _)) = self.state {
                if marker == open_char
                    && len >= open_len
                    && fence_scan_text[len..].trim().is_empty()
                {
                    self.state = None;
                }
            } else {
                let kind = if is_markdown_fence_info(fence_scan_text, len) {
                    FenceKind::Markdown
                } else {
                    FenceKind::Other
                };
                self.state = Some((marker, len, kind));
            }
        }
    }

    pub(crate) fn kind(&self) -> FenceKind {
        self.state.map_or(FenceKind::Outside, |(_, _, kind)| kind)
    }
}

/// The marker character and run length of a fence line (three or more
/// backticks or tildes), its indentation and quote markers stripped.
pub(crate) fn parse_fence_marker(line: &str) -> Option<(char, usize)> {
    let first = line.as_bytes().first().copied()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let len = line.bytes().take_while(|&b| b == first).count();
    if len < 3 {
        return None;
    }
    Some((first as char, len))
}

/// Whether the info string after a fence's `marker_len` marker characters
/// is `md` or `markdown`, in any case.
pub(crate) fn is_markdown_fence_info(trimmed_line: &str, marker_len: usize) -> bool {
    let info = trimmed_line
        .get(marker_len..)
        .unwrap_or_default()
        .split_whitespace()
        .next()
        .unwrap_or_default();
    info.eq_ignore_ascii_case("md") || info.eq_ignore_ascii_case("markdown")
}

/// `line` without its leading `>` blockquote markers.
pub(crate) fn strip_blockquote_prefix(line: &str) -> &str {
    let mut rest = line.trim_start();
    loop {
        let Some(stripped) = rest.strip_prefix('>') else {
            return rest;
        };
        rest = stripped.strip_prefix(' ').unwrap_or(stripped).trim_start();
    }
}

#[cfg(test)]
mod tests;
