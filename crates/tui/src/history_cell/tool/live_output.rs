// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A running call's output as it streams, after Codex's
//! `LiveCommandOutput`: the lines a terminal would show, bounded. The first
//! and the newest lines are kept, with how many fell between them; each
//! line is cut at a bound, so output that never ends a line cannot grow the
//! entry either.
//!
//! The text arrives as terminal text: a pipe's as the process wrote it, a
//! PTY's as its screen's rows (each styled run between its SGR sequence and
//! a reset), a row the last update left unfinished written again after
//! `\r ESC[2K`. Only lines ended by a newline are committed; the line still
//! being written is kept raw and shown as it renders now, so a `\r` redraw
//! (a progress bar, a PTY's repainted row) replaces it.
//!
//! Lines keep the colours of their SGR sequences, as the finished output
//! shows them: a style goes on from one line and one update to the next,
//! as in a terminal, and starts plain again after a gap.

use std::collections::VecDeque;

use hanafubuki_agent::{
    complete_prefix_len, render_styled_text, sgr_sequence, styled_terminal_text, AgentStyledText,
    AgentTextStyle, StyledRender,
};
use ratatui::text::Line;

use super::styled_output::{line_text, styled_line, truncate_line};

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
    head: Vec<Line<'static>>,
    /// The newest lines, once the head is full.
    tail: VecDeque<Line<'static>>,
    /// Lines dropped between the head and the tail.
    omitted: usize,
    /// The raw text after the last newline.
    pending: String,
    /// The style in force where `pending` starts.
    pen: AgentTextStyle,
}

impl LiveOutput {
    /// Adds `text`; `truncated` says output was skipped before it.
    pub(crate) fn push(&mut self, text: &str, truncated: bool) {
        if truncated {
            // The line before the gap is never finished.
            let pending = std::mem::take(&mut self.pending);
            if !pending.is_empty() {
                let rendered = render_styled_text(pending.as_bytes(), self.pen);
                let mut line = rendered_line(&rendered.text, 0, &rendered.styles);
                cut_line(&mut line);
                self.commit(line);
            }
            self.commit(Line::from(GAP));
            self.pen = AgentTextStyle::default();
        }
        self.pending.push_str(text);
        if let Some(end) = self.pending.rfind('\n') {
            let complete: String = self.pending.drain(..=end).collect();
            let rendered = render_styled_text(complete.as_bytes(), self.pen);
            self.pen = rendered.pen;
            self.commit_lines(&rendered);
        }
        if self.pending.len() > MAX_PENDING_BYTES {
            let complete = complete_prefix_len(self.pending.as_bytes());
            let mut shown = render_styled_text(&self.pending.as_bytes()[..complete], self.pen);
            cut(&mut shown.text, &mut shown.styles);
            // What it shows, written again as terminal text in its styles,
            // then the style in force at its end.
            let mut collapsed = styled_terminal_text(&shown.text, &shown.styles);
            collapsed.push_str(&sgr_sequence(&shown.pen));
            self.pending.replace_range(..complete, &collapsed);
            self.pen = AgentTextStyle::default();
        }
    }

    /// The lines to show, the omitted ones as one line between the first
    /// and the newest, without trailing blank lines; and how many lines of
    /// output they stand for.
    pub(crate) fn lines(&self) -> (Vec<Line<'static>>, usize) {
        let mut lines = self.head.clone();
        if self.omitted > 0 {
            lines.push(Line::from(format!("… +{} lines", self.omitted)));
        }
        lines.extend(self.tail.iter().cloned());
        let complete = complete_prefix_len(self.pending.as_bytes());
        let pending = render_styled_text(&self.pending.as_bytes()[..complete], self.pen);
        let mut pending = rendered_line(&pending.text, 0, &pending.styles);
        cut_line(&mut pending);
        lines.push(pending);
        while lines
            .last()
            .is_some_and(|line| line_text(line).trim().is_empty())
        {
            lines.pop();
        }
        if let Some(last) = lines.last_mut() {
            let trimmed = line_text(last).trim_end().len();
            truncate_line(last, trimmed);
        }
        let count = lines.len() + self.omitted - usize::from(self.omitted > 0);
        (lines, count)
    }

    /// Commits each line of `rendered`, which ends with a newline unless it
    /// is the last of the output before a gap.
    fn commit_lines(&mut self, rendered: &StyledRender) {
        let mut start = 0;
        for text in rendered.text.split_terminator('\n') {
            let mut line = rendered_line(text, start, &rendered.styles);
            cut_line(&mut line);
            self.commit(line);
            start += text.len() + 1;
        }
    }

    fn commit(&mut self, line: Line<'static>) {
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

/// The line `text`, which starts at byte `start` of the text `styles`
/// styles.
fn rendered_line(text: &str, start: usize, styles: &AgentStyledText) -> Line<'static> {
    let end = start + text.len();
    styled_line(
        text,
        styles
            .spans
            .iter()
            .filter(|span| span.start < end && span.end > start)
            .map(|span| {
                (
                    span.start.max(start) - start,
                    span.end.min(end) - start,
                    span.style,
                )
            }),
    )
}

/// Cuts a line as shown to [`MAX_LINE_BYTES`], marking the cut.
fn cut_line(line: &mut Line<'static>) {
    if line_text(line).len() <= MAX_LINE_BYTES {
        return;
    }
    truncate_line(line, MAX_LINE_BYTES);
    if let Some(last) = line.spans.last_mut() {
        last.content.to_mut().push('…');
    }
}

/// Cuts `text` to [`MAX_LINE_BYTES`] on a character boundary, marking the
/// cut, and its styles with it.
fn cut(text: &mut String, styles: &mut AgentStyledText) {
    if text.len() <= MAX_LINE_BYTES {
        return;
    }
    let mut end = MAX_LINE_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    styles.spans.retain_mut(|span| {
        span.end = span.end.min(end);
        span.start < span.end
    });
    text.push('…');
    styles.text_bytes = text.len();
}

#[cfg(test)]
mod tests;
