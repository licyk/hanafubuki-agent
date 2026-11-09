// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The live tail a transcript view shows after the committed cells: the
//! thinking and the tool calls still running, with a key that changes
//! whenever their lines can change, so the view lays them out again only
//! then. Codex has one active cell; here every live cell is part of it.

use ratatui::text::Line;

use super::ChatWidget;
use crate::history_cell::HistoryCell;

/// What identifies the live tail's lines at a width.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ActiveCellTranscriptKey {
    /// Whether an unchanged key may reuse the laid-out tail.
    pub(crate) cacheable: bool,
    /// Advanced by every change to the live cells.
    pub(crate) revision: u64,
    /// Whether the tail continues the streamed message before it, which
    /// leaves out the blank line between them.
    pub(crate) is_stream_continuation: bool,
    /// Changes while the tail's lines change with time (a spinner, a timer).
    pub(crate) animation_tick: Option<u64>,
}

#[allow(dead_code, reason = "P2.2 draws the live tail in the transcript view")]
impl ChatWidget {
    pub(crate) fn active_cell_transcript_key(&self) -> Option<ActiveCellTranscriptKey> {
        let cells = self.active_cells();
        if cells.is_empty() {
            return None;
        }
        Some(ActiveCellTranscriptKey {
            cacheable: cells.iter().all(|cell| cell.has_stable_transcript_height()),
            revision: self.live_revision,
            is_stream_continuation: cells[0].is_stream_continuation(),
            animation_tick: cells
                .iter()
                .filter_map(|cell| cell.transcript_animation_tick())
                .max(),
        })
    }

    /// The live cells' transcript, a blank line between two cells; `None`
    /// when nothing is live.
    pub(crate) fn active_cell_transcript_lines(&self, width: u16) -> Option<Vec<Line<'static>>> {
        let mut lines = Vec::new();
        for cell in self.active_cells() {
            let cell_lines = cell.transcript_lines(width);
            if !lines.is_empty() && !cell_lines.is_empty() {
                lines.push(Line::default());
            }
            lines.extend(cell_lines);
        }
        (!lines.is_empty()).then_some(lines)
    }

    /// The live cells in the order the viewport draws them, for a
    /// transcript view's live tail.
    pub(crate) fn active_cells(&self) -> Vec<&dyn HistoryCell> {
        self.live_thinking
            .iter()
            .map(|(_, cell)| cell as &dyn HistoryCell)
            .chain(
                self.running_tools
                    .iter()
                    .map(|cell| cell as &dyn HistoryCell),
            )
            .collect()
    }
}
