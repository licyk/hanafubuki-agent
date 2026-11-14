// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A streamed answer or plan, rendered as it arrives (Codex's streaming
//! pipeline):
//!
//! - `render`: the committed source (complete lines) rendered incrementally,
//!   the blocks before the last one kept; `code_fence` extends an open
//!   code fence a line at a time;
//! - `controller`: the rendered rows split into the stable ones, queued to
//!   be committed, and the mutable tail (a table arriving, a ```` ```md ````
//!   fence, a list marker alone), shown live after them; `table_holdback`
//!   finds a table's start; `prose_preview` shows the line still arriving;
//! - `chunking` and `commit_tick`: how many queued rows each tick commits.
//!
//! [`StreamState`] holds the source and the queue of rows to commit, in
//! order: rows are drained from its front only, each with the time it was
//! queued, which the pacing reads.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::terminal_hyperlinks::HyperlinkLine;

use crate::markdown_stream::MarkdownStreamCollector;

pub(crate) mod chunking;
mod code_fence;
pub(crate) mod commit_tick;
pub(crate) mod controller;
mod prose_preview;
mod render;
mod table_holdback;

struct QueuedLine {
    line: HyperlinkLine,
    enqueued_at: Instant,
}

/// A stream's source and its rows queued to be committed.
pub(crate) struct StreamState {
    pub(crate) collector: MarkdownStreamCollector,
    queued_lines: VecDeque<QueuedLine>,
}

impl StreamState {
    pub(crate) fn new() -> Self {
        Self {
            collector: MarkdownStreamCollector::new(),
            queued_lines: VecDeque::new(),
        }
    }

    /// The first queued row.
    pub(crate) fn step(&mut self) -> Vec<HyperlinkLine> {
        self.queued_lines
            .pop_front()
            .map(|queued| queued.line)
            .into_iter()
            .collect()
    }

    /// Up to `max_lines` queued rows from the front.
    pub(crate) fn drain_n(&mut self, max_lines: usize) -> Vec<HyperlinkLine> {
        let end = max_lines.min(self.queued_lines.len());
        self.queued_lines
            .drain(..end)
            .map(|queued| queued.line)
            .collect()
    }

    pub(crate) fn clear_queue(&mut self) {
        self.queued_lines.clear();
    }

    pub(crate) fn is_idle(&self) -> bool {
        self.queued_lines.is_empty()
    }

    pub(crate) fn queued_len(&self) -> usize {
        self.queued_lines.len()
    }

    pub(crate) fn oldest_queued_age(&self, now: Instant) -> Option<Duration> {
        self.queued_lines
            .front()
            .map(|queued| now.saturating_duration_since(queued.enqueued_at))
    }

    /// Queues `lines`, all at `now`.
    pub(crate) fn enqueue(&mut self, lines: Vec<HyperlinkLine>, now: Instant) {
        self.queued_lines
            .extend(lines.into_iter().map(|line| QueuedLine {
                line,
                enqueued_at: now,
            }));
    }
}

#[cfg(test)]
mod tests;
