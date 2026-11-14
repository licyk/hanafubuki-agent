//! A command's output in the colours it was printed in: lines whose spans
//! carry the styles the execution service read from its SGR sequences, as
//! Codex's exec cell shows `ansi_escape_line`'s. Only styles reach a line,
//! never an escape sequence, so output cannot move the cursor, set a title
//! or leave a style on past its cell; and its text is always the plain
//! output's, the lines a display without styles shows.
//!
//! The 16 palette colours are the terminal's own (its theme), the 256-colour
//! palette and 24-bit colour pass through.

use hanafubuki_agent::{AgentStyledText, AgentTextColor, AgentTextStyle};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::{expand_tabs_at, text_lines};
use crate::text_formatting::strip_terminal_controls;

/// The lines of `text`, as [`text_lines`] gives them, in the styles of
/// `styles` when they are the styles of `text`.
pub(super) fn output_lines(text: &str, styles: Option<&AgentStyledText>) -> Vec<Line<'static>> {
    let plain = text_lines(text);
    let Some(styles) = styles.filter(|styles| !styles.spans.is_empty() && styles.fits(text)) else {
        return plain.into_iter().map(Line::from).collect();
    };
    let mut lines = Vec::with_capacity(plain.len());
    let mut start = 0;
    for (raw, shown) in text.split('\n').zip(&plain) {
        let end = start + raw.len();
        let spans = styles
            .spans
            .iter()
            .filter(|span| span.start < end && span.end > start)
            .map(|span| {
                (
                    span.start.max(start) - start,
                    span.end.min(end) - start,
                    span.style,
                )
            });
        let mut line = styled_line(raw, spans);
        // The last line lost its trailing blanks with the output's.
        if line_text(&line) != *shown {
            if !line_text(&line).starts_with(shown.as_str()) {
                return plain.into_iter().map(Line::from).collect();
            }
            truncate_line(&mut line, shown.len());
        }
        lines.push(line);
        start = end + 1;
    }
    lines
}

/// One line of output (no `\n`) with its runs (byte ranges of `text` and
/// their styles, in order), its tabs laid out and stray controls dropped. A
/// line without runs is one plain span, as a display without styles has it.
pub(super) fn styled_line(
    text: &str,
    runs: impl IntoIterator<Item = (usize, usize, AgentTextStyle)>,
) -> Line<'static> {
    let mut column = 0;
    let mut spans = Vec::new();
    let mut written = 0;
    let mut push = |segment: &str, style: Option<&AgentTextStyle>, column: &mut usize| {
        let shown = expand_tabs_at(&strip_terminal_controls(segment), column);
        if !shown.is_empty() {
            spans.push(match style {
                Some(style) => Span::styled(shown, ratatui_style(style)),
                None => Span::raw(shown),
            });
        }
    };
    for (start, end, style) in runs {
        let (Some(before), Some(styled)) = (text.get(written..start), text.get(start..end)) else {
            break;
        };
        push(before, None, &mut column);
        push(styled, Some(&style), &mut column);
        written = end;
    }
    if written == 0 {
        let mut column = 0;
        return Line::from(expand_tabs_at(&strip_terminal_controls(text), &mut column));
    }
    push(text.get(written..).unwrap_or_default(), None, &mut column);
    Line::from(spans)
}

/// A run's style as ratatui draws it.
pub(super) fn ratatui_style(style: &AgentTextStyle) -> Style {
    let mut shown = Style::default();
    if let Some(color) = style.fg {
        shown = shown.fg(color_of(color));
    }
    if let Some(color) = style.bg {
        shown = shown.bg(color_of(color));
    }
    for (on, modifier) in [
        (style.bold, Modifier::BOLD),
        (style.dim, Modifier::DIM),
        (style.italic, Modifier::ITALIC),
        (style.underline, Modifier::UNDERLINED),
        (style.reverse, Modifier::REVERSED),
    ] {
        if on {
            shown = shown.add_modifier(modifier);
        }
    }
    shown
}

fn color_of(color: AgentTextColor) -> Color {
    const NAMED: [Color; 16] = [
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
        Color::DarkGray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::White,
    ];
    match color {
        AgentTextColor::Indexed(index) => NAMED
            .get(usize::from(index))
            .copied()
            .unwrap_or(Color::Indexed(index)),
        AgentTextColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// The text of `line`'s spans.
pub(super) fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// `line` cut to its first `bytes` bytes (on a character boundary, at or
/// before it).
pub(super) fn truncate_line(line: &mut Line<'static>, bytes: usize) {
    let mut room = bytes;
    line.spans.retain_mut(|span| {
        if room == 0 {
            return false;
        }
        if span.content.len() > room {
            let mut end = room;
            while !span.content.is_char_boundary(end) {
                end -= 1;
            }
            span.content.to_mut().truncate(end);
            room = 0;
        } else {
            room -= span.content.len();
        }
        !span.content.is_empty()
    });
}

#[cfg(test)]
mod tests;
