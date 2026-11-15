// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What the person sent that the conversation has not taken yet, shown
//! above the composer (Codex's `pending_input_preview.rs`): the steering
//! messages the running turn reads at its next step, with the key that
//! interrupts it to send them at once; the messages sent at the turn's end
//! (sent while a review ran, or a steer that did not reach the turn); and
//! the messages queued with Tab, with the key that puts the last one back
//! in the composer. A message shows its first three lines. Codex's links
//! and the questions sharing the group are not ported.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::keymap::{self, Action};
use crate::render::renderable::Renderable;
use crate::wrapping::{word_wrap_lines, RtOptions};

/// The lines of a message shown; a longer one ends with `…`.
const PREVIEW_LINE_LIMIT: usize = 3;

/// The messages waiting, oldest first in each group.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PendingInput {
    /// Sent with Enter, not yet read by the running turn.
    pub(crate) steers: Vec<String>,
    /// Sent once the turn (or the review) ends, together.
    pub(crate) held: Vec<String>,
    /// Queued with Tab, each starting a turn of its own.
    pub(crate) queued: Vec<String>,
}

impl PendingInput {
    pub(crate) fn is_empty(&self) -> bool {
        self.steers.is_empty() && self.held.is_empty() && self.queued.is_empty()
    }

    fn lines(&self, width: u16) -> Vec<Line<'static>> {
        if self.is_empty() || width < 4 {
            return Vec::new();
        }
        let width = usize::from(width);
        let mut lines = Vec::new();
        if !self.steers.is_empty() {
            let mut header = vec![Span::from("Messages to be submitted after next tool call")];
            if let Some(key) = keymap::hint(Action::InterruptTurn) {
                header.push(" (press ".dim());
                header.push(key.into());
                header.push(" to interrupt and send immediately)".dim());
            }
            push_section(&mut lines, width, header, &self.steers, false);
        }
        if !self.held.is_empty() {
            let header = vec![Span::from("Messages to be submitted at end of turn")];
            push_section(&mut lines, width, header, &self.held, false);
        }
        if !self.queued.is_empty() {
            let header = vec![Span::from("Queued follow-up inputs")];
            push_section(&mut lines, width, header, &self.queued, true);
            if let Some(key) = keymap::hint(Action::EditQueuedMessage) {
                lines.push(Line::from(vec![
                    "    ".into(),
                    key.into(),
                    " edit last queued message".dim(),
                ]));
            }
        }
        lines
    }
}

/// A group's header, then each message under it, after a blank line when
/// a group came before.
fn push_section(
    lines: &mut Vec<Line<'static>>,
    width: usize,
    header: Vec<Span<'static>>,
    messages: &[String],
    italic: bool,
) {
    if !lines.is_empty() {
        lines.push(Line::from(""));
    }
    let mut spans = vec!["• ".dim()];
    spans.extend(header);
    lines.extend(word_wrap_lines(
        [Line::from(spans)],
        RtOptions::new(width).subsequent_indent(Line::from("  ")),
    ));
    for message in messages {
        let source = message.lines().take(PREVIEW_LINE_LIMIT + 1).map(|line| {
            let span = line.to_string().dim();
            Line::from(if italic { span.italic() } else { span })
        });
        let wrapped = word_wrap_lines(
            source,
            RtOptions::new(width)
                .initial_indent(Line::from("  ↳ ".dim()))
                .subsequent_indent(Line::from("    ")),
        );
        let more = wrapped.len() > PREVIEW_LINE_LIMIT;
        lines.extend(wrapped.into_iter().take(PREVIEW_LINE_LIMIT));
        if more {
            let ellipsis = "    …".dim();
            lines.push(Line::from(if italic {
                ellipsis.italic()
            } else {
                ellipsis
            }));
        }
    }
}

impl Renderable for PendingInput {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        if !area.is_empty() {
            Paragraph::new(self.lines(area.width)).render(area, buf);
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        u16::try_from(self.lines(width).len()).unwrap_or(u16::MAX)
    }
}

#[cfg(test)]
mod tests;
