// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! A tool call: the active cell while it runs (with a spinner), then its
//! outcome and an excerpt of its output, or the diff of the patch it
//! applied. Codex's exec and MCP cells, for every tool the Agent has.

use std::time::Instant;

use hanafubuki_agent::PatchPreview;
use ratatui::style::{Modifier, Stylize};
use ratatui::text::{Line, Span};

use super::HistoryCell;
use crate::diff_render::create_diff_summary;
use crate::shimmer::spinner;
use crate::text_formatting::strip_terminal_controls;
use crate::wrapping::{word_wrap_line, RtOptions};

/// The lines of output shown from each end of a long output.
const TOOL_CALL_MAX_LINES: usize = 5;

/// What a finished call left to show.
#[derive(Debug, Default)]
pub(crate) struct ToolResultView {
    /// The text it returned, for a command its output.
    pub(crate) text: Option<String>,
    pub(crate) exit_code: Option<i32>,
    /// The patch an `apply_patch` call applied (or tried to).
    pub(crate) patch: Option<PatchPreview>,
}

#[derive(Debug)]
pub(crate) struct ToolCallCell {
    call_id: String,
    name: String,
    subject: Option<String>,
    start_time: Instant,
    outcome: Option<Outcome>,
}

#[derive(Debug)]
struct Outcome {
    is_error: bool,
    result: ToolResultView,
}

pub(crate) fn new_tool_call(
    call_id: String,
    name: String,
    subject: Option<String>,
) -> ToolCallCell {
    ToolCallCell {
        call_id,
        name,
        subject,
        start_time: Instant::now(),
        outcome: None,
    }
}

impl ToolCallCell {
    pub(crate) fn call_id(&self) -> &str {
        &self.call_id
    }

    pub(crate) fn complete(&mut self, is_error: bool, result: ToolResultView) {
        self.outcome = Some(Outcome { is_error, result });
    }

    fn header(&self) -> Line<'static> {
        let (bullet, verb) = match &self.outcome {
            None => (spinner(Some(self.start_time), true), "Running".bold()),
            Some(outcome) if outcome.is_error => ("•".red().bold(), "Failed".bold()),
            Some(_) => ("•".green().bold(), "Called".bold()),
        };
        let mut spans = vec![
            bullet,
            " ".into(),
            verb,
            " ".into(),
            self.name.clone().cyan(),
        ];
        if let Some(subject) = &self.subject {
            spans.push(": ".dim());
            spans.push(Span::from(strip_terminal_controls(subject)));
        }
        Line::from(spans)
    }
}

impl HistoryCell for ToolCallCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let header = self.header();
        let mut lines: Vec<Line<'static>> = word_wrap_line(
            &header,
            RtOptions::new(width.max(1) as usize).subsequent_indent("    ".into()),
        )
        .iter()
        .map(crate::render::line_utils::line_to_static)
        .collect();
        let Some(Outcome { result, .. }) = &self.outcome else {
            return lines;
        };
        if let Some(patch) = &result.patch {
            lines.extend(create_diff_summary(patch, width.saturating_sub(4) as usize));
            return lines;
        }
        let mut body = result.text.as_deref().map(output_lines).unwrap_or_default();
        if let Some(code) = result.exit_code.filter(|code| *code != 0) {
            body.push(Line::from(format!("exit code {code}")).red());
        }
        for (index, line) in body.into_iter().enumerate() {
            let prefix = if index == 0 { "  └ " } else { "    " };
            let mut spans = vec![Span::from(prefix).dim()];
            spans.extend(line.spans);
            lines.push(Line::from(spans).style(line.style));
        }
        lines
    }
}

/// The first and last lines of `text`, dimmed, with how many lines between
/// them are left out.
fn output_lines(text: &str) -> Vec<Line<'static>> {
    let text = strip_terminal_controls(text);
    let raw: Vec<&str> = text.trim_end().lines().collect();
    let dim = |line: &str| Line::from(line.to_string()).add_modifier(Modifier::DIM);
    if raw.len() <= 2 * TOOL_CALL_MAX_LINES {
        return raw.into_iter().map(dim).collect();
    }
    let omitted = raw.len() - 2 * TOOL_CALL_MAX_LINES;
    let mut out: Vec<Line<'static>> = raw[..TOOL_CALL_MAX_LINES].iter().map(|l| dim(l)).collect();
    out.push(format!("… +{omitted} lines").into());
    out.extend(
        raw[raw.len() - TOOL_CALL_MAX_LINES..]
            .iter()
            .map(|l| dim(l)),
    );
    out
}

#[cfg(test)]
mod tests;
