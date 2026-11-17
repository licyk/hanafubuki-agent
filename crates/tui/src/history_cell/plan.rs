// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The cells of plan mode's proposed plan (its streamed rows, then its
//! Markdown) and of the task checklist.

use hanafubuki_agent::{AgentTaskPlanStep, AgentTaskPlanStepStatus};
use ratatui::style::{Style, Styled, Stylize};
use ratatui::text::Line;
use unicode_width::UnicodeWidthStr;

use super::markdown_cache::MarkdownRenderCache;
use super::{plain_lines, raw_lines_from_source, ActivityDisclosure, HistoryCell, StreamId};
use crate::markdown_render::ListSpacing;
use crate::render::line_utils::prefix_lines;
use crate::style::proposed_plan_style;
use crate::terminal_hyperlinks::{prefix_hyperlink_lines, visible_lines, HyperlinkLine};

/// The rendered checklist rows shown below its explanation when collapsed.
const PLAN_PREVIEW_LINES: usize = 5;

pub(crate) fn new_proposed_plan_stream(
    lines: Vec<HyperlinkLine>,
    is_stream_continuation: bool,
    stream: StreamId,
) -> ProposedPlanStreamCell {
    ProposedPlanStreamCell {
        lines,
        is_stream_continuation,
        stream,
    }
}

/// Rows of a streaming proposed plan, rendered at the width its stream
/// started at, with their links.
#[derive(Debug)]
pub(crate) struct ProposedPlanStreamCell {
    lines: Vec<HyperlinkLine>,
    is_stream_continuation: bool,
    stream: StreamId,
}

impl HistoryCell for ProposedPlanStreamCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        visible_lines(self.lines.clone())
    }

    fn display_hyperlink_lines(&self, _width: u16) -> Vec<HyperlinkLine> {
        self.lines.clone()
    }

    fn transcript_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.display_hyperlink_lines(width)
    }

    fn compact_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.display_hyperlink_lines(width)
    }

    fn retained_hyperlink_lines(&self, width: u16, _detailed: bool) -> Vec<HyperlinkLine> {
        self.display_hyperlink_lines(width)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        plain_lines(self.display_lines(0))
    }

    fn is_stream_continuation(&self) -> bool {
        self.is_stream_continuation
    }

    fn stream_id(&self) -> Option<StreamId> {
        Some(self.stream)
    }
}

/// A finished proposed plan: its Markdown, rendered for the width it is
/// shown at, in the plan's block as it streamed.
#[derive(Debug)]
pub(crate) struct ProposedPlanCell {
    source: String,
    cwd: String,
    rendered: MarkdownRenderCache,
}

impl ProposedPlanCell {
    pub(crate) fn new(source: String, cwd: String) -> Self {
        Self {
            source,
            cwd,
            rendered: MarkdownRenderCache::default(),
        }
    }

    fn render(&self, width: u16, list_spacing: ListSpacing) -> Vec<HyperlinkLine> {
        self.rendered.render(width, list_spacing, || {
            let mut body = vec![HyperlinkLine::from(Line::from(" "))];
            body.extend(crate::markdown::render_markdown_with_list_spacing(
                &self.source,
                Some(usize::from(width.saturating_sub(4)).max(1)),
                Some(&self.cwd),
                list_spacing,
            ));
            body.push(Line::from(" ").into());
            let style = proposed_plan_style();
            let mut lines = vec![
                HyperlinkLine::from(Line::from(vec!["• ".dim(), "Proposed Plan".bold()])),
                Line::from(" ").into(),
            ];
            lines.extend(
                prefix_hyperlink_lines(body, "  ".into(), "  ".into())
                    .into_iter()
                    .map(|line| line.style(style)),
            );
            lines
        })
    }
}

impl HistoryCell for ProposedPlanCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        visible_lines(self.display_hyperlink_lines(width))
    }

    fn retained_lines(&self, width: u16, detailed: bool) -> Vec<Line<'static>> {
        visible_lines(self.retained_hyperlink_lines(width, detailed))
    }

    fn display_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.render(width, ListSpacing::AfterMultiline)
    }

    fn transcript_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.display_hyperlink_lines(width)
    }

    fn compact_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.display_hyperlink_lines(width)
    }

    fn retained_hyperlink_lines(&self, width: u16, _detailed: bool) -> Vec<HyperlinkLine> {
        self.render(width, ListSpacing::Uniform)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        raw_lines_from_source(&self.source)
    }

    fn copy_source(&self) -> Option<String> {
        Some(self.source.clone())
    }
}

#[derive(Debug)]
pub(crate) struct PlanUpdateCell {
    activity_id: String,
    explanation: Option<String>,
    plan: Vec<AgentTaskPlanStep>,
}

impl PlanUpdateCell {
    pub(crate) fn from_tool(
        activity_id: String,
        explanation: Option<String>,
        plan: Vec<AgentTaskPlanStep>,
    ) -> Self {
        Self {
            activity_id,
            explanation,
            plan,
        }
    }
    /// `• Updated Plan · 1/3 complete`, the same collapsed and expanded.
    fn header(&self) -> Line<'static> {
        let completed = self
            .plan
            .iter()
            .filter(|step| step.status == AgentTaskPlanStepStatus::Completed)
            .count();
        Line::from(vec![
            "• ".dim(),
            "Updated Plan".bold(),
            format!(" · {completed}/{} complete", self.plan.len()).dim(),
        ])
    }

    fn note(&self) -> Option<&str> {
        self.explanation
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }

    /// The header with `body` under its `└`.
    fn with_body(&self, mut body: Vec<Line<'static>>) -> Vec<Line<'static>> {
        if self.plan.is_empty() {
            body.push(Line::from("(no steps provided)".dim().italic()));
        }
        let mut lines = vec![self.header()];
        lines.extend(prefix_lines(body, "  └ ".dim(), "    ".into()));
        lines
    }

    fn lines(&self, width: u16, expanded: bool) -> Vec<Line<'static>> {
        let mut body = Vec::new();
        if let Some(note) = self.note() {
            let wrap_width = width.saturating_sub(4).max(1) as usize;
            body.extend(
                textwrap::wrap(note, wrap_width)
                    .into_iter()
                    .map(|s| s.to_string().dim().italic().into()),
            );
        }
        let steps = self
            .plan
            .iter()
            .flat_map(|AgentTaskPlanStep { step, status }| step_lines(status, step, width));
        body.extend(steps.take(if expanded {
            usize::MAX
        } else {
            PLAN_PREVIEW_LINES
        }));
        self.with_body(body)
    }
}

impl HistoryCell for PlanUpdateCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.lines(width, true)
    }

    fn compact_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.lines(width, false)
    }

    fn activity_ids(&self) -> Vec<String> {
        vec![self.activity_id.clone()]
    }

    fn activity_disclosure(&self, width: u16) -> Option<ActivityDisclosure> {
        (self
            .plan
            .iter()
            .flat_map(|step| step_lines(&step.status, &step.step, width))
            .take(PLAN_PREVIEW_LINES + 1)
            .count()
            > PLAN_PREVIEW_LINES)
            .then_some(ActivityDisclosure::Generic)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from("Updated Plan")];
        if let Some(note) = self.note() {
            lines.extend(raw_lines_from_source(note));
        }
        if self.plan.is_empty() {
            lines.push(Line::from("(no steps provided)"));
        }
        for AgentTaskPlanStep { step, status } in &self.plan {
            let marker = match status {
                AgentTaskPlanStepStatus::Completed => "[x]",
                AgentTaskPlanStepStatus::InProgress => "[~]",
                AgentTaskPlanStepStatus::Pending => "[ ]",
            };
            lines.push(Line::from(format!("{marker} {step}")));
        }
        lines
    }
}

/// A step's checkbox and text, wrapped under the `└` at `width`.
fn step_lines(status: &AgentTaskPlanStepStatus, text: &str, width: u16) -> Vec<Line<'static>> {
    let (box_str, step_style) = match status {
        AgentTaskPlanStepStatus::Completed => ("✔ ", Style::default().crossed_out().dim()),
        AgentTaskPlanStepStatus::InProgress => ("□ ", Style::default().cyan().bold()),
        AgentTaskPlanStepStatus::Pending => ("□ ", Style::default().dim()),
    };
    let wrap_width = (width as usize)
        .saturating_sub(4)
        .saturating_sub(box_str.width())
        .max(1);
    let step_text = textwrap::wrap(text, wrap_width)
        .into_iter()
        .map(|s| s.to_string().set_style(step_style).into())
        .collect();
    prefix_lines(step_text, box_str.into(), "  ".into())
}
