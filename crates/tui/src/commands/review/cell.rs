// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What a review shows in the transcript: Codex's banners around it, and
//! the review itself: the verdict, the explanation, and each finding with
//! its priority, title, location and body.

use hanafubuki_agent::{AgentReviewFinding, AgentReviewOutput};
use ratatui::style::Stylize;
use ratatui::text::Line;

use crate::history_cell::{HistoryCell, PlainHistoryCell};
use crate::wrapping::{word_wrap_lines, RtOptions};

const FINISHED: &str = "<< Code review finished >>";

/// A review's start or end, as Codex marks it.
pub(super) fn review_banner(text: String) -> PlainHistoryCell {
    PlainHistoryCell::new(vec![Line::from(text.cyan())])
}

#[derive(Debug)]
pub(super) struct ReviewCell(AgentReviewOutput);

impl ReviewCell {
    pub(super) fn new(output: AgentReviewOutput) -> Self {
        Self(output)
    }
}

impl HistoryCell for ReviewCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let width = usize::from(width.max(1));
        let output = &self.0;
        let mut lines = vec![Line::from(FINISHED.cyan())];
        let verdict = output.overall_correctness.trim();
        if !verdict.is_empty() {
            let mut spans = vec!["Verdict: ".dim(), verdict.to_string().bold()];
            if output.overall_confidence_score > 0.0 {
                spans.push(format!(" (confidence {:.2})", output.overall_confidence_score).dim());
            }
            lines.extend(wrap(Line::from(spans), width, "  ", "  "));
        }
        let explanation = output.overall_explanation.trim();
        for paragraph in explanation.lines() {
            lines.extend(wrap(Line::from(paragraph.to_string()), width, "  ", "  "));
        }
        if !output.findings.is_empty() {
            lines.push(Line::default());
            let heading = if output.findings.len() > 1 {
                "Full review comments:"
            } else {
                "Review comment:"
            };
            lines.push(Line::from(vec!["  ".into(), heading.bold()]));
            for finding in &output.findings {
                lines.push(Line::default());
                lines.extend(finding_lines(finding, width));
            }
        }
        if explanation.is_empty() && output.findings.is_empty() {
            lines.push(Line::from(format!("  {}", output.render_text()).dim()));
        }
        lines
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        std::iter::once(FINISHED.to_string())
            .chain(self.0.render_text().lines().map(str::to_string))
            .map(Line::from)
            .collect()
    }
}

/// A finding: its title (its priority first, unless the title has it), its
/// location, then its body, under a bullet.
fn finding_lines(finding: &AgentReviewFinding, width: usize) -> Vec<Line<'static>> {
    let title = match finding.priority {
        Some(priority) if !finding.title.starts_with("[P") => {
            format!("[P{priority}] {}", finding.title)
        }
        _ => finding.title.clone(),
    };
    let mut lines = wrap(Line::from(title.bold()), width, "  • ", "    ");
    if let Some(location) = &finding.code_location {
        let location = format!(
            "{}:{}-{}",
            location.absolute_file_path.display(),
            location.line_range.start,
            location.line_range.end
        );
        lines.extend(wrap(Line::from(location.cyan()), width, "    ", "    "));
    }
    for paragraph in finding.body.lines() {
        lines.extend(wrap(
            Line::from(paragraph.to_string()),
            width,
            "    ",
            "    ",
        ));
    }
    lines
}

fn wrap(
    line: Line<'static>,
    width: usize,
    first: &'static str,
    rest: &'static str,
) -> Vec<Line<'static>> {
    let options = RtOptions::new(width)
        .initial_indent(Line::from(first))
        .subsequent_indent(Line::from(rest));
    word_wrap_lines([line], options)
}
