// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use ratatui::text::Line;

use crate::table_detect::{
    is_table_delimiter_line, is_table_header_line, parse_table_segments, strip_blockquote_prefix,
    FenceKind, FenceTracker,
};

/// How a stream's Markdown is rendered: an answer's (its table fences
/// unwrapped) or a plan's.
pub(crate) type RenderMarkdown = fn(&str, Option<usize>, Option<&str>, &mut Vec<Line<'static>>);

/// Newline-gated accumulator that renders markdown and commits only fully
/// completed logical lines.
pub(crate) struct MarkdownStreamCollector {
    buffer: String,
    committed_line_count: usize,
    width: Option<usize>,
    /// The session's directory, local file links are shown relative to.
    cwd: Option<String>,
    render: RenderMarkdown,
}

impl MarkdownStreamCollector {
    pub fn new(width: Option<usize>, cwd: Option<String>, render: RenderMarkdown) -> Self {
        Self {
            buffer: String::new(),
            committed_line_count: 0,
            width,
            cwd,
            render,
        }
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.committed_line_count = 0;
    }

    pub fn push_delta(&mut self, delta: &str) {
        self.buffer.push_str(delta);
    }

    /// The Markdown received so far.
    pub fn source(&self) -> &str {
        &self.buffer
    }

    /// Render the full buffer and return only the newly completed logical lines
    /// since the last commit. When the buffer does not end with a newline, the
    /// final rendered line is considered incomplete and is not emitted. The
    /// rows of a table, or of a ```` ```md ```` fence, wait until it has ended
    /// ([`mutable_tail_start`]): a row arriving lays its table out again.
    pub fn commit_complete_lines(&mut self) -> Vec<Line<'static>> {
        let Some(last_newline_idx) = self.buffer.rfind('\n') else {
            return Vec::new();
        };
        let source = &self.buffer[..=last_newline_idx];
        let source = mutable_tail_start(source).map_or(source, |start| &source[..start]);
        let mut rendered: Vec<Line<'static>> = Vec::new();
        (self.render)(source, self.width, self.cwd.as_deref(), &mut rendered);
        let mut complete_line_count = rendered.len();
        if complete_line_count > 0
            && crate::render::line_utils::is_blank_line_spaces_only(
                &rendered[complete_line_count - 1],
            )
        {
            complete_line_count -= 1;
        }

        if self.committed_line_count >= complete_line_count {
            return Vec::new();
        }

        let out = rendered[self.committed_line_count..complete_line_count].to_vec();
        self.committed_line_count = complete_line_count;
        out
    }

    /// Finalize the stream: emit all remaining lines beyond the last commit.
    /// If the buffer does not end with a newline, a temporary one is appended
    /// for rendering.
    pub fn finalize_and_drain(&mut self) -> Vec<Line<'static>> {
        let mut source: String = self.buffer.clone();
        if !source.ends_with('\n') {
            source.push('\n');
        }

        let mut rendered: Vec<Line<'static>> = Vec::new();
        (self.render)(&source, self.width, self.cwd.as_deref(), &mut rendered);

        let out = if self.committed_line_count >= rendered.len() {
            Vec::new()
        } else {
            rendered[self.committed_line_count..].to_vec()
        };

        // Reset collector state for next stream.
        self.clear();
        out
    }
}

/// Where the source's rows may still change, as later lines arrive: a table
/// from its header until a blank line ends it, a last line that may be a
/// table's header, a last line that is a list marker alone (its item's text
/// may come on the next line, on the marker's row), and a ```` ```md ````
/// fence from its opening until a blank line after it closes (whether it
/// is unwrapped is decided at its end). Codex's stream (a later milestone)
/// keeps such a tail mutable on screen instead.
fn mutable_tail_start(source: &str) -> Option<usize> {
    let mut fences = FenceTracker::new();
    let mut table_start = None;
    let mut markdown_fence_start = None;
    // The start of the line before, when it may be a table's header.
    let mut previous_header = None;
    // The start of the line before, when it is a list marker alone.
    let mut bare_marker = None;
    let mut offset = 0;
    for raw_line in source.split_inclusive('\n') {
        let start = offset;
        offset += raw_line.len();
        let line = raw_line.trim_end_matches(['\n', '\r']);
        let fence = fences.kind();
        if fence == FenceKind::Outside && strip_blockquote_prefix(line).trim().is_empty() {
            table_start = None;
            markdown_fence_start = None;
            previous_header = None;
            bare_marker = None;
            continue;
        }
        let candidate = Some(strip_blockquote_prefix(line).trim())
            .filter(|text| fence != FenceKind::Other && parse_table_segments(text).is_some());
        if previous_header.is_some() && candidate.is_some_and(is_table_delimiter_line) {
            table_start = table_start.or(previous_header);
        }
        let is_header = candidate.is_some_and(is_table_header_line);
        previous_header = is_header.then_some(start);
        bare_marker = (fence == FenceKind::Outside && is_bare_list_marker(line)).then_some(start);
        fences.advance(line);
        if fence == FenceKind::Outside && fences.kind() == FenceKind::Markdown {
            markdown_fence_start = markdown_fence_start.or(Some(start));
        }
    }
    [
        table_start,
        markdown_fence_start,
        previous_header,
        bare_marker,
    ]
    .into_iter()
    .flatten()
    .min()
}

/// `-`, `*`, `+` or `1.`/`1)` with nothing after it but a task box, its
/// quote markers left out.
fn is_bare_list_marker(line: &str) -> bool {
    let text = strip_blockquote_prefix(line).trim();
    let marker = ["[ ]", "[x]", "[X]"]
        .iter()
        .find_map(|task| text.strip_suffix(task))
        .map_or(text, str::trim_end);
    match marker.strip_suffix(['.', ')']) {
        Some(number) => !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()),
        None => matches!(marker, "-" | "*" | "+"),
    }
}

#[cfg(test)]
pub(crate) fn simulate_stream_markdown_for_tests(
    deltas: &[&str],
    finalize: bool,
) -> Vec<Line<'static>> {
    let mut collector =
        MarkdownStreamCollector::new(None, None, crate::markdown::append_markdown_agent);
    let mut out = Vec::new();
    for d in deltas {
        collector.push_delta(d);
        if d.contains('\n') {
            out.extend(collector.commit_complete_lines());
        }
    }
    if finalize {
        out.extend(collector.finalize_and_drain());
    }
    out
}

#[cfg(test)]
mod tests;
