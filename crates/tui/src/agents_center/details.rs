// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The details pane (Codex's `agents_overview_view.rs` `render_details` and
//! `agents_overview_details.rs`): the selected session's title and status,
//! model, reasoning effort, directory, branch and token usage; what the
//! most urgent of it and its children waits on you for and what it does
//! now; its children, each with its status, which `c` (or a click) selects
//! to open or stop; and the latest answer and the message that asked for
//! it. From 90 columns it stands beside the list; narrower, it takes the
//! list's place while a child is selected.

use hanafubuki_agent::AgentSessionSummary;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use super::render::{status_look, title_of};
use super::status::{Group, Status};
use super::{AgentsCenter, Row};
use crate::agent_bridge::Waiting;
use crate::appearance::display_directory;
use crate::history_cell::format_tokens_compact;
use crate::text_formatting::{center_truncate_path, truncate_text};
use crate::wrapping::word_wrap_lines;

/// The width from which the details stand beside the list.
pub(super) const DETAILS_FROM: u16 = 90;
pub(super) const DETAILS_WIDTH: u16 = 38;
/// The children shown at once; the list scrolls with the selection.
const CHILD_ROWS: usize = 6;
/// The lines the prompt keeps.
const PROMPT_LINES: usize = 2;

impl AgentsCenter {
    /// Draws the selected session's details over `area`, and keeps where
    /// its children are for the mouse.
    pub(super) fn render_details(&mut self, area: Rect, buf: &mut Buffer) {
        self.layout.children.clear();
        let Some(row) = self.selected_row() else {
            return;
        };
        let width = usize::from(area.width);
        let mut lines = self.head_lines(&row, width);
        let children = self.child_lines(&row, width);
        let child_rows: Vec<(usize, _)> = children
            .rows
            .iter()
            .map(|(index, id)| (lines.len() + index, *id))
            .collect();
        lines.extend(children.lines);
        lines.extend(self.urgent_lines(&row, width));
        let height = usize::from(area.height);
        let (prompt, answer) = self.exchange_lines(&row, width);
        let room = height.saturating_sub(lines.len() + prompt.len());
        // Its label and at least a line, or nothing.
        if room >= 3 {
            lines.extend(clipped(answer, room));
        }
        lines.extend(prompt);
        for (index, id) in child_rows {
            if let Ok(offset) = u16::try_from(index) {
                if offset < area.height {
                    let line = Rect::new(area.x, area.y + offset, area.width, 1);
                    self.layout.children.push((line, id));
                }
            }
        }
        Paragraph::new(lines).render(area, buf);
    }

    /// The title, status and facts.
    fn head_lines(&self, row: &Row, width: usize) -> Vec<Line<'static>> {
        let summary = &row.summary;
        let (label, dot, style) = status_look(row.status);
        let effort = summary
            .reasoning_effort
            .as_ref()
            .map_or_else(|| "default".to_string(), |effort| effort.to_string());
        let mut lines = vec![
            Line::from("Session details".bold()),
            Line::default(),
            Line::from(truncate_text(&title_of(summary), width).bold()),
            Line::from(vec![dot, " ".into(), Span::styled(label, style)]),
            Line::default(),
            fact("Model: ", &summary.model, width),
            fact("Reasoning: ", &effort, width),
        ];
        if let Some(branch) = &summary.branch {
            lines.push(fact("Branch: ", branch, width));
        }
        if let Some(usage) = &summary.usage {
            let tokens = format!(
                "{} in · {} out",
                format_tokens_compact(usage.cumulative_input_tokens),
                format_tokens_compact(usage.cumulative_output_tokens),
            );
            lines.push(fact("Tokens: ", &tokens, width));
        }
        if let Some(cwd) = &summary.cwd {
            lines.extend([Line::default(), Line::from("Directory".dim())]);
            lines.push(Line::from(center_truncate_path(
                &display_directory(cwd),
                width,
            )));
        }
        lines
    }

    /// What the most urgent of the session and its children waits on you
    /// for, and what it does now.
    fn urgent_lines(&self, row: &Row, width: usize) -> Vec<Line<'static>> {
        let source = self.most_urgent(row);
        let live = self.live.get(&source.session_id);
        let mut lines = Vec::new();
        let child = source.session_id != row.summary.session_id;
        let needs_you =
            Status::of(source, [], |session| self.waits_approval(session)).group == Group::NeedsYou;
        if needs_you {
            lines.extend([Line::default(), Line::from("Needs you".red())]);
            if child {
                lines.push(fact("Subagent: ", &title_of(source), width));
            }
            let waiting = match live.and_then(|live| live.waiting.as_ref()) {
                Some(Waiting::Approval(what)) => format!("Waiting for approval: {what}"),
                Some(Waiting::Question(question)) => format!("Asks: {question}"),
                None if row.status.error && !child => "Its last turn failed.".to_string(),
                None => "Waiting for your answer.".to_string(),
            };
            lines.extend(word_wrap_lines([Line::from(waiting)], width));
            lines.push(Line::from("Open it to answer.".dim()));
        }
        if let Some(activity) = live.and_then(|live| live.activity.as_ref()) {
            lines.extend([Line::default(), Line::from("Latest activity".dim())]);
            if child && !needs_you {
                lines.push(fact("Subagent: ", &title_of(source), width));
            }
            lines.push(Line::from(truncate_text(activity, width)));
        }
        lines
    }

    /// The children with their status, the one selected marked, at most
    /// [`CHILD_ROWS`] of them around it; and where each is among the lines.
    fn child_lines(&self, row: &Row, width: usize) -> ChildLines {
        let children = self.children_of(row.summary.session_id);
        let mut lines = Vec::new();
        let mut rows = Vec::new();
        if children.is_empty() {
            return ChildLines { lines, rows };
        }
        lines.extend([
            Line::default(),
            Line::from(format!("Subagents and reviews  {}", children.len()).dim()),
        ]);
        let selected = self
            .child_selected
            .and_then(|id| children.iter().position(|child| child.session_id == id));
        let first = selected
            .map_or(0, |index| index.saturating_sub(CHILD_ROWS - 1))
            .min(children.len().saturating_sub(CHILD_ROWS));
        for (index, child) in children.iter().enumerate().skip(first).take(CHILD_ROWS) {
            rows.push((lines.len(), child.session_id));
            lines.push(self.child_line(child, selected == Some(index), width));
        }
        let hidden = children.len() - first - CHILD_ROWS.min(children.len() - first);
        if hidden > 0 || first > 0 {
            lines.push(Line::from(format!("… {} more", hidden + first).dim()));
        }
        ChildLines { lines, rows }
    }

    fn child_line(
        &self,
        child: &AgentSessionSummary,
        selected: bool,
        width: usize,
    ) -> Line<'static> {
        let status = Status::of(child, [], |session| self.waits_approval(session));
        let (label, dot, style) = status_look(Status {
            error: false,
            ..status
        });
        let marker = if selected { "› ".bold() } else { "  ".into() };
        let room = width.saturating_sub(5 + label.len());
        let mut title: Span<'static> = truncate_text(&title_of(child), room).into();
        if selected {
            title = title.bold();
        }
        Line::from(vec![
            marker,
            dot,
            " ".into(),
            title,
            "  ".into(),
            Span::styled(label, style),
        ])
    }

    /// The prompt (at most [`PROMPT_LINES`]) and the latest answer, as read
    /// for the session; nothing until they are.
    fn exchange_lines(&self, row: &Row, width: usize) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
        let Some((_, exchange)) = self.exchanges.get(&row.summary.session_id) else {
            return (Vec::new(), Vec::new());
        };
        let wrap = |text: &str| -> Vec<Line<'static>> {
            word_wrap_lines(text.lines().map(|line| Line::from(line.to_string())), width)
        };
        let mut answer = Vec::new();
        if let Some(text) = &exchange.answer {
            answer.extend([Line::default(), Line::from("Last message".dim())]);
            answer.extend(wrap(text));
        }
        let mut prompt = vec![Line::default(), Line::from("Prompt".dim())];
        match &exchange.prompt {
            Some(text) => prompt.extend(clipped(wrap(text), PROMPT_LINES)),
            None => prompt.push(Line::from("No prompt available.".dim())),
        }
        (prompt, answer)
    }
}

struct ChildLines {
    lines: Vec<Line<'static>>,
    /// Each child's line among `lines`.
    rows: Vec<(usize, hanafubuki_agent::AgentSessionId)>,
}

fn fact(name: &'static str, value: &str, width: usize) -> Line<'static> {
    let room = width.saturating_sub(name.len());
    Line::from(vec![name.dim(), truncate_text(value, room).into()])
}

/// At most `room` of `lines`, the last one `…` when some are left out.
fn clipped(mut lines: Vec<Line<'static>>, room: usize) -> Vec<Line<'static>> {
    if lines.len() > room {
        lines.truncate(room);
        if let Some(last) = lines.last_mut() {
            *last = Line::from("…".dim());
        }
    }
    lines
}
