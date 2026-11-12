//! The model's thinking, one entry per reasoning item: while it streams, a
//! live entry that counts up (`• Thinking 3.4s`); once complete,
//! `• Thought · 20.7s`. Collapsed it is that line; its control shows the
//! text below it as dim Markdown, independently of every other entry. The
//! detailed transcript always shows the text.

use std::time::{Duration, Instant};

use hanafubuki_agent::AgentReasoningId;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};

use super::{plain_lines, raw_lines_from_source, ActivityDisclosure, HistoryCell};
use crate::render::line_utils::prefix_lines;
use crate::shimmer::shimmer_spans;
use crate::status_indicator_widget::fmt_elapsed_tenths;

/// What a thinking's activity id starts with, so Ctrl-O finds them all.
pub(crate) const THINKING_ACTIVITY: &str = "thinking:";

/// The live header's clock ticks in tenths of a second.
const THINKING_TICK_MILLIS: u128 = 100;

#[derive(Debug)]
pub(crate) struct ThinkingCell {
    /// `thinking:{reasoning id}` while the run lasts, `thinking:h{history
    /// item id}` replayed: disclosure is per run and never persisted.
    activity_id: String,
    /// The Markdown streamed so far, or the reasoning item's text.
    text: String,
    state: ThinkingState,
}

#[derive(Debug)]
enum ThinkingState {
    Live {
        started: Instant,
    },
    Completed {
        duration: Duration,
        /// The item's text was cut at the Agent's bound.
        truncated: bool,
    },
}

impl ThinkingCell {
    pub(crate) fn live(reasoning_id: AgentReasoningId) -> Self {
        Self {
            activity_id: format!("{THINKING_ACTIVITY}{reasoning_id}"),
            text: String::new(),
            state: ThinkingState::Live {
                started: Instant::now(),
            },
        }
    }

    /// A reasoning item of the session's history, `item_id` its history
    /// item id.
    pub(crate) fn replayed(
        item_id: u64,
        text: String,
        duration_millis: u64,
        truncated: bool,
    ) -> Self {
        Self {
            activity_id: format!("{THINKING_ACTIVITY}h{item_id}"),
            text,
            state: ThinkingState::Completed {
                duration: Duration::from_millis(duration_millis),
                truncated,
            },
        }
    }

    pub(crate) fn push(&mut self, delta: &str) {
        self.text.push_str(delta);
    }

    /// Ends the live cell after `duration_millis`, or, when the turn ended
    /// before the item did, after the time so far.
    pub(crate) fn complete(&mut self, duration_millis: Option<u64>) {
        let ThinkingState::Live { started } = self.state else {
            return;
        };
        let duration = duration_millis.map_or_else(|| started.elapsed(), Duration::from_millis);
        self.state = ThinkingState::Completed {
            duration,
            truncated: false,
        };
    }

    fn has_text(&self) -> bool {
        !self.text.trim().is_empty()
    }

    fn header(&self) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec!["• ".dim()];
        match self.state {
            ThinkingState::Live { started } => {
                spans.extend(shimmer_spans("Thinking"));
                spans.push(format!(" {}", fmt_elapsed_tenths(started.elapsed())).dim());
            }
            ThinkingState::Completed {
                duration,
                truncated,
            } => {
                spans.push(format!("Thought · {}", fmt_elapsed_tenths(duration)).dim());
                if truncated {
                    spans.push(" (truncated)".dim().italic());
                }
            }
        }
        Line::from(spans)
    }

    /// The text as dimmed Markdown under the header, wrapped to `width`.
    fn body(&self, width: u16) -> Vec<Line<'static>> {
        if !self.has_text() {
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
    /// Collapsed: the header alone.
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        vec![self.header()]
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

    fn copy_source(&self) -> Option<String> {
        Some(self.text.clone()).filter(|text| !text.trim().is_empty())
    }

    /// None before any text arrives, so a thinking without text has no
    /// control and Ctrl-O leaves it alone.
    fn activity_ids(&self) -> Vec<String> {
        if self.has_text() {
            vec![self.activity_id.clone()]
        } else {
            Vec::new()
        }
    }

    fn activity_disclosure(&self, _width: u16) -> Option<ActivityDisclosure> {
        self.has_text().then_some(ActivityDisclosure::Generic)
    }

    /// The clock moves every tenth of a second while the cell is live.
    fn transcript_animation_tick(&self) -> Option<u64> {
        match self.state {
            ThinkingState::Live { started } => Some(
                u64::try_from(started.elapsed().as_millis() / THINKING_TICK_MILLIS)
                    .unwrap_or(u64::MAX),
            ),
            ThinkingState::Completed { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests;
