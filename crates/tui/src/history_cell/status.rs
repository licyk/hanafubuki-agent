// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The `/status` card (Codex's `status` module, trimmed): the session's
//! model and Provider, the workspace, the permission mode and the session
//! mode, its id, the tokens it used and a compaction running in the
//! background. Codex's account, rate limits,
//! sandbox and AGENTS.md lines have no counterpart here.

use std::path::PathBuf;

use hanafubuki_agent::{AgentBackgroundCompactionState, AgentPermissionMode, AgentSessionMode};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{plain_lines, with_border, HistoryCell};
use crate::agent_bridge::StatusReport;
use crate::text_formatting::center_truncate_path;

const LABELS: [&str; 9] = [
    "Model",
    "Provider",
    "Directory",
    "Permissions",
    "Mode",
    "Session",
    "Token usage",
    "Context window",
    "Compaction",
];

#[derive(Debug)]
pub(crate) struct StatusHistoryCell {
    model: String,
    reasoning_effort: Option<String>,
    provider: String,
    roots: Vec<PathBuf>,
    permission_mode: AgentPermissionMode,
    mode: AgentSessionMode,
    session: String,
    input_tokens: u64,
    output_tokens: u64,
    /// The tokens in the context and the window's size, when both are known.
    context: Option<(u64, u64)>,
    /// Read from the snapshot as the command ran: it changes without a
    /// revision.
    background_compaction: Option<AgentBackgroundCompactionState>,
}

pub(crate) fn new_status_output(report: &StatusReport) -> StatusHistoryCell {
    let session = &report.session;
    let usage = session.usage.as_ref();
    StatusHistoryCell {
        model: session.model.clone(),
        reasoning_effort: session
            .reasoning_effort
            .as_ref()
            .map(|effort| effort.as_str().to_string()),
        provider: report.provider.clone(),
        roots: report.roots.clone(),
        permission_mode: report.permission_mode,
        mode: session.mode,
        session: session.id.to_string(),
        input_tokens: usage.map_or(0, |usage| usage.cumulative_input_tokens),
        output_tokens: usage.map_or(0, |usage| usage.cumulative_output_tokens),
        context: usage
            .and_then(|usage| Some((usage.context_consumed_tokens?, usage.context_window_tokens?))),
        background_compaction: session.background_compaction,
    }
}

impl StatusHistoryCell {
    fn token_usage_spans(&self) -> Vec<Span<'static>> {
        let total_fmt = format_tokens_compact(self.input_tokens + self.output_tokens);
        let input_fmt = format_tokens_compact(self.input_tokens);
        let output_fmt = format_tokens_compact(self.output_tokens);

        vec![
            Span::from(total_fmt),
            Span::from(" total "),
            Span::from(" (").dim(),
            Span::from(input_fmt).dim(),
            Span::from(" input").dim(),
            Span::from(" + ").dim(),
            Span::from(output_fmt).dim(),
            Span::from(" output").dim(),
            Span::from(")").dim(),
        ]
    }

    fn context_window_spans(&self) -> Option<Vec<Span<'static>>> {
        let (used, window) = self.context?;
        let percent = 100u64.saturating_sub(used.saturating_mul(100) / window.max(1));
        Some(vec![
            Span::from(format!("{percent}% left")),
            Span::from(" (").dim(),
            Span::from(format_tokens_compact(used)).dim(),
            Span::from(" used / ").dim(),
            Span::from(format_tokens_compact(window)).dim(),
            Span::from(")").dim(),
        ])
    }
}

impl HistoryCell for StatusHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let available_inner_width = usize::from(width.saturating_sub(4));
        if available_inner_width == 0 {
            return Vec::new();
        }
        let formatter = FieldFormatter::from_labels(LABELS);
        let value_width = formatter.value_width(available_inner_width);

        let mut lines: Vec<Line<'static>> = vec![
            Line::from(vec![
                Span::from(format!("{}>_ ", FieldFormatter::INDENT)).dim(),
                Span::from("hanafubuki-agent").bold(),
                Span::from(" ").dim(),
                Span::from(format!("(v{})", env!("CARGO_PKG_VERSION"))).dim(),
            ]),
            Line::default(),
        ];

        let mut model_spans = vec![Span::from(self.model.clone())];
        if let Some(effort) = &self.reasoning_effort {
            model_spans.push(Span::from(format!(" (reasoning {effort})")).dim());
        }
        lines.push(formatter.line("Model", model_spans));
        lines.push(formatter.line("Provider", vec![Span::from(self.provider.clone())]));
        let mut roots = self.roots.iter().map(|root| {
            let root = root.to_string_lossy();
            if root.width() > value_width {
                center_truncate_path(&root, value_width)
            } else {
                root.into_owned()
            }
        });
        if let Some(first) = roots.next() {
            lines.push(formatter.line("Directory", vec![Span::from(first)]));
        }
        for root in roots {
            lines.push(formatter.continuation(vec![Span::from(root)]));
        }
        let permissions = permission_mode_name(self.permission_mode);
        lines.push(formatter.line("Permissions", vec![Span::from(permissions)]));
        let mode = match self.mode {
            AgentSessionMode::Default => "default",
            AgentSessionMode::Plan => "plan",
        };
        lines.push(formatter.line("Mode", vec![Span::from(mode)]));
        lines.push(formatter.line("Session", vec![Span::from(self.session.clone())]));
        lines.push(Line::default());
        lines.push(formatter.line("Token usage", self.token_usage_spans()));
        if let Some(spans) = self.context_window_spans() {
            lines.push(formatter.line("Context window", spans));
        }
        if let Some(state) = self.background_compaction {
            let state = match state {
                AgentBackgroundCompactionState::Running => "compacting in the background",
                AgentBackgroundCompactionState::Ready => {
                    "compacted in the background, installed before the next request"
                }
            };
            lines.push(formatter.line("Compaction", vec![Span::from(state)]));
        }

        let content_width = lines.iter().map(Line::width).max().unwrap_or(0);
        let inner_width = content_width.min(available_inner_width);
        let lines = lines
            .into_iter()
            .map(|line| truncate_line_to_width(line, inner_width))
            .collect();
        let mut out: Vec<Line<'static>> = vec!["/status".magenta().into(), Line::default()];
        out.extend(with_border(lines));
        out
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        plain_lines(self.display_lines(u16::MAX))
    }
}

/// Lays out `label:   value` rows with the values aligned.
#[derive(Debug, Clone)]
struct FieldFormatter {
    indent: &'static str,
    label_width: usize,
    value_offset: usize,
    value_indent: String,
}

impl FieldFormatter {
    const INDENT: &'static str = " ";

    fn from_labels<S>(labels: impl IntoIterator<Item = S>) -> Self
    where
        S: AsRef<str>,
    {
        let label_width = labels
            .into_iter()
            .map(|label| UnicodeWidthStr::width(label.as_ref()))
            .max()
            .unwrap_or(0);
        let indent_width = UnicodeWidthStr::width(Self::INDENT);
        let value_offset = indent_width + label_width + 1 + 3;

        Self {
            indent: Self::INDENT,
            label_width,
            value_offset,
            value_indent: " ".repeat(value_offset),
        }
    }

    fn line(&self, label: &'static str, mut value_spans: Vec<Span<'static>>) -> Line<'static> {
        let mut spans = Vec::with_capacity(value_spans.len() + 1);
        spans.push(self.label_span(label));
        spans.append(&mut value_spans);
        Line::from(spans)
    }

    fn continuation(&self, mut spans: Vec<Span<'static>>) -> Line<'static> {
        let mut all_spans = Vec::with_capacity(spans.len() + 1);
        all_spans.push(Span::from(self.value_indent.clone()).dim());
        all_spans.append(&mut spans);
        Line::from(all_spans)
    }

    fn value_width(&self, available_inner_width: usize) -> usize {
        available_inner_width.saturating_sub(self.value_offset)
    }

    fn label_span(&self, label: &str) -> Span<'static> {
        let mut buf = String::with_capacity(self.value_offset);
        buf.push_str(self.indent);

        buf.push_str(label);
        buf.push(':');

        let label_width = UnicodeWidthStr::width(label);
        let padding = 3 + self.label_width.saturating_sub(label_width);
        for _ in 0..padding {
            buf.push(' ');
        }

        Span::from(buf).dim()
    }
}

fn truncate_line_to_width(line: Line<'static>, max_width: usize) -> Line<'static> {
    if max_width == 0 {
        return Line::from(Vec::<Span<'static>>::new());
    }

    let mut used = 0usize;
    let mut spans_out: Vec<Span<'static>> = Vec::new();

    for span in line.spans {
        let text = span.content.into_owned();
        let style = span.style;
        let span_width = UnicodeWidthStr::width(text.as_str());

        if span_width == 0 {
            spans_out.push(Span::styled(text, style));
            continue;
        }

        if used >= max_width {
            break;
        }

        if used + span_width <= max_width {
            used += span_width;
            spans_out.push(Span::styled(text, style));
            continue;
        }

        let mut truncated = String::new();
        for ch in text.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + ch_width > max_width {
                break;
            }
            truncated.push(ch);
            used += ch_width;
        }

        if !truncated.is_empty() {
            spans_out.push(Span::styled(truncated, style));
        }

        break;
    }

    Line::from(spans_out)
}

/// The permission mode as the configuration names it.
pub(crate) fn permission_mode_name(mode: AgentPermissionMode) -> &'static str {
    match mode {
        AgentPermissionMode::Manual => "manual",
        AgentPermissionMode::AutoReview => "auto-review",
        AgentPermissionMode::FullAccess => "full-access",
    }
}

pub(crate) fn format_tokens_compact(value: u64) -> String {
    if value < 1_000 {
        return value.to_string();
    }

    let value_f64 = value as f64;
    let (scaled, suffix) = if value >= 1_000_000_000_000 {
        (value_f64 / 1_000_000_000_000.0, "T")
    } else if value >= 1_000_000_000 {
        (value_f64 / 1_000_000_000.0, "B")
    } else if value >= 1_000_000 {
        (value_f64 / 1_000_000.0, "M")
    } else {
        (value_f64 / 1_000.0, "K")
    };

    let decimals = if scaled < 10.0 {
        2
    } else if scaled < 100.0 {
        1
    } else {
        0
    };

    let mut formatted = format!("{scaled:.decimals$}");
    if formatted.contains('.') {
        while formatted.ends_with('0') {
            formatted.pop();
        }
        if formatted.ends_with('.') {
            formatted.pop();
        }
    }

    format!("{formatted}{suffix}")
}
