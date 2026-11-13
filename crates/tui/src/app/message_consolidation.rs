// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A stream in the transcript (Codex's active stream tail and
//! `app/agent_message_consolidation.rs`).
//!
//! While it streams, its committed rows are cells that do not change, and
//! its mutable tail one live entry after them, which its first update
//! inserts: each update puts the rows it committed before the tail and
//! gives the tail its new rows at once. Its last empties the tail, which
//! Scrollback's tail may then write (as nothing).
//!
//! At its end, the cells its rows were committed in, rendered at the width
//! the stream had, give way to one cell of its Markdown, which renders for
//! the width it is shown at. The transcript, the Ctrl+T overlay and
//! Fullscreen's view take it in their place; a reader inside them keeps
//! what it reads (`TranscriptView::replace_entries`).
//!
//! Scrollback's tail keeps the stream's rows it has not written, which at
//! the same width are the cell's; when the terminal was resized while they
//! were in the transcript, or a committed row is not the cell's (a
//! reference definition arriving last, an underline making a heading), the
//! scrollback is written again from the cell (`resize_reflow`).

use std::sync::Arc;
use std::time::Instant;

use super::App;
use crate::history_cell::{insert_before, splice_stream, stream_span, HistoryCell};
use crate::pager_overlay::Overlay;
use crate::streaming::controller::{Consolidation, StreamUpdate, TailRows};

impl App {
    pub(super) fn apply_stream_update(&mut self, update: StreamUpdate) {
        let StreamUpdate {
            tail,
            committed,
            shown,
            ended,
        } = update;
        let entry: Arc<dyn HistoryCell> = tail.clone();
        let present = self
            .transcript_cells
            .iter()
            .rev()
            .any(|cell| Arc::ptr_eq(cell, &entry));
        if let Some(cell) = committed {
            let cell: Arc<dyn HistoryCell> = cell.into();
            if present {
                self.insert_stream_cell(cell, &entry);
            } else {
                self.insert_history_cell(cell, false);
            }
        }
        if let Some(TailRows(rows)) = shown {
            tail.update(|tail| tail.show(rows));
        }
        if ended {
            self.tail.complete(&entry);
        } else if !present {
            self.insert_history_cell(entry, true);
        }
        self.schedule_frame(Instant::now());
    }

    /// A stream's committed rows, before its tail entry.
    fn insert_stream_cell(&mut self, cell: Arc<dyn HistoryCell>, tail: &Arc<dyn HistoryCell>) {
        if let (Some(Overlay::Transcript(transcript)), false) =
            (&mut self.overlay, self.workspace.handling_parked())
        {
            transcript.insert_cell_before(cell.clone(), tail);
        }
        if !self.mode.is_owned() {
            self.tail.insert_before(cell.clone(), tail);
        }
        insert_before(&mut self.transcript_cells, cell, tail);
    }

    pub(super) fn consolidate_stream(&mut self, consolidation: Consolidation) {
        let Consolidation {
            stream,
            cell,
            repair,
        } = consolidation;
        let cell: Arc<dyn HistoryCell> = cell.into();
        let parked = self.workspace.handling_parked();
        if let (Some(Overlay::Transcript(transcript)), false) = (&mut self.overlay, parked) {
            transcript.consolidate(stream, Arc::clone(&cell));
        }
        let Some(span) = stream_span(&self.transcript_cells, stream) else {
            return;
        };
        if self.mode.is_owned() {
            self.transcript_view
                .replace_entries(&self.transcript_cells, span.clone(), &cell);
        }
        splice_stream(&mut self.transcript_cells, span, stream, cell);
        if !self.mode.is_owned() && !parked && (self.reflow.take_stream_repair() || repair) {
            self.reflow.schedule_immediate(Instant::now());
        }
        self.schedule_frame(Instant::now());
    }
}
