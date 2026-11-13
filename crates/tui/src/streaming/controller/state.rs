// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What a stream keeps: its source, its incremental render at the current
//! width, the rows queued and committed, the preview, and the holdback; how
//! a delta, a resize and the stream's end change them.

use std::time::Instant;

use ratatui::text::Line;

use super::super::prose_preview::ProsePreview;
use super::super::render::{StreamRenderer, StreamingRender};
use super::super::table_holdback::TableHoldbackScanner;
use super::super::StreamState;
use crate::markdown_render::ListSpacing;

/// The bookkeeping both controllers share. Rows go through three counts,
/// `emitted_stable_len <= enqueued_stable_len <= render.lines.len()`:
/// committed, queued, and the mutable tail after them.
pub(in crate::streaming) struct StreamCore {
    pub(super) state: StreamState,
    /// The columns the Markdown is rendered in.
    pub(super) width: Option<usize>,
    pub(super) render: StreamingRender,
    /// The line still arriving, never counted in the rows above.
    pub(super) preview: ProsePreview,
    pub(super) enqueued_stable_len: usize,
    pub(super) emitted_stable_len: usize,
    /// The rows committed, to compare with the final render at the end.
    pub(super) emitted: Vec<Line<'static>>,
    /// The session's directory, local file links are shown relative to.
    pub(super) cwd: String,
    /// The rows of the source before a table's start, at a width.
    pub(super) stable_prefix_len_cache: Option<StablePrefixLenCache>,
    pub(super) holdback_scanner: TableHoldbackScanner,
}

pub(in crate::streaming) struct StablePrefixLenCache {
    pub(super) source_start: usize,
    pub(super) width: Option<usize>,
    pub(super) stable_prefix_len: usize,
}

/// The stream's end: the rows not committed yet, its whole source, and
/// whether a row committed differs from the final render (a reference
/// definition, an underline making a heading, a resize), so the scrollback
/// must be written again.
pub(in crate::streaming) struct Remaining {
    pub(super) lines: Vec<Line<'static>>,
    pub(super) source: String,
    pub(super) diverged: bool,
}

impl StreamCore {
    pub(super) fn new(
        width: Option<usize>,
        cwd: String,
        renderer: StreamRenderer,
        list_spacing: ListSpacing,
    ) -> Self {
        Self {
            state: StreamState::new(),
            width,
            render: StreamingRender::new(renderer, list_spacing),
            preview: ProsePreview::default(),
            enqueued_stable_len: 0,
            emitted_stable_len: 0,
            emitted: Vec::new(),
            cwd,
            stable_prefix_len_cache: None,
            holdback_scanner: TableHoldbackScanner::new(),
        }
    }

    /// Pushes a delta: `true` when rows were queued or the preview changed.
    ///
    /// Only source ending with a newline reaches the render: a table's
    /// unterminated row stays out of the queue and the tail until its shape
    /// is known; the preview shows unterminated prose apart.
    pub(super) fn push_delta(&mut self, delta: &str, now: Instant) -> bool {
        self.state.collector.push_delta(delta);
        if delta.contains('\n') {
            self.preview = ProsePreview::default();
        }
        let mut enqueued = false;
        if delta.contains('\n') {
            if let Some(range) = self.state.collector.commit_complete_source() {
                let source = self.state.collector.committed_source();
                self.render
                    .append(source, &source[range], self.width, &self.cwd);
                self.holdback_scanner
                    .scan(source, self.render.completed_source_len);
                enqueued = self.sync_stable_queue(now);
            }
        }
        let preview_changed = self.refresh_preview();
        enqueued || preview_changed
    }

    /// The stream's end: the source rendered whole once more, past the
    /// rows committed, in the stream's spacing so the count of committed
    /// rows still holds.
    pub(super) fn finalize_remaining(&mut self) -> Remaining {
        let source = self.state.collector.finalize_and_take_source();
        let mut rendered = self
            .render
            .render_source(&source, self.width, &self.cwd)
            .lines;
        let remaining = rendered.split_off(self.emitted_stable_len.min(rendered.len()));
        Remaining {
            lines: remaining,
            diverged: rendered != self.emitted,
            source,
        }
    }

    /// At a new width: rendered again, the committed rows kept, the queue
    /// rebuilt from the new render. Rows are not mapped byte for byte;
    /// what was committed at the old width is repaired when the stream is
    /// consolidated.
    pub(super) fn set_width(&mut self, width: Option<usize>, now: Instant) {
        if self.width == width {
            return;
        }
        let had_pending_queue = self.state.queued_len() > 0;
        let had_live_tail = self.has_tail();
        let previous_width = self.width;
        self.width = width;
        if self.state.collector.committed_source().is_empty() {
            self.refresh_preview();
            return;
        }

        self.recompute_render(previous_width);
        self.refresh_preview();
        if had_pending_queue
            && self.emitted_stable_len == self.render.lines.len()
            && self.emitted_stable_len > 0
        {
            // What was queued may fit fewer rows at the new width: one row
            // stays uncommitted so it is not skipped.
            self.emitted_stable_len -= 1;
        }
        self.state.clear_queue();
        if self.emitted_stable_len > 0 && !had_pending_queue && !had_live_tail {
            // Nothing waited: what was committed is not committed again.
            self.emitted_stable_len = self.render.lines.len();
            self.enqueued_stable_len = self.render.lines.len();
            return;
        }
        self.rebuild_stable_queue_from_render(now);
    }

    /// Rendered again whole at the current width; the committed count is
    /// carried over the rows before the tail's start, which a width
    /// changes.
    fn recompute_render(&mut self, previous_width: Option<usize>) {
        let previous_tail_start = self.active_tail_source_start();
        let source = self.state.collector.committed_source();
        self.render.recompute(source, self.width, &self.cwd);
        self.holdback_scanner
            .scan(source, self.render.completed_source_len);
        if let Some(start) = previous_tail_start.or(self.active_tail_source_start()) {
            let source = &self.state.collector.committed_source()[..start];
            let previous_prefix_len = self
                .render
                .render_source(source, previous_width, &self.cwd)
                .lines
                .len();
            let prefix_len = self
                .render
                .render_source(source, self.width, &self.cwd)
                .lines
                .len();
            self.emitted_stable_len = if self.emitted_stable_len >= previous_prefix_len {
                prefix_len.saturating_add(self.emitted_stable_len - previous_prefix_len)
            } else {
                self.emitted_stable_len.min(prefix_len)
            };
        }
        self.emitted_stable_len = self.emitted_stable_len.min(self.render.lines.len());
    }
}
