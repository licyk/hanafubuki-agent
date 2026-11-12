// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The cells of plan mode's proposed plan and of the task checklist.

use hanafubuki_agent::{AgentTaskPlanSnapshot, AgentTaskPlanStep, AgentTaskPlanStepStatus};
use ratatui::style::{Style, Styled, Stylize};
use ratatui::text::Line;
use unicode_width::UnicodeWidthStr;

use super::{
    next_activity_id, plain_lines, raw_lines_from_source, ActivityDisclosure, HistoryCell,
};
use crate::render::line_utils::prefix_lines;

/// The steps a collapsed checklist shows.
const PLAN_PREVIEW_STEPS: usize = 3;

/// The task checklist the model keeps with `update_plan`, as it stood at
/// one update.
pub(crate) fn new_plan_update(snapshot: &AgentTaskPlanSnapshot) -> PlanUpdateCell {
    PlanUpdateCell {
        activity_id: next_activity_id("plan:"),
        explanation: snapshot.explanation.clone(),
        plan: snapshot.plan.clone(),
    }
}

pub(crate) fn new_proposed_plan_stream(
    lines: Vec<Line<'static>>,
    is_stream_continuation: bool,
) -> ProposedPlanStreamCell {
    ProposedPlanStreamCell {
        lines,
        is_stream_continuation,
    }
}

#[derive(Debug)]
pub(crate) struct ProposedPlanStreamCell {
    lines: Vec<Line<'static>>,
    is_stream_continuation: bool,
}

impl HistoryCell for ProposedPlanStreamCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.lines.clone()
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        plain_lines(self.lines.clone())
    }

    fn is_stream_continuation(&self) -> bool {
        self.is_stream_continuation
    }
}

#[derive(Debug)]
pub(crate) struct PlanUpdateCell {
    activity_id: String,
    explanation: Option<String>,
    plan: Vec<AgentTaskPlanStep>,
}

impl PlanUpdateCell {
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
}

impl HistoryCell for PlanUpdateCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut body = Vec::new();
        if let Some(note) = self.note() {
            let wrap_width = width.saturating_sub(4).max(1) as usize;
            body.extend(
                textwrap::wrap(note, wrap_width)
                    .into_iter()
                    .map(|s| s.to_string().dim().italic().into()),
            );
        }
        for AgentTaskPlanStep { step, status } in &self.plan {
            body.extend(step_lines(status, step, width));
        }
        self.with_body(body)
    }

    /// Without the note; past [`PLAN_PREVIEW_STEPS`], Codex's preview: the
    /// steps in progress, then those pending, then those completed.
    fn compact_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut steps: Vec<&AgentTaskPlanStep> = self.plan.iter().collect();
        if steps.len() > PLAN_PREVIEW_STEPS {
            steps.sort_by_key(|step| match step.status {
                AgentTaskPlanStepStatus::InProgress => 0,
                AgentTaskPlanStepStatus::Pending => 1,
                AgentTaskPlanStepStatus::Completed => 2,
            });
        }
        let body = steps
            .into_iter()
            .take(PLAN_PREVIEW_STEPS)
            .flat_map(|AgentTaskPlanStep { step, status }| step_lines(status, step, width))
            .collect();
        self.with_body(body)
    }

    fn activity_ids(&self) -> Vec<String> {
        vec![self.activity_id.clone()]
    }

    /// The note, and the steps past the preview.
    fn activity_disclosure(&self, _width: u16) -> Option<ActivityDisclosure> {
        (self.note().is_some() || self.plan.len() > PLAN_PREVIEW_STEPS)
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
