// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! Transcript cells: the unit the chat widget shows. Most are written once;
//! a thinking or a tool call is a [`LiveCell`] that changes in place. A cell
//! renders for the width it is shown at; an answer's or a plan's streamed
//! rows give way to a cell of its Markdown when its stream ends
//! (`messages`).

mod context;
mod live;
mod markdown_cache;
mod messages;
mod plan;
mod reports;
mod status;
mod thinking;
mod tool;

use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::render::renderable::Renderable;
use crate::status_indicator_widget::{fmt_elapsed_compact, fmt_elapsed_tenths};
use crate::terminal_hyperlinks::{plain_hyperlink_lines, HyperlinkLine};

pub(crate) use context::new_context_output;
pub(crate) use live::LiveCell;
pub(crate) use messages::{
    insert_before, new_user_prompt, splice_stream, stream_span, user_prompt, AgentMarkdownCell,
    AgentMessageCell, StreamId, StreamTailCell, UserHistoryCell,
};
pub(crate) use plan::{new_plan_update, new_proposed_plan_stream, ProposedPlanCell};
pub(crate) use reports::{new_mcp_output, new_skills_output};
pub(crate) use status::{format_tokens_compact, new_status_output, permission_mode_name};
pub(crate) use thinking::{ThinkingCell, THINKING_ACTIVITY};
pub(crate) use tool::{explores, new_tool_call, new_user_shell_command, ToolCallCell};

/// How the transcript view shows the cells: styled, or as the plain text
/// the cells were made from, for copying.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HistoryRenderMode {
    Rich,
    Raw,
}

/// What an activity's details add to its compact presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActivityDisclosure {
    Generic,
    /// The output lines the compact presentation leaves out.
    OutputLines(usize),
}

/// A new activity id, `{prefix}{n}`, for an entry without an identity of
/// its own: disclosure lasts a run and is never persisted.
fn next_activity_id(prefix: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("{prefix}{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// The lines' text without its styles.
pub(crate) fn plain_lines(lines: impl IntoIterator<Item = Line<'static>>) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| {
            Line::from(
                line.spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
                    .collect::<String>(),
            )
        })
        .collect()
}

/// One line per line of `source`, without a trailing empty one.
pub(crate) fn raw_lines_from_source(source: &str) -> Vec<Line<'static>> {
    let source = source.strip_suffix('\n').unwrap_or(source);
    if source.is_empty() {
        return Vec::new();
    }
    source
        .split('\n')
        .map(|line| Line::from(line.to_string()))
        .collect()
}

/// An entry of the conversation, rendered at a width.
pub(crate) trait HistoryCell: std::fmt::Debug + Send + Sync + Any {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>>;

    /// The cell's text as plain, unwrapped lines, for the raw presentation.
    fn raw_lines(&self) -> Vec<Line<'static>>;

    fn display_lines_for_mode(&self, width: u16, mode: HistoryRenderMode) -> Vec<Line<'static>> {
        match mode {
            HistoryRenderMode::Rich => self.display_lines(width),
            HistoryRenderMode::Raw => self.raw_lines(),
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        Paragraph::new(Text::from(self.display_lines(width)))
            .wrap(Wrap { trim: false })
            .line_count(width)
            .try_into()
            .unwrap_or(0)
    }

    /// The cell as the transcript pager shows it, which may show more than
    /// the scrollback (a collapsed cell's text).
    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.display_lines(width)
    }

    /// The compact presentation of the transcript view, which can reveal an
    /// activity's details in place.
    fn compact_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.display_lines(width)
    }

    /// The rich presentation in a view that lays its entries out again
    /// (Fullscreen, the transcript overlay), rather than writing them into
    /// the terminal's scrollback.
    fn retained_lines(&self, width: u16, detailed: bool) -> Vec<Line<'static>> {
        if detailed {
            self.transcript_lines(width)
        } else {
            self.compact_lines(width)
        }
    }

    /// An activity's details, revealed in the compact presentation.
    fn expanded_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.transcript_lines(width)
    }

    /// Stable identities of the activities the cell shows, which keep a
    /// disclosure while the cell changes. Empty for a cell with no details
    /// to disclose.
    fn activity_ids(&self) -> Vec<String> {
        Vec::new()
    }

    /// What the details add, or `None` when the compact presentation
    /// already shows everything.
    fn activity_disclosure(&self, _width: u16) -> Option<ActivityDisclosure> {
        Some(ActivityDisclosure::Generic)
    }

    /// Each presentation's lines with their terminal hyperlinks. Only a
    /// cell that renders Markdown has links; it overrides these, and its
    /// plain presentations are their visible lines.
    fn display_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        plain_hyperlink_lines(self.display_lines(width))
    }

    fn transcript_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        plain_hyperlink_lines(self.transcript_lines(width))
    }

    fn compact_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        plain_hyperlink_lines(self.compact_lines(width))
    }

    fn retained_hyperlink_lines(&self, width: u16, detailed: bool) -> Vec<HyperlinkLine> {
        plain_hyperlink_lines(self.retained_lines(width, detailed))
    }

    /// The source text the cell was rendered from, for copying it whole.
    #[allow(dead_code, reason = "`/copy` (P4) copies a cell's source")]
    fn copy_source(&self) -> Option<String> {
        None
    }

    /// Raised by every change of a cell that changes in place, so a layout
    /// of an earlier revision is not reused.
    fn revision(&self) -> u64 {
        0
    }

    /// A tick that changes while the cell's lines change with time (a
    /// spinner, a timer), so a cached layout of it is refreshed.
    fn transcript_animation_tick(&self) -> Option<u64> {
        None
    }

    /// Whether the cell continues the streamed message before it, so no
    /// blank line separates them in the scrollback.
    fn is_stream_continuation(&self) -> bool {
        false
    }

    /// The stream whose rows the cell holds, until the stream ends and its
    /// cells are replaced by one rendered from its source.
    fn stream_id(&self) -> Option<StreamId> {
        None
    }
}

/// The lines of an entry of the transcript view, with their hyperlinks:
/// the full transcript when `detailed`, otherwise the compact or the raw
/// presentation (no links); `retained` in a view that lays its entries out
/// again.
pub(crate) fn transcript_entry_lines(
    cell: &dyn HistoryCell,
    width: u16,
    detailed: bool,
    mode: HistoryRenderMode,
    retained: bool,
) -> Vec<HyperlinkLine> {
    if retained && (detailed || mode == HistoryRenderMode::Rich) {
        cell.retained_hyperlink_lines(width, detailed)
    } else if detailed {
        cell.transcript_hyperlink_lines(width)
    } else if mode == HistoryRenderMode::Rich {
        cell.compact_hyperlink_lines(width)
    } else {
        plain_hyperlink_lines(cell.display_lines_for_mode(width, mode))
    }
}

impl dyn HistoryCell {
    pub(crate) fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Renderable for Box<dyn HistoryCell> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let lines = self.display_lines(area.width);
        let y = if area.height == 0 {
            0
        } else {
            let overflow = lines.len().saturating_sub(usize::from(area.height));
            u16::try_from(overflow).unwrap_or(u16::MAX)
        };
        Paragraph::new(Text::from(lines))
            .scroll((y, 0))
            .render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        HistoryCell::desired_height(self.as_ref(), width)
    }
}

#[derive(Debug)]
pub(crate) struct PlainHistoryCell {
    lines: Vec<Line<'static>>,
}

impl PlainHistoryCell {
    pub(crate) fn new(lines: Vec<Line<'static>>) -> Self {
        Self { lines }
    }
}

impl HistoryCell for PlainHistoryCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.lines.clone()
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        plain_lines(self.lines.clone())
    }
}

/// The session's header card: the application, the model and the working
/// directory.
#[derive(Debug)]
pub(crate) struct SessionHeaderHistoryCell {
    version: &'static str,
    model: String,
    directory: String,
}

const SESSION_HEADER_MAX_INNER_WIDTH: usize = 56; // Just an eyeballed value

impl SessionHeaderHistoryCell {
    pub(crate) fn new(model: String, directory: String, version: &'static str) -> Self {
        Self {
            version,
            model,
            directory,
        }
    }
}

impl HistoryCell for SessionHeaderHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        if width < 4 {
            return Vec::new();
        }
        let inner_width = (width.saturating_sub(4) as usize).min(SESSION_HEADER_MAX_INNER_WIDTH);
        const DIR_LABEL: &str = "directory:";
        let label_width = DIR_LABEL.len();
        let title = vec![
            Span::from(">_ ").dim(),
            Span::from("hanafubuki-agent").bold(),
            Span::from(" ").dim(),
            Span::from(format!("(v{})", self.version)).dim(),
        ];
        let model = vec![
            Span::from(format!("{:<label_width$} ", "model:")).dim(),
            Span::from(self.model.clone()),
        ];
        let dir_prefix = format!("{DIR_LABEL:<label_width$} ");
        let dir_max_width = inner_width.saturating_sub(dir_prefix.width());
        let directory = if self.directory.width() > dir_max_width {
            crate::text_formatting::center_truncate_path(&self.directory, dir_max_width)
        } else {
            self.directory.clone()
        };
        with_border(vec![
            Line::from(title),
            Line::default(),
            Line::from(model),
            Line::from(vec![Span::from(dir_prefix).dim(), Span::from(directory)]),
        ])
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        vec![
            Line::from(format!("hanafubuki-agent (v{})", self.version)),
            Line::from(format!("model: {}", self.model)),
            Line::from(format!("directory: {}", self.directory)),
        ]
    }
}

/// Render `lines` inside a border sized to the widest span in the content.
fn with_border(lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    let content_width = lines.iter().map(Line::width).max().unwrap_or(0);
    let border_inner_width = content_width + 2;
    let mut out = Vec::with_capacity(lines.len() + 2);
    out.push(vec![format!("╭{}╮", "─".repeat(border_inner_width)).dim()].into());
    for line in lines {
        let used_width = line.width();
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(line.spans.len() + 3);
        spans.push(Span::from("│ ").dim());
        spans.extend(line);
        if used_width < content_width {
            spans.push(Span::from(" ".repeat(content_width - used_width)).dim());
        }
        spans.push(Span::from(" │").dim());
        out.push(Line::from(spans));
    }
    out.push(vec![format!("╰{}╯", "─".repeat(border_inner_width)).dim()].into());
    out
}

pub(crate) fn new_info_event(message: String, hint: Option<String>) -> PlainHistoryCell {
    let mut line = vec!["• ".dim(), message.into()];
    if let Some(hint) = hint {
        line.push(" ".into());
        line.push(hint.dark_gray());
    }
    PlainHistoryCell::new(vec![line.into()])
}

/// What the session's background work reported (a `WakeEvents` item): one
/// line per event.
pub(crate) fn new_background_updates(summaries: Vec<String>) -> PlainHistoryCell {
    let mut lines: Vec<Line<'static>> = vec![vec!["• ".dim(), "Background updates".into()].into()];
    lines.extend(summaries.into_iter().enumerate().map(|(index, summary)| {
        let branch = if index == 0 { "  └ " } else { "    " };
        Line::from(vec![branch.dim(), summary.dim()])
    }));
    PlainHistoryCell::new(lines)
}

pub(crate) fn new_error_event(message: String) -> PlainHistoryCell {
    PlainHistoryCell::new(vec![vec![format!("■ {message}").red()].into()])
}

pub(crate) fn new_warning_event(message: String) -> PlainHistoryCell {
    PlainHistoryCell::new(vec![vec![format!("⚠ {message}").yellow()].into()])
}

/// A divider between the work of a turn and its final answer, with how
/// long the turn worked and, when it thought, how long it thought.
#[derive(Debug)]
pub(crate) struct FinalMessageSeparator {
    elapsed_seconds: Option<u64>,
    thought: Option<Duration>,
}

impl FinalMessageSeparator {
    pub(crate) fn new(elapsed_seconds: Option<u64>, thought: Option<Duration>) -> Self {
        Self {
            elapsed_seconds,
            thought,
        }
    }

    /// `Worked for 25s · thought for 20.7s`.
    fn label(&self) -> Option<String> {
        let elapsed = fmt_elapsed_compact(self.elapsed_seconds?);
        Some(match self.thought {
            Some(thought) => format!(
                "Worked for {elapsed} · thought for {}",
                fmt_elapsed_tenths(thought)
            ),
            None => format!("Worked for {elapsed}"),
        })
    }
}

impl HistoryCell for FinalMessageSeparator {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        match self.label() {
            Some(label) => {
                let worked_for = format!("─ {label} ─");
                let rest = (width as usize).saturating_sub(worked_for.width());
                vec![Line::from_iter([worked_for, "─".repeat(rest)]).dim()]
            }
            None => vec![Line::from_iter(["─".repeat(width as usize).dim()])],
        }
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        self.label()
            .map(|label| vec![Line::from(label)])
            .unwrap_or_default()
    }
}

/// How the person answered an approval, as the transcript keeps it.
pub(crate) fn new_approval_decision_cell(
    summary: &str,
    approved: Option<bool>,
) -> PlainHistoryCell {
    let (symbol, verb) = match approved {
        Some(true) => ("✔ ".green(), "approved".bold()),
        Some(false) => ("✗ ".red(), "did not approve".bold()),
        None => ("✗ ".red(), "cancelled".bold()),
    };
    PlainHistoryCell::new(vec![Line::from(vec![
        symbol,
        "You ".into(),
        verb,
        " ".into(),
        Span::from(summary.to_string()).dim(),
    ])])
}

#[cfg(test)]
mod tests;
