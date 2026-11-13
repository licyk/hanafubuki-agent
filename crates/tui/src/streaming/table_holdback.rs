// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Where a stream's table starts: from its header on, rows stay the
//! stream's to change until the top-level block holding the table is
//! complete, since a row arriving can widen every column. With tables off,
//! only a line that may be a header waits for its delimiter row.
//!
//! The scan only finds where the mutable rows start; it neither validates a
//! table nor predicts its layout, which stay the renderer's.

use crate::table_detect::{
    is_table_delimiter_line, is_table_header_line, parse_table_segments, strip_blockquote_prefix,
    FenceKind, FenceTracker,
};

/// What the committed source holds, as the holdback sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TableHoldbackState {
    /// No table: every row may be committed.
    None,
    /// The last non-blank line may be a table's header; its delimiter row
    /// may come next.
    PendingHeader { header_start: usize },
    /// A header and a delimiter row: the table starts at `table_start`.
    Confirmed { table_start: usize },
}

/// What is remembered of the line before: a table is a header line
/// followed by a delimiter line.
#[derive(Clone, Copy)]
struct PreviousLineState {
    source_start: usize,
    fence_kind: FenceKind,
    is_header: bool,
}

/// The holdback over append-only source, scanned incrementally. A table is
/// released only once its top-level block is complete; the rest is scanned
/// again from that block for later tables.
pub(super) struct TableHoldbackScanner {
    source_offset: usize,
    fence_tracker: FenceTracker,
    previous_line: Option<PreviousLineState>,
    pending_header_start: Option<usize>,
    confirmed_table_start: Option<usize>,
    /// Inside a table's rows, which a blank line ends.
    in_table: bool,
}

impl TableHoldbackScanner {
    pub(super) fn new() -> Self {
        Self {
            source_offset: 0,
            fence_tracker: FenceTracker::new(),
            previous_line: None,
            pending_header_start: None,
            confirmed_table_start: None,
            in_table: false,
        }
    }

    pub(super) fn reset(&mut self) {
        *self = Self::new();
    }

    pub(super) fn state(&self) -> TableHoldbackState {
        if let Some(table_start) = self.confirmed_table_start {
            TableHoldbackState::Confirmed { table_start }
        } else if let Some(header_start) = self.pending_header_start {
            TableHoldbackState::PendingHeader { header_start }
        } else {
            TableHoldbackState::None
        }
    }

    /// The unterminated line may be previewed only outside tables and
    /// fences.
    pub(super) fn allows_prose_preview(&self) -> bool {
        !self.in_table
            && !self.previous_line.is_some_and(|line| line.is_header)
            && self.fence_tracker.kind() == FenceKind::Outside
    }

    /// Scans the source committed since the last scan (complete lines).
    /// When the renderer's completed blocks reach past what is held, the
    /// scan starts again from the last top-level block, where no fence is
    /// open.
    pub(super) fn scan(&mut self, source: &str, completed_source_len: usize) {
        let holdback_start = match self.state() {
            TableHoldbackState::Confirmed { table_start } => Some(table_start),
            TableHoldbackState::PendingHeader { header_start } => Some(header_start),
            TableHoldbackState::None => None,
        };
        if self.source_offset < completed_source_len
            || holdback_start.is_some_and(|start| start < completed_source_len)
        {
            self.reset();
            self.source_offset = completed_source_len;
        }
        let Some(source_chunk) = source.get(self.source_offset..) else {
            return;
        };
        for source_line in source_chunk.split_inclusive('\n') {
            self.push_line(source_line);
        }
    }

    fn push_line(&mut self, source_line: &str) {
        let line = source_line.strip_suffix('\n').unwrap_or(source_line);
        let source_start = self.source_offset;
        let fence_kind = self.fence_tracker.kind();

        let candidate_text = if fence_kind == FenceKind::Other {
            None
        } else {
            table_candidate_text(line)
        };
        let is_header = candidate_text.is_some_and(is_table_header_line);
        let is_delimiter = candidate_text.is_some_and(is_table_delimiter_line);

        if strip_blockquote_prefix(line).is_empty() {
            self.in_table = false;
        }
        let previous_header = self.previous_line.filter(|previous| {
            previous.fence_kind != FenceKind::Other
                && fence_kind != FenceKind::Other
                && previous.is_header
        });
        if let Some(previous) = previous_header.filter(|_| is_delimiter) {
            if crate::markdown_render::preferences::current().tables {
                self.confirmed_table_start
                    .get_or_insert(previous.source_start);
            }
            self.pending_header_start = None;
            self.in_table = true;
        }

        if self.confirmed_table_start.is_none() && !line.trim().is_empty() {
            self.pending_header_start =
                (fence_kind != FenceKind::Other && is_header).then_some(source_start);
        }

        self.previous_line = Some(PreviousLineState {
            source_start,
            fence_kind,
            is_header,
        });

        self.fence_tracker.advance(line);
        if fence_kind != FenceKind::Outside && self.fence_tracker.kind() == FenceKind::Outside {
            self.in_table = false;
        }
        self.source_offset = self.source_offset.saturating_add(source_line.len());
    }
}

/// The line without its quote markers, when it has the shape of a table's
/// row: a quoted table is a table.
fn table_candidate_text(line: &str) -> Option<&str> {
    let stripped = strip_blockquote_prefix(line).trim();
    parse_table_segments(stripped).map(|_| stripped)
}

/// The holdback of `source` scanned at once, for the tests to compare the
/// incremental scan with.
#[cfg(test)]
pub(super) fn table_holdback_state(source: &str) -> TableHoldbackState {
    struct ParsedLine<'a> {
        text: &'a str,
        fence_context: FenceKind,
        source_start: usize,
    }
    let mut tracker = FenceTracker::new();
    let mut lines = Vec::new();
    let mut source_start = 0usize;
    for raw_line in source.split('\n') {
        lines.push(ParsedLine {
            text: raw_line,
            fence_context: tracker.kind(),
            source_start,
        });
        tracker.advance(raw_line);
        source_start = source_start + raw_line.len() + 1;
    }
    for pair in lines.windows(2) {
        let [header_line, delimiter_line] = pair else {
            continue;
        };
        if header_line.fence_context == FenceKind::Other
            || delimiter_line.fence_context == FenceKind::Other
        {
            continue;
        }
        let header = table_candidate_text(header_line.text);
        let delimiter = table_candidate_text(delimiter_line.text);
        if header.is_some_and(is_table_header_line)
            && delimiter.is_some_and(is_table_delimiter_line)
        {
            return TableHoldbackState::Confirmed {
                table_start: header_line.source_start,
            };
        }
    }
    let pending_header = lines.iter().rev().find(|line| !line.text.trim().is_empty());
    match pending_header {
        Some(line)
            if line.fence_context != FenceKind::Other
                && table_candidate_text(line.text).is_some_and(is_table_header_line) =>
        {
            TableHoldbackState::PendingHeader {
                header_start: line.source_start,
            }
        }
        _ => TableHoldbackState::None,
    }
}

#[cfg(test)]
mod tests;
