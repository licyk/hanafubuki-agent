// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A running call's output as it streams, after Codex's
//! `LiveCommandOutput`: the lines a terminal would show, bounded. The first
//! and the newest lines are kept, with how many fell between them; each
//! line is cut at a bound, so output that never ends a line cannot grow the
//! entry either.
//!
//! The text arrives as terminal text: a pipe's as the process wrote it, a
//! PTY's as its screen's rows, a row the last update left unfinished written
//! again after `\r ESC[2K`. Only lines ended by a newline are committed; the
//! line still being written is kept raw and shown as it renders now, so a
//! `\r` redraw (a progress bar, a PTY's repainted row) replaces it.

use std::collections::VecDeque;

use hanafubuki_agent::{complete_prefix_len, render_plain_text};

use super::expand_tabs;

/// The first lines kept, and the newest.
const HEAD_LINES: usize = 200;
const TAIL_LINES: usize = 200;
/// A line's bytes kept, as shown; the rest is cut.
const MAX_LINE_BYTES: usize = 1024;
/// Raw text of the unfinished line kept before it is collapsed to what it
/// shows, so a progress bar redrawn with `\r` cannot grow it without limit.
const MAX_PENDING_BYTES: usize = 16 * 1024;
/// The line that stands for output that was skipped.
const GAP: &str = "…";

#[derive(Debug, Default)]
pub(crate) struct LiveOutput {
    head: Vec<String>,
    /// The newest lines, once the head is full.
    tail: VecDeque<String>,
    /// Lines dropped between the head and the tail.
    omitted: usize,
    /// The raw text after the last newline.
    pending: String,
}

impl LiveOutput {
    /// Adds `text`; `truncated` says output was skipped before it.
    pub(crate) fn push(&mut self, text: &str, truncated: bool) {
        if truncated {
            // The line before the gap is never finished.
            let pending = std::mem::take(&mut self.pending);
            if !pending.is_empty() {
                self.commit(&render_plain_text(pending.as_bytes()));
            }
            self.commit(GAP);
        }
        self.pending.push_str(text);
        if let Some(end) = self.pending.rfind('\n') {
            let complete: String = self.pending.drain(..=end).collect();
            for line in render_plain_text(complete.as_bytes()).lines() {
                self.commit(line);
            }
        }
        if self.pending.len() > MAX_PENDING_BYTES {
            let complete = complete_prefix_len(self.pending.as_bytes());
            let mut shown = render_plain_text(&self.pending.as_bytes()[..complete]);
            cut(&mut shown);
            self.pending.replace_range(..complete, &shown);
        }
    }

    /// The lines to show, the omitted ones as one line between the first
    /// and the newest, without trailing blank lines; and how many lines of
    /// output they stand for.
    pub(crate) fn lines(&self) -> (Vec<String>, usize) {
        let mut lines = self.head.clone();
        if self.omitted > 0 {
            lines.push(format!("… +{} lines", self.omitted));
        }
        lines.extend(self.tail.iter().cloned());
        let complete = complete_prefix_len(self.pending.as_bytes());
        let mut pending = expand_tabs(&render_plain_text(&self.pending.as_bytes()[..complete]));
        cut(&mut pending);
        lines.push(pending);
        while lines.last().is_some_and(|line| line.trim().is_empty()) {
            lines.pop();
        }
        if let Some(last) = lines.last_mut() {
            last.truncate(last.trim_end().len());
        }
        let count = lines.len() + self.omitted - usize::from(self.omitted > 0);
        (lines, count)
    }

    fn commit(&mut self, line: &str) {
        let mut line = expand_tabs(line);
        cut(&mut line);
        if self.head.len() < HEAD_LINES {
            self.head.push(line);
            return;
        }
        if self.tail.len() == TAIL_LINES {
            self.tail.pop_front();
            self.omitted += 1;
        }
        self.tail.push_back(line);
    }
}

/// Cuts `line` to [`MAX_LINE_BYTES`] on a character boundary, marking the
/// cut.
fn cut(line: &mut String) {
    if line.len() <= MAX_LINE_BYTES {
        return;
    }
    let mut end = MAX_LINE_BYTES;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    line.truncate(end);
    line.push('…');
}

#[cfg(test)]
mod tests;
