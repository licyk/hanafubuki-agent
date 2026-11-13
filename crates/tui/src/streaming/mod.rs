// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use std::collections::VecDeque;

use ratatui::text::Line;

use crate::markdown_stream::{MarkdownStreamCollector, RenderMarkdown};
pub(crate) mod controller;

pub(crate) struct StreamState {
    pub(crate) collector: MarkdownStreamCollector,
    queued_lines: VecDeque<Line<'static>>,
    pub(crate) has_seen_delta: bool,
}

impl StreamState {
    pub(crate) fn new(width: Option<usize>, cwd: Option<String>, render: RenderMarkdown) -> Self {
        Self {
            collector: MarkdownStreamCollector::new(width, cwd, render),
            queued_lines: VecDeque::new(),
            has_seen_delta: false,
        }
    }
    pub(crate) fn clear(&mut self) {
        self.collector.clear();
        self.queued_lines.clear();
        self.has_seen_delta = false;
    }
    pub(crate) fn step(&mut self) -> Vec<Line<'static>> {
        self.queued_lines.pop_front().into_iter().collect()
    }
    pub(crate) fn drain_all(&mut self) -> Vec<Line<'static>> {
        self.queued_lines.drain(..).collect()
    }
    pub(crate) fn is_idle(&self) -> bool {
        self.queued_lines.is_empty()
    }
    pub(crate) fn enqueue(&mut self, lines: Vec<Line<'static>>) {
        self.queued_lines.extend(lines);
    }

    /// Pushes a delta; when it holds a newline, the lines it completed are
    /// queued. `true` when there are new lines to commit.
    pub(crate) fn push(&mut self, delta: &str) -> bool {
        if !delta.is_empty() {
            self.has_seen_delta = true;
        }
        self.collector.push_delta(delta);
        if delta.contains('\n') {
            let newly_completed = self.collector.commit_complete_lines();
            if !newly_completed.is_empty() {
                self.enqueue(newly_completed);
                return true;
            }
        }
        false
    }

    /// The stream's end: every row not committed yet, queued or not; the
    /// state is cleared for the next stream.
    pub(crate) fn finalize(&mut self) -> Vec<Line<'static>> {
        let remaining = self.collector.finalize_and_drain();
        self.enqueue(remaining);
        let lines = self.drain_all();
        self.clear();
        lines
    }
}
