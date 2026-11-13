// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A stream's end (Codex's `app/agent_message_consolidation.rs`): the cells
//! an answer's or a plan's rows were committed in, rendered at the width
//! its stream started at, give way to one cell of its Markdown, which
//! renders for the width it is shown at. The transcript, the Ctrl+T overlay
//! and Fullscreen's view take it in their place; a reader inside them keeps
//! what it reads (`TranscriptView::replace_entries`).
//!
//! Scrollback's tail keeps the stream's rows it has not written, which at
//! the same width are the cell's; when the terminal was resized while they
//! were in the transcript, the scrollback is written again from the cell
//! (`resize_reflow`).

use std::sync::Arc;
use std::time::Instant;

use super::App;
use crate::history_cell::{splice_stream, stream_span, HistoryCell};
use crate::pager_overlay::Overlay;
use crate::streaming::controller::Consolidation;

impl App {
    pub(super) fn consolidate_stream(&mut self, consolidation: Consolidation) {
        let Consolidation { stream, cell } = consolidation;
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
        if !self.mode.is_owned() && !parked && self.reflow.take_stream_repair() {
            self.reflow.schedule_immediate(Instant::now());
        }
        self.schedule_frame(Instant::now());
    }
}
