// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The streams of an answer and of a proposed plan (Codex's two-region
//! controllers). The rendered rows split into the stable region, queued
//! and committed by the commit ticks into cells that do not change, and
//! the mutable tail, shown live after them in the stream's tail entry
//! ([`StreamTailCell`]) as it would be committed now. Each change reaches
//! the app as one [`StreamUpdate`]: the rows it committed and what the
//! tail shows then, together, so no frame shows rows twice or not at all.
//!
//! At the end, the stream's cells are consolidated into one cell of its
//! Markdown ([`Consolidation`]), which the app puts in their place.

mod commit;
mod preview;
mod state;

use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::prelude::Stylize;
use ratatui::text::Line;

use crate::history_cell::{
    self, AgentMarkdownCell, HistoryCell, LiveCell, ProposedPlanCell, StreamId, StreamTailCell,
};
use crate::markdown_render::ListSpacing;
use crate::render::line_utils::prefix_lines;
use crate::style::proposed_plan_style;
use state::StreamCore;

/// A finished stream's cell, to replace the cells its rows were committed
/// in; `repair` when a committed row differs from it at the stream's width,
/// so the scrollback is written again.
#[derive(Debug)]
pub(crate) struct Consolidation {
    pub(crate) stream: StreamId,
    pub(crate) cell: Box<dyn HistoryCell>,
    pub(crate) repair: bool,
}

/// What a stream changed, applied by the app at once: `committed` goes
/// before the tail entry, which shows `shown` from then on.
#[derive(Debug)]
pub(crate) struct StreamUpdate {
    pub(crate) tail: Arc<LiveCell<StreamTailCell>>,
    pub(crate) committed: Option<Box<dyn HistoryCell>>,
    /// What the tail shows now, when it changed: the cell its rows would be
    /// committed in, or nothing.
    pub(crate) shown: Option<TailRows>,
    /// The stream ended: its tail is empty, and complete.
    pub(crate) ended: bool,
}

#[derive(Debug)]
pub(crate) struct TailRows(pub(crate) Option<Box<dyn HistoryCell>>);

/// What a stream's end gives: the last update, and the consolidated cell
/// when the stream had any source.
pub(crate) struct Finalized {
    pub(crate) update: Option<StreamUpdate>,
    pub(crate) consolidation: Option<Consolidation>,
}

/// The stream's tail entry and what it was last told to show.
struct Tail {
    cell: Arc<LiveCell<StreamTailCell>>,
    shown: (Vec<Line<'static>>, bool),
}

impl Tail {
    fn new(stream: StreamId) -> Self {
        Self {
            cell: LiveCell::new(StreamTailCell::new(stream)),
            shown: (Vec::new(), false),
        }
    }

    /// The update for `committed` and the tail's `lines` (`first`: they
    /// start the stream); `None` when neither changed anything.
    fn update(
        &mut self,
        committed: Option<Box<dyn HistoryCell>>,
        lines: Vec<Line<'static>>,
        first: bool,
        shown: impl FnOnce(Vec<Line<'static>>) -> Box<dyn HistoryCell>,
    ) -> Option<StreamUpdate> {
        let changed = self.shown.0 != lines || (!lines.is_empty() && self.shown.1 != first);
        if committed.is_none() && !changed {
            return None;
        }
        let rows = changed.then(|| {
            self.shown = (lines.clone(), first);
            TailRows((!lines.is_empty()).then(|| shown(lines)))
        });
        Some(StreamUpdate {
            tail: Arc::clone(&self.cell),
            committed,
            shown: rows,
            ended: false,
        })
    }

    /// The stream's last update: `committed`, and the tail emptied.
    fn end(&self, committed: Option<Box<dyn HistoryCell>>) -> StreamUpdate {
        StreamUpdate {
            tail: Arc::clone(&self.cell),
            committed,
            shown: Some(TailRows(None)),
            ended: true,
        }
    }
}

/// An answer's stream.
pub(crate) struct StreamController {
    core: StreamCore,
    header_emitted: bool,
    stream: StreamId,
    tail: Tail,
}

impl StreamController {
    /// An answer's stream, wrapped to `width`, local file links relative to
    /// `cwd`, its lists spaced by `list_spacing` while it streams.
    pub(crate) fn new(width: Option<usize>, cwd: String, list_spacing: ListSpacing) -> Self {
        let stream = StreamId::next();
        Self {
            core: StreamCore::new(
                width,
                cwd,
                crate::markdown::render_streaming_markdown_agent,
                list_spacing,
            ),
            header_emitted: false,
            stream,
            tail: Tail::new(stream),
        }
    }

    /// Pushes a delta: `true` when rows were queued or the tail changed.
    pub(crate) fn push(&mut self, delta: &str) -> bool {
        self.push_at(delta, Instant::now())
    }

    pub(crate) fn push_at(&mut self, delta: &str, now: Instant) -> bool {
        self.core.push_delta(delta, now)
    }

    /// Commits the first queued row; `true` when the queue is empty then.
    pub(crate) fn on_commit_tick(&mut self) -> (Option<Box<dyn HistoryCell>>, bool) {
        let step = self.core.tick();
        (self.emit(step), self.core.is_idle())
    }

    /// Commits up to `max_lines` queued rows.
    pub(crate) fn on_commit_tick_batch(
        &mut self,
        max_lines: usize,
    ) -> (Option<Box<dyn HistoryCell>>, bool) {
        let step = self.core.tick_batch(max_lines);
        (self.emit(step), self.core.is_idle())
    }

    /// The app's update for `committed` and the tail now, if anything
    /// changed.
    pub(crate) fn update(
        &mut self,
        committed: Option<Box<dyn HistoryCell>>,
    ) -> Option<StreamUpdate> {
        let lines = self.core.current_tail_lines();
        let first = self.tail_starts_stream();
        let stream = self.stream;
        self.tail.update(committed, lines, first, |lines| {
            Box::new(history_cell::AgentMessageCell::new(lines, first, stream))
        })
    }

    /// The stream's end: its last rows committed, its tail emptied, and its
    /// cell of Markdown.
    pub(crate) fn finalize(&mut self) -> Finalized {
        let remaining = self.core.finalize_remaining();
        let committed = self.emit(remaining.lines);
        let consolidation = (!remaining.source.is_empty()).then(|| Consolidation {
            stream: self.stream,
            cell: Box::new(AgentMarkdownCell::new(
                remaining.source,
                self.core.cwd.clone(),
            )),
            repair: remaining.diverged,
        });
        Finalized {
            update: Some(self.tail.end(committed)),
            consolidation,
        }
    }

    pub(crate) fn queued_lines(&self) -> usize {
        self.core.queued_lines()
    }

    pub(crate) fn oldest_queued_age(&self, now: Instant) -> Option<Duration> {
        self.core.oldest_queued_age(now)
    }

    /// The width changed: what is queued and the tail are laid out again.
    pub(crate) fn set_width(&mut self, width: Option<usize>) {
        self.core.set_width(width, Instant::now());
    }

    pub(crate) fn width(&self) -> Option<usize> {
        self.core.width
    }

    /// The tail's rows start the stream: nothing is committed or queued
    /// before them.
    fn tail_starts_stream(&self) -> bool {
        !self.header_emitted && self.core.enqueued_stable_len == 0
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

    #[cfg(test)]
    pub(crate) fn tail_lines(&self) -> Vec<Line<'static>> {
        self.core.current_tail_lines()
    }

    #[cfg(test)]
    pub(crate) fn has_live_tail(&self) -> bool {
        self.core.has_tail()
    }

    #[cfg(test)]
    pub(crate) fn committed_source(&self) -> &str {
        self.core.state.collector.committed_source()
    }
}

/// A proposed plan's stream, in its styled block.
pub(crate) struct PlanStreamController {
    core: StreamCore,
    header_emitted: bool,
    top_padding_emitted: bool,
    stream: StreamId,
    tail: Tail,
}

impl PlanStreamController {
    pub(crate) fn new(width: Option<usize>, cwd: String, list_spacing: ListSpacing) -> Self {
        let stream = StreamId::next();
        Self {
            core: StreamCore::new(
                width,
                cwd,
                crate::markdown_render::render_streaming_markdown,
                list_spacing,
            ),
            header_emitted: false,
            top_padding_emitted: false,
            stream,
            tail: Tail::new(stream),
        }
    }

    pub(crate) fn push(&mut self, delta: &str) -> bool {
        self.push_at(delta, Instant::now())
    }

    pub(crate) fn push_at(&mut self, delta: &str, now: Instant) -> bool {
        self.core.push_delta(delta, now)
    }

    pub(crate) fn on_commit_tick(&mut self) -> (Option<Box<dyn HistoryCell>>, bool) {
        let step = self.core.tick();
        (self.emit(step, false), self.core.is_idle())
    }

    pub(crate) fn on_commit_tick_batch(
        &mut self,
        max_lines: usize,
    ) -> (Option<Box<dyn HistoryCell>>, bool) {
        let step = self.core.tick_batch(max_lines);
        (self.emit(step, false), self.core.is_idle())
    }

    pub(crate) fn update(
        &mut self,
        committed: Option<Box<dyn HistoryCell>>,
    ) -> Option<StreamUpdate> {
        let lines = self.core.current_tail_lines();
        let first = !self.header_emitted;
        // The plan's block around the tail's rows, its header while the
        // tail starts the stream.
        let shown = if lines.is_empty() {
            Vec::new()
        } else {
            self.render_display_lines(lines, false)
        };
        let stream = self.stream;
        let continuation = self.header_emitted;
        self.tail.update(committed, shown, first, |shown| {
            Box::new(history_cell::new_proposed_plan_stream(
                shown,
                continuation,
                stream,
            ))
        })
    }

    pub(crate) fn finalize(&mut self) -> Finalized {
        let remaining = self.core.finalize_remaining();
        let committed = self.emit(remaining.lines, true);
        Finalized {
            update: Some(self.tail.end(committed)),
            consolidation: Some(Consolidation {
                stream: self.stream,
                cell: Box::new(ProposedPlanCell::new(
                    remaining.source,
                    self.core.cwd.clone(),
                )),
                repair: remaining.diverged,
            }),
        }
    }

    pub(crate) fn queued_lines(&self) -> usize {
        self.core.queued_lines()
    }

    pub(crate) fn oldest_queued_age(&self, now: Instant) -> Option<Duration> {
        self.core.oldest_queued_age(now)
    }

    pub(crate) fn set_width(&mut self, width: Option<usize>) {
        self.core.set_width(width, Instant::now());
    }

    pub(crate) fn width(&self) -> Option<usize> {
        self.core.width
    }

    fn emit(
        &mut self,
        lines: Vec<Line<'static>>,
        include_bottom_padding: bool,
    ) -> Option<Box<dyn HistoryCell>> {
        if lines.is_empty() && !include_bottom_padding {
            return None;
        }
        let is_stream_continuation = self.header_emitted;
        let out_lines = self.render_display_lines(lines, include_bottom_padding);
        self.header_emitted = true;
        self.top_padding_emitted = true;
        Some(Box::new(history_cell::new_proposed_plan_stream(
            out_lines,
            is_stream_continuation,
            self.stream,
        )))
    }

    /// `lines` in the plan's block: its header and top padding before the
    /// first, its bottom padding at the end.
    fn render_display_lines(
        &self,
        lines: Vec<Line<'static>>,
        include_bottom_padding: bool,
    ) -> Vec<Line<'static>> {
        let mut out_lines: Vec<Line<'static>> = Vec::new();
        if !self.header_emitted {
            out_lines.push(vec!["• ".dim(), "Proposed Plan".bold()].into());
            out_lines.push(Line::from(" "));
        }
        let mut plan_lines: Vec<Line<'static>> = Vec::new();
        if !self.top_padding_emitted {
            plan_lines.push(Line::from(" "));
        }
        plan_lines.extend(lines);
        if include_bottom_padding {
            plan_lines.push(Line::from(" "));
        }
        let plan_style = proposed_plan_style();
        out_lines.extend(
            prefix_lines(plan_lines, "  ".into(), "  ".into())
                .into_iter()
                .map(|line| line.style(plan_style)),
        );
        out_lines
    }
}

/// An answer's stream as the tests read it: the rows it commits and its
/// tail, without the cells' indent.
#[cfg(test)]
pub(crate) struct TestStream {
    core: StreamCore,
}

#[cfg(test)]
impl TestStream {
    pub(crate) fn new(width: Option<usize>, cwd: &str) -> Self {
        Self::with_spacing(width, cwd, ListSpacing::AfterMultiline)
    }

    pub(crate) fn with_spacing(width: Option<usize>, cwd: &str, spacing: ListSpacing) -> Self {
        Self {
            core: StreamCore::new(
                width,
                cwd.to_string(),
                crate::markdown::render_streaming_markdown_agent,
                spacing,
            ),
        }
    }

    pub(crate) fn push(&mut self, delta: &str) {
        self.core.push_delta(delta, Instant::now());
    }

    /// Every queued row, committed.
    pub(crate) fn commit(&mut self) -> Vec<Line<'static>> {
        self.core.tick_batch(usize::MAX)
    }

    pub(crate) fn tail(&self) -> Vec<Line<'static>> {
        self.core.current_tail_lines()
    }

    pub(crate) fn set_width(&mut self, width: Option<usize>) {
        self.core.set_width(width, Instant::now());
    }

    /// The rest at the stream's end, and whether a committed row differs
    /// from the final render.
    pub(crate) fn finalize(&mut self) -> Vec<Line<'static>> {
        self.finalize_checked().0
    }

    pub(crate) fn finalize_checked(&mut self) -> (Vec<Line<'static>>, bool) {
        let remaining = self.core.finalize_remaining();
        (remaining.lines, remaining.diverged)
    }
}

#[cfg(test)]
mod tests;
