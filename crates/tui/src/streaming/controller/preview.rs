// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What the stream shows past its queued rows: the mutable tail, then the
//! preview of the line still arriving, outside tables and fences, or as
//! its source inside a display formula.

use crate::terminal_hyperlinks::HyperlinkLine;

use super::super::prose_preview::{PreviewMode, ProsePreview};
use super::state::StreamCore;

impl StreamCore {
    /// Updates the preview of the unterminated line: `true` when it
    /// changed.
    pub(super) fn refresh_preview(&mut self) -> bool {
        let pending = self.state.collector.pending_source();
        let prose = self.holdback_scanner.allows_prose_preview();
        let math = self.render.pending_math_start.is_some()
            || prose && (pending.starts_with("$$") || pending.starts_with("\\["));
        if !math && !prose {
            let changed = !self.preview.lines.is_empty();
            self.preview = ProsePreview::default();
            return changed;
        }
        let mode = if math {
            PreviewMode::Math
        } else {
            PreviewMode::Prose(self.render.renderer)
        };
        self.preview.update(pending, self.width, &self.cwd, mode)
    }

    /// The rows past the queued ones, which may still change: they start
    /// at the queued count, so a row queued but not committed never shows
    /// twice.
    pub(super) fn current_tail_lines(&self) -> Vec<HyperlinkLine> {
        let start = self.enqueued_stable_len.min(self.render.lines.len());
        let mut lines = self.render.lines[start..].to_vec();
        lines.extend(self.preview.lines.iter().cloned());
        lines
    }

    /// Whether rendered rows are held as the tail; a preview is not.
    pub(super) fn has_tail(&self) -> bool {
        self.enqueued_stable_len < self.render.lines.len()
    }
}
