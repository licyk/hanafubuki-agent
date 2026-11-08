//! Pseudo-terminal grid size and its history over an output stream.
//!
//! PTY output is only meaningful at the size it was produced at: line wrapping,
//! cursor addressing, and on Windows ConPTY's own repaints all assume the grid
//! in effect at that moment. Retained output therefore records every size
//! change at the byte offset it took effect from, and a viewer replays the
//! bytes through that same sequence of sizes rather than at its own.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// Columns and rows of a pseudo-terminal grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

impl TerminalSize {
    /// The size every PTY starts at when its creator names none.
    ///
    /// No viewer exists when a process starts, so this is a placeholder rather
    /// than a guess at any view: the first viewer corrects it with a recorded
    /// resize, and the output before that resize is replayed at this size.
    pub const DEFAULT: Self = Self::new(80, 24);

    pub const fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }
}

/// A size change at an exact output offset: bytes from `offset` on were
/// produced at `size`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalResize {
    pub offset: u64,
    #[serde(flatten)]
    pub size: TerminalSize,
}

/// The size history of one PTY's output stream.
///
/// Always holds the size in effect at the oldest retained offset, followed by
/// every later change in offset order. Changes at one offset collapse into the
/// last, so the history grows with output, not with resize requests.
#[derive(Clone, Debug)]
pub struct TerminalSizeTimeline {
    marks: VecDeque<TerminalResize>,
}

impl TerminalSizeTimeline {
    pub fn new(size: TerminalSize) -> Self {
        Self {
            marks: VecDeque::from([TerminalResize { offset: 0, size }]),
        }
    }

    pub fn current(&self) -> TerminalSize {
        self.last().size
    }

    /// Record that output from `offset` on is produced at `size`. Returns
    /// `false` when that is already the current size, so the caller can skip
    /// resizing the PTY at all.
    pub fn record(&mut self, offset: u64, size: TerminalSize) -> bool {
        if self.current() == size {
            return false;
        }
        let last = self.marks.back_mut().expect("timeline is never empty");
        if last.offset >= offset {
            // Nothing was produced at the previous size: replace it, and drop
            // it entirely when that returns to the size before it.
            last.size = size;
            let len = self.marks.len();
            if len > 1 && self.marks[len - 2].size == size {
                self.marks.pop_back();
            }
        } else {
            self.marks.push_back(TerminalResize { offset, size });
        }
        true
    }

    /// Forget history before `oldest`, keeping the size in effect there.
    pub fn trim_before(&mut self, oldest: u64) {
        while self.marks.len() > 1 && self.marks[1].offset <= oldest {
            self.marks.pop_front();
        }
    }

    /// The size bytes at `offset` were produced at.
    pub fn size_at(&self, offset: u64) -> TerminalSize {
        self.marks
            .iter()
            .rev()
            .find(|mark| mark.offset <= offset)
            .unwrap_or_else(|| self.first())
            .size
    }

    /// Changes strictly inside the byte range `start..end`. A change at `end`
    /// applies to the bytes after the range, so the next read reports it as its
    /// starting size.
    pub fn changes_between(&self, start: u64, end: u64) -> Vec<TerminalResize> {
        self.marks
            .iter()
            .filter(|mark| mark.offset > start && mark.offset < end)
            .copied()
            .collect()
    }

    fn first(&self) -> &TerminalResize {
        self.marks.front().expect("timeline is never empty")
    }

    fn last(&self) -> &TerminalResize {
        self.marks.back().expect("timeline is never empty")
    }
}

#[cfg(test)]
mod tests;
