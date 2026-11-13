// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The streams of an answer and of a proposed plan: rows committed as their
//! lines complete, one per commit tick, at the width the stream started at;
//! at the end, the stream's cells are consolidated into one cell of its
//! Markdown ([`Consolidation`]), which the app puts in their place.

use crate::history_cell::{self, AgentMarkdownCell, HistoryCell, ProposedPlanCell, StreamId};
use crate::render::line_utils::prefix_lines;
use crate::style::proposed_plan_style;
use ratatui::prelude::Stylize;
use ratatui::text::Line;

use super::StreamState;

/// A finished stream's cell, to replace the cells its rows were committed
/// in.
#[derive(Debug)]
pub(crate) struct Consolidation {
    pub(crate) stream: StreamId,
    pub(crate) cell: Box<dyn HistoryCell>,
}

/// What a stream's end gives: the last of its rows, and its consolidated
/// cell when it committed any.
pub(crate) struct Finalized {
    pub(crate) cell: Option<Box<dyn HistoryCell>>,
    pub(crate) consolidation: Option<Consolidation>,
}

/// Controller that manages newline-gated streaming, header emission, and
/// commit animation across streams.
pub(crate) struct StreamController {
    state: StreamState,
    header_emitted: bool,
    stream: StreamId,
    cwd: String,
}

impl StreamController {
    /// An answer's stream, wrapped to `width`, local file links relative to
    /// `cwd`.
    pub(crate) fn new(width: Option<usize>, cwd: String) -> Self {
        Self {
            state: StreamState::new(
                width,
                Some(cwd.clone()),
                crate::markdown::append_markdown_agent,
            ),
            header_emitted: false,
            stream: StreamId::next(),
            cwd,
        }
    }

    /// Push a delta; if it contains a newline, commit completed lines and start animation.
    pub(crate) fn push(&mut self, delta: &str) -> bool {
        self.state.push(delta)
    }

    /// Finalize the active stream. Drain and emit now.
    pub(crate) fn finalize(&mut self) -> Finalized {
        let source = self.state.collector.source().to_string();
        let lines = self.state.finalize();
        let cell = self.emit(lines);
        let consolidation = self.header_emitted.then(|| Consolidation {
            stream: self.stream,
            cell: Box::new(AgentMarkdownCell::new(source, self.cwd.clone())),
        });
        Finalized {
            cell,
            consolidation,
        }
    }

    /// Step animation: commit at most one queued line and handle end-of-drain cleanup.
    pub(crate) fn on_commit_tick(&mut self) -> (Option<Box<dyn HistoryCell>>, bool) {
        let step = self.state.step();
        (self.emit(step), self.state.is_idle())
    }

    fn emit(&mut self, lines: Vec<Line<'static>>) -> Option<Box<dyn HistoryCell>> {
        if lines.is_empty() {
            return None;
        }
        let is_first_line = !self.header_emitted;
        self.header_emitted = true;
        Some(Box::new(history_cell::AgentMessageCell::new(
            lines,
            is_first_line,
            self.stream,
        )))
    }
}

/// Controller that streams proposed plan markdown into a styled plan block.
pub(crate) struct PlanStreamController {
    state: StreamState,
    header_emitted: bool,
    top_padding_emitted: bool,
    stream: StreamId,
    cwd: String,
}

impl PlanStreamController {
    pub(crate) fn new(width: Option<usize>, cwd: String) -> Self {
        Self {
            state: StreamState::new(width, Some(cwd.clone()), crate::markdown::append_markdown),
            header_emitted: false,
            top_padding_emitted: false,
            stream: StreamId::next(),
            cwd,
        }
    }

    /// Push a delta; if it contains a newline, commit completed lines and start animation.
    pub(crate) fn push(&mut self, delta: &str) -> bool {
        self.state.push(delta)
    }

    /// Finalize the active stream. Drain and emit now.
    pub(crate) fn finalize(&mut self) -> Finalized {
        let source = self.state.collector.source().to_string();
        let lines = self.state.finalize();
        let cell = self.emit(lines, true);
        Finalized {
            cell,
            consolidation: Some(Consolidation {
                stream: self.stream,
                cell: Box::new(ProposedPlanCell::new(source, self.cwd.clone())),
            }),
        }
    }

    /// Step animation: commit at most one queued line and handle end-of-drain cleanup.
    pub(crate) fn on_commit_tick(&mut self) -> (Option<Box<dyn HistoryCell>>, bool) {
        let step = self.state.step();
        (self.emit(step, false), self.state.is_idle())
    }

    fn emit(
        &mut self,
        lines: Vec<Line<'static>>,
        include_bottom_padding: bool,
    ) -> Option<Box<dyn HistoryCell>> {
        if lines.is_empty() && !include_bottom_padding {
            return None;
        }

        let mut out_lines: Vec<Line<'static>> = Vec::new();
        let is_stream_continuation = self.header_emitted;
        if !self.header_emitted {
            out_lines.push(vec!["• ".dim(), "Proposed Plan".bold()].into());
            out_lines.push(Line::from(" "));
            self.header_emitted = true;
        }

        let mut plan_lines: Vec<Line<'static>> = Vec::new();
        if !self.top_padding_emitted {
            plan_lines.push(Line::from(" "));
            self.top_padding_emitted = true;
        }
        plan_lines.extend(lines);
        if include_bottom_padding {
            plan_lines.push(Line::from(" "));
        }

        let plan_style = proposed_plan_style();
        let plan_lines = prefix_lines(plan_lines, "  ".into(), "  ".into())
            .into_iter()
            .map(|line| line.style(plan_style))
            .collect::<Vec<_>>();
        out_lines.extend(plan_lines);

        Some(Box::new(history_cell::new_proposed_plan_stream(
            out_lines,
            is_stream_continuation,
            self.stream,
        )))
    }
}

#[cfg(test)]
mod tests;
