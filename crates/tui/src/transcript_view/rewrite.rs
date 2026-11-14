// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The rows Scrollback writes again after a resize (Codex's
//! `render_transcript_lines_for_reflow`): the entries written before the
//! tail, each as the tail writes it, laid out for the new width, the latest
//! up to a cap, under a notice of where the earlier ones are.

use ratatui::style::Stylize;
use ratatui::text::Span;

use super::*;
use crate::keymap::{self, Action};
use crate::terminal_hyperlinks::{plain_hyperlink_lines, HyperlinkLine};
use crate::wrapping::word_wrap_lines;

impl TranscriptView {
    /// The rows Scrollback writes again for `cells`, the transcript before
    /// its tail, at `width` (Codex's resize reflow): each entry as the tail
    /// writes it, at most the last `cap` rows, under a notice of where the
    /// earlier ones are when some are left out. Laid out from the last
    /// entry back, so a long transcript is laid out only as far as the cap.
    pub(crate) fn scrollback_lines(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        width: u16,
        cap: usize,
    ) -> Vec<HyperlinkLine> {
        self.cache.begin_frame();
        self.scrollback_tail = true;
        self.prepare_width(width);
        let mut entries = std::collections::VecDeque::new();
        let mut rows = 0;
        let mut truncated = false;
        for index in (0..cells.len()).rev() {
            let lines = self.tail_lines(cells, index, false);
            rows += lines.len();
            entries.push_front(lines);
            if rows > cap {
                truncated = true;
                break;
            }
        }
        let mut lines = entries.into_iter().flatten().collect::<Vec<_>>();
        if lines.len() > cap {
            lines.drain(..lines.len() - cap);
        }
        if truncated {
            let notice = plain_hyperlink_lines(word_wrap_lines(
                [earlier_notice()],
                usize::from(width.max(1)),
            ));
            // A cap too small for the notice keeps the conversation.
            if let Some(room) = cap.checked_sub(notice.len()).filter(|room| *room > 0) {
                lines.drain(..lines.len().saturating_sub(room));
                lines.splice(0..0, notice);
            }
        }
        lines
    }
}

/// What heads a rewritten scrollback that leaves earlier rows out.
fn earlier_notice() -> Line<'static> {
    let mut spans = vec![Span::from("Earlier messages are available — ")];
    match keymap::hint(Action::OpenTranscript) {
        Some(key) => {
            spans.push("press ".into());
            spans.push(key.into());
            spans.push(" to view the full transcript".into());
        }
        None => spans.push("view the full transcript".into()),
    }
    Line::from(spans).dim()
}

#[cfg(test)]
mod tests;
