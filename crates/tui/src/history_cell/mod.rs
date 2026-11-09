// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! Transcript cells: the unit the chat widget shows, both the entries it
//! has committed to the terminal's scrollback and, transiently, the active
//! cell a running tool call keeps updating in place.

mod plan;
mod reports;
mod status;
mod thinking;
mod tool;

use std::any::Any;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::render::line_utils::prefix_lines;
use crate::render::renderable::Renderable;
use crate::style::user_message_style;
use crate::ui_consts::LIVE_PREFIX_COLS;
use crate::wrapping::{word_wrap_lines, RtOptions};

pub(crate) use plan::{new_plan_update, new_proposed_plan_stream};
pub(crate) use reports::{new_mcp_output, new_skills_output};
pub(crate) use status::new_status_output;
pub(crate) use thinking::ThinkingCell;
pub(crate) use tool::{new_tool_call, ToolCallCell, ToolResultView};

/// An entry of the conversation, rendered at a width.
pub(crate) trait HistoryCell: std::fmt::Debug + Send + Sync + Any {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>>;

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

    fn desired_transcript_height(&self, width: u16) -> u16 {
        Paragraph::new(Text::from(self.transcript_lines(width)))
            .wrap(Wrap { trim: false })
            .line_count(width)
            .try_into()
            .unwrap_or(0)
    }

    /// Whether the cell continues the streamed message before it, so no
    /// blank line separates them in the scrollback.
    fn is_stream_continuation(&self) -> bool {
        false
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
pub(crate) struct UserHistoryCell {
    pub message: String,
}

impl HistoryCell for UserHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
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
        let mut lines = vec![Line::from("").style(style)];
        lines.extend(prefix_lines(wrapped, "› ".bold().dim(), "  ".into()));
        lines.push(Line::from("").style(style));
        lines
    }
}

pub(crate) fn new_user_prompt(message: String) -> UserHistoryCell {
    UserHistoryCell { message }
}

#[derive(Debug)]
pub(crate) struct AgentMessageCell {
    lines: Vec<Line<'static>>,
    is_first_line: bool,
}

impl AgentMessageCell {
    pub(crate) fn new(lines: Vec<Line<'static>>, is_first_line: bool) -> Self {
        Self {
            lines,
            is_first_line,
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

    fn is_stream_continuation(&self) -> bool {
        !self.is_first_line
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

pub(crate) fn new_error_event(message: String) -> PlainHistoryCell {
    PlainHistoryCell::new(vec![vec![format!("■ {message}").red()].into()])
}

/// A divider between the work of a turn and its final answer, with how
/// long the turn worked.
#[derive(Debug)]
pub(crate) struct FinalMessageSeparator {
    elapsed_seconds: Option<u64>,
}

impl FinalMessageSeparator {
    pub(crate) fn new(elapsed_seconds: Option<u64>) -> Self {
        Self { elapsed_seconds }
    }
}

impl HistoryCell for FinalMessageSeparator {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        match self
            .elapsed_seconds
            .map(crate::status_indicator_widget::fmt_elapsed_compact)
        {
            Some(elapsed) => {
                let worked_for = format!("─ Worked for {elapsed} ─");
                let rest = (width as usize).saturating_sub(worked_for.width());
                vec![Line::from_iter([worked_for, "─".repeat(rest)]).dim()]
            }
            None => vec![Line::from_iter(["─".repeat(width as usize).dim()])],
        }
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
