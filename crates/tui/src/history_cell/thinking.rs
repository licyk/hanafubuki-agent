//! The model's thinking: while it streams, a cell in the viewport that
//! counts up (`• Thinking 5s`) and shows the streamed Markdown when
//! expanded; once complete, `• Thought for 12s` with the text below it when
//! expanded. The scrollback keeps the cell as it was shown when it
//! completed, and the transcript pager always shows its text.

use std::time::Instant;

use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};

use super::{plain_lines, raw_lines_from_source, HistoryCell};
use crate::render::line_utils::prefix_lines;
use crate::shimmer::shimmer_spans;
use crate::status_indicator_widget::fmt_elapsed_compact;

#[derive(Debug)]
pub(crate) struct ThinkingCell {
    /// The Markdown streamed so far, or the reasoning item's text.
    text: String,
    expanded: bool,
    state: ThinkingState,
}

#[derive(Debug)]
enum ThinkingState {
    Live {
        started: Instant,
    },
    Completed {
        duration_millis: u64,
        /// The item's text was cut at the Agent's bound.
        truncated: bool,
    },
}

impl ThinkingCell {
    pub(crate) fn live(expanded: bool) -> Self {
        Self {
            text: String::new(),
            expanded,
            state: ThinkingState::Live {
                started: Instant::now(),
            },
        }
    }

    pub(crate) fn completed(
        text: String,
        duration_millis: u64,
        truncated: bool,
        expanded: bool,
    ) -> Self {
        Self {
            text,
            expanded,
            state: ThinkingState::Completed {
                duration_millis,
                truncated,
            },
        }
    }

    pub(crate) fn push(&mut self, delta: &str) {
        self.text.push_str(delta);
    }

    pub(crate) fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
    }

    /// Ends the live cell after `duration_millis`, or, when the turn ended
    /// before the item did, after the time so far.
    pub(crate) fn complete(&mut self, duration_millis: Option<u64>) {
        let ThinkingState::Live { started } = self.state else {
            return;
        };
        let duration_millis = duration_millis
            .unwrap_or_else(|| u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
        self.state = ThinkingState::Completed {
            duration_millis,
            truncated: false,
        };
    }

    fn header(&self) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec!["• ".dim()];
        match self.state {
            ThinkingState::Live { started } => {
                spans.extend(shimmer_spans("Thinking"));
                spans.push(format!(" {}", fmt_elapsed_compact(started.elapsed().as_secs())).dim());
            }
            ThinkingState::Completed {
                duration_millis,
                truncated,
            } => {
                let elapsed = fmt_elapsed_compact(duration_millis / 1000);
                spans.push(format!("Thought for {elapsed}").dim());
                if truncated {
                    spans.push(" (truncated)".dim().italic());
                }
            }
        }
        Line::from(spans)
    }

    /// The text as dimmed Markdown under the header, wrapped to `width`.
    fn body(&self, width: u16) -> Vec<Line<'static>> {
        if self.text.trim().is_empty() {
            return Vec::new();
        }
        let mut lines = Vec::new();
        crate::markdown::append_markdown(
            &self.text,
            Some(usize::from(width.saturating_sub(2)).max(1)),
            &mut lines,
        );
        let dim = Style::default().dim();
        let lines = lines
            .into_iter()
            .map(|line| {
                let spans = line.spans.into_iter().map(|span| span.patch_style(dim));
                Line::from(spans.collect::<Vec<_>>()).style(line.style.patch(dim))
            })
            .collect();
        prefix_lines(lines, "  ".into(), "  ".into())
    }
}

impl HistoryCell for ThinkingCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = vec![self.header()];
        if self.expanded {
            lines.extend(self.body(width));
        }
        lines
    }

    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = vec![self.header()];
        lines.extend(self.body(width));
        lines
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        let mut lines = plain_lines([self.header()]);
        lines.extend(raw_lines_from_source(self.text.trim_end()));
        lines
    }

    fn copy_source(&self) -> Option<&str> {
        Some(self.text.as_str()).filter(|text| !text.trim().is_empty())
    }

    /// The shimmer moves every frame while the cell is live.
    fn transcript_animation_tick(&self) -> Option<u64> {
        match self.state {
            ThinkingState::Live { started } => {
                Some(u64::try_from(started.elapsed().as_millis() / 32).unwrap_or(u64::MAX))
            }
            ThinkingState::Completed { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests;
