// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Which rows are stable: every row but the mutable tail is queued, and
//! each tick commits rows from the queue's front. The tail starts at the
//! earliest of a table's header (`table_holdback`), a ```` ```md ````
//! fence in the last block, a last line that is a list marker alone (its
//! item's text may come on the marker's row), and a last line ending with
//! a local file link (a `:` line joins it).

use std::time::{Duration, Instant};

use crate::terminal_hyperlinks::HyperlinkLine;

use super::super::table_holdback::TableHoldbackState;
use super::state::{StablePrefixLenCache, StreamCore};
use crate::table_detect::strip_blockquote_prefix;

impl StreamCore {
    /// The first queued row, committed.
    pub(super) fn tick(&mut self) -> Vec<HyperlinkLine> {
        let step = self.state.step();
        self.note_emitted(&step);
        step
    }

    /// Up to `max_lines` queued rows, committed.
    pub(super) fn tick_batch(&mut self, max_lines: usize) -> Vec<HyperlinkLine> {
        let step = self.state.drain_n(max_lines);
        self.note_emitted(&step);
        step
    }

    fn note_emitted(&mut self, lines: &[HyperlinkLine]) {
        self.emitted_stable_len += lines.len();
        self.emitted.extend_from_slice(lines);
    }

    pub(super) fn is_idle(&self) -> bool {
        self.state.is_idle()
    }

    pub(super) fn queued_lines(&self) -> usize {
        self.state.queued_len()
    }

    pub(super) fn oldest_queued_age(&self, now: Instant) -> Option<Duration> {
        self.state.oldest_queued_age(now)
    }

    /// Where in the committed source the mutable tail starts, if anywhere.
    pub(super) fn active_tail_source_start(&self) -> Option<usize> {
        let table_start = match self.holdback_scanner.state() {
            TableHoldbackState::Confirmed { table_start }
            | TableHoldbackState::PendingHeader {
                header_start: table_start,
            } => Some(table_start),
            TableHoldbackState::None => None,
        };
        let source = self.state.collector.committed_source();
        let source = source.strip_suffix('\n').unwrap_or(source);
        let last_line_start = source.rfind('\n').map_or(0, |index| index + 1);
        let held_last_line =
            is_bare_list_marker(&source[last_line_start..]) || self.render.trailing_local_link;
        [
            table_start,
            self.render.mutable_fence_start,
            held_last_line.then_some(last_line_start),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// How many rendered rows are stable.
    fn compute_target_stable_len(&mut self) -> usize {
        let tail_budget = self.active_tail_budget_lines();
        self.render
            .lines
            .len()
            .saturating_sub(tail_budget)
            .max(self.emitted_stable_len)
    }

    /// Queues the rows that became stable: `true` when the queue grew.
    pub(super) fn sync_stable_queue(&mut self, now: Instant) -> bool {
        let target_stable_len = self.compute_target_stable_len();

        // A row arriving moved the stable boundary back into rows queued
        // but not committed: the queue is rebuilt from the new render.
        if target_stable_len < self.enqueued_stable_len {
            self.state.clear_queue();
            if self.emitted_stable_len < target_stable_len {
                self.state.enqueue(
                    self.render.lines[self.emitted_stable_len..target_stable_len].to_vec(),
                    now,
                );
            }
            self.enqueued_stable_len = target_stable_len;
            return self.state.queued_len() > 0;
        }

        if target_stable_len == self.enqueued_stable_len {
            return false;
        }

        self.state.enqueue(
            self.render.lines[self.enqueued_stable_len..target_stable_len].to_vec(),
            now,
        );
        self.enqueued_stable_len = target_stable_len;
        true
    }

    /// The queue again from the current render, after a resize made the
    /// rows queued at the old width wrong.
    pub(super) fn rebuild_stable_queue_from_render(&mut self, now: Instant) {
        let target_stable_len = self.compute_target_stable_len();
        self.state.clear_queue();
        if self.emitted_stable_len < target_stable_len {
            self.state.enqueue(
                self.render.lines[self.emitted_stable_len..target_stable_len].to_vec(),
                now,
            );
        }
        self.enqueued_stable_len = target_stable_len;
    }

    /// How many rendered rows the mutable tail takes.
    fn active_tail_budget_lines(&mut self) -> usize {
        match self.active_tail_source_start() {
            None => 0,
            Some(0) => self.render.lines.len(),
            Some(start) => {
                let start = start.min(self.state.collector.committed_source().len());
                let stable_prefix_len = self.stable_prefix_len_for_source_start(start);
                self.render.lines.len().saturating_sub(stable_prefix_len)
            }
        }
    }

    /// The rows of the source before `source_start`, rendered alone; kept,
    /// since a table arriving asks again at each of its rows.
    fn stable_prefix_len_for_source_start(&mut self, source_start: usize) -> usize {
        if let Some(cache) = self
            .stable_prefix_len_cache
            .as_ref()
            .filter(|cache| cache.source_start == source_start && cache.width == self.width)
        {
            return cache.stable_prefix_len;
        }
        let source = self.state.collector.committed_source();
        let stable_prefix_len = self
            .render
            .render_source(&source[..source_start], self.width, &self.cwd)
            .lines
            .len();
        self.stable_prefix_len_cache = Some(StablePrefixLenCache {
            source_start,
            width: self.width,
            stable_prefix_len,
        });
        stable_prefix_len
    }
}

/// `-`, `*`, `+` or `1.`/`1)` with nothing after it but a task box, its
/// quote markers left out: the item's text may come on the next line, on
/// its row.
fn is_bare_list_marker(line: &str) -> bool {
    let text = strip_blockquote_prefix(line).trim();
    let marker = ["[ ]", "[x]", "[X]"]
        .iter()
        .find_map(|task| text.strip_suffix(task))
        .map_or(text, str::trim_end);
    match marker.strip_suffix(['.', ')']) {
        Some(number) => !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()),
        None => matches!(marker, "-" | "+" | "*"),
    }
}
