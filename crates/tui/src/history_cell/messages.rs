// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The messages of the conversation: yours, and the model's answer.
//!
//! An answer streams as [`AgentMessageCell`]s, rows rendered at the width
//! the stream started at and committed as they complete. When the stream
//! ends, the app replaces them with one [`AgentMarkdownCell`] holding the
//! answer's Markdown, which renders itself for the width it is shown at
//! (Codex's consolidation): a resize lays its tables, lists and prose out
//! again. A replayed answer is an [`AgentMarkdownCell`] from the start.

use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ratatui::style::Stylize;
use ratatui::text::Line;

use super::markdown_cache::MarkdownRenderCache;
use super::{
    next_activity_id, plain_lines, raw_lines_from_source, ActivityDisclosure, HistoryCell,
};
use crate::markdown_render::ListSpacing;
use crate::render::line_utils::prefix_lines;
use crate::style::user_message_style;
use crate::ui_consts::LIVE_PREFIX_COLS;
use crate::user_message::UserMessage;
use crate::wrapping::{word_wrap_lines, RtOptions};

/// The wrapped rows of a message of yours shown collapsed; a longer one has
/// a control.
const USER_MESSAGE_MAX_ROWS: usize = 12;

#[derive(Debug)]
pub(crate) struct UserHistoryCell {
    /// The text as sent, image labels and all.
    pub message: String,
    /// `[Image #N: name]` for each of its images.
    images: Vec<String>,
    activity_id: String,
}

impl UserHistoryCell {
    /// The message wrapped at `width`, behind its `›`, then a row for each
    /// image.
    fn rows(&self, width: u16) -> Vec<Line<'static>> {
        let wrap_width = width
            .saturating_sub(
                LIVE_PREFIX_COLS + 1, /* keep a one-column right margin for wrapping */
            )
            .max(1);
        let style = user_message_style();
        let wrapped = word_wrap_lines(
            self.message.split('\n').map(|l| Line::from(l).style(style)),
            // Wrap algorithm matches textarea.rs.
            RtOptions::new(usize::from(wrap_width))
                .wrap_algorithm(textwrap::WrapAlgorithm::FirstFit),
        );
        let mut rows = prefix_lines(wrapped, "› ".bold().dim(), "  ".into());
        rows.extend(
            self.images
                .iter()
                .map(|image| Line::from(vec!["  ".into(), image.clone().cyan()]).style(style)),
        );
        rows
    }

    /// `rows` between a blank row above and below, in the message's style.
    fn framed(rows: Vec<Line<'static>>) -> Vec<Line<'static>> {
        let style = user_message_style();
        let mut lines = vec![Line::from("").style(style)];
        lines.extend(rows);
        lines.push(Line::from("").style(style));
        lines
    }
}

impl HistoryCell for UserHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        Self::framed(self.rows(width))
    }

    /// A long message shows its first rows.
    fn compact_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut rows = self.rows(width);
        rows.truncate(USER_MESSAGE_MAX_ROWS);
        Self::framed(rows)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        raw_lines_from_source(&self.message)
    }

    fn copy_source(&self) -> Option<String> {
        Some(self.message.clone())
    }

    fn activity_ids(&self) -> Vec<String> {
        vec![self.activity_id.clone()]
    }

    fn activity_disclosure(&self, width: u16) -> Option<ActivityDisclosure> {
        let rows = self.rows(width).len();
        (rows > USER_MESSAGE_MAX_ROWS)
            .then(|| ActivityDisclosure::OutputLines(rows - USER_MESSAGE_MAX_ROWS))
    }
}

pub(crate) fn new_user_prompt(message: &UserMessage) -> UserHistoryCell {
    user_prompt(message.text.clone(), message.image_rows())
}

/// A message as sent, `images` its images' rows.
pub(crate) fn user_prompt(message: String, images: Vec<String>) -> UserHistoryCell {
    UserHistoryCell {
        message,
        images,
        activity_id: next_activity_id("user:"),
    }
}

/// One stream of an answer or a plan: its cells until it is consolidated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StreamId(u64);

impl StreamId {
    pub(crate) fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// Rows of a streaming answer, rendered at the width its stream started at.
#[derive(Debug)]
pub(crate) struct AgentMessageCell {
    lines: Vec<Line<'static>>,
    is_first_line: bool,
    stream: StreamId,
}

impl AgentMessageCell {
    pub(crate) fn new(lines: Vec<Line<'static>>, is_first_line: bool, stream: StreamId) -> Self {
        Self {
            lines,
            is_first_line,
            stream,
        }
    }
}

impl HistoryCell for AgentMessageCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        word_wrap_lines(
            &self.lines,
            RtOptions::new(width as usize)
                .initial_indent(if self.is_first_line {
                    "• ".dim().into()
                } else {
                    "  ".into()
                })
                .subsequent_indent("  ".into()),
        )
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        plain_lines(self.lines.clone())
    }

    fn is_stream_continuation(&self) -> bool {
        !self.is_first_line
    }

    fn stream_id(&self) -> Option<StreamId> {
        Some(self.stream)
    }
}

/// A finished answer: its Markdown, rendered for the width it is shown at.
/// Scrollback writes it with Codex's spacing of the terminal's scrollback;
/// a view that lays entries out again spaces its lists uniformly.
///
/// The session's directory is kept with it, so its local file links read
/// the same after a resize.
#[derive(Debug)]
pub(crate) struct AgentMarkdownCell {
    source: String,
    cwd: String,
    rendered: MarkdownRenderCache,
}

impl AgentMarkdownCell {
    /// `source` is the answer's Markdown as the model wrote it, not its
    /// rendered rows.
    pub(crate) fn new(source: String, cwd: String) -> Self {
        Self {
            source,
            cwd,
            rendered: MarkdownRenderCache::default(),
        }
    }

    fn render(&self, width: u16, list_spacing: ListSpacing) -> Vec<Line<'static>> {
        self.rendered.render(width, list_spacing, || {
            // Two columns for the `• ` or `  ` in front of each row.
            let wrap_width = usize::from(width.saturating_sub(2)).max(1);
            let lines = crate::markdown::render_markdown_agent_with_list_spacing(
                &self.source,
                Some(wrap_width),
                Some(&self.cwd),
                list_spacing,
            );
            // As the stream's rows show (`AgentMessageCell`), so the
            // answer keeps its rows when its stream is consolidated.
            word_wrap_lines(
                &lines,
                RtOptions::new(usize::from(width))
                    .initial_indent("• ".dim().into())
                    .subsequent_indent("  ".into()),
            )
        })
    }
}

impl HistoryCell for AgentMarkdownCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.render(width, ListSpacing::AfterMultiline)
    }

    fn retained_lines(&self, width: u16, _detailed: bool) -> Vec<Line<'static>> {
        self.render(width, ListSpacing::Uniform)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        raw_lines_from_source(&self.source)
    }

    fn copy_source(&self) -> Option<String> {
        Some(self.source.clone())
    }
}

/// The span of `cells` that `stream`'s cells take: from its first to its
/// last, with whatever was inserted among them.
pub(crate) fn stream_span(
    cells: &[Arc<dyn HistoryCell>],
    stream: StreamId,
) -> Option<Range<usize>> {
    let first = cells
        .iter()
        .position(|cell| cell.stream_id() == Some(stream))?;
    let last = cells
        .iter()
        .rposition(|cell| cell.stream_id() == Some(stream))?;
    Some(first..last + 1)
}

/// `stream`'s cells in `span` replaced by `consolidated`; a cell inserted
/// among them follows it.
pub(crate) fn splice_stream(
    cells: &mut Vec<Arc<dyn HistoryCell>>,
    span: Range<usize>,
    stream: StreamId,
    consolidated: Arc<dyn HistoryCell>,
) {
    let others = cells[span.clone()]
        .iter()
        .filter(|cell| cell.stream_id() != Some(stream))
        .cloned()
        .collect::<Vec<_>>();
    cells.splice(span, std::iter::once(consolidated).chain(others));
}

#[cfg(test)]
mod tests;
