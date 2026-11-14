// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A closed ```` ```mermaid ```` fence drawn as a text diagram
//! ([`crate::mermaid`]), its nodes and edges in the syntax theme's colours.
//! A diagram that cannot be drawn (unsupported, past a limit, wider than
//! the width) is a dim notice saying why, then the fence's code as any code
//! block shows it. The writer draws only closed fences, and only with
//! `tui.rendering.mermaid` on.

use std::ops::Range;

use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use syntect::highlighting::Theme;

use crate::mermaid::{RenderError, Role};
use crate::render::highlight::{foreground_for_scopes_with_theme, highlight_code_to_lines};

/// The width a diagram is fitted to when the render has none.
const UNBOUNDED_WIDTH: usize = 120;

/// Whether the code block at `range`, whose last text ends at
/// `content_end`, has a closing fence: pulldown-cmark ends a block at the
/// end of the input too.
pub(super) fn has_closing_fence(input: &str, range: Range<usize>, content_end: usize) -> bool {
    let Some(block) = input.get(range.clone()) else {
        return false;
    };
    let Some(marker @ (b'`' | b'~')) = block.as_bytes().first().copied() else {
        return false;
    };
    let opening_len = block.bytes().take_while(|byte| *byte == marker).count();
    let Some(suffix) = input.get(content_end..range.end) else {
        return false;
    };
    suffix
        .trim_end_matches([' ', '\t', '\r', '\n'])
        .bytes()
        .rev()
        .take_while(|byte| *byte == marker)
        .count()
        >= opening_len
}

/// `source`'s diagram fitted to `width`, or the notice and the code.
pub(super) fn render(source: &str, width: Option<usize>, theme: &Theme) -> Vec<Line<'static>> {
    let width = width.unwrap_or(UNBOUNDED_WIDTH);
    let diagram = match crate::mermaid::render_spans(source, width) {
        Ok(diagram) => diagram,
        Err(error) => return fallback(source, width, error),
    };
    let node =
        foreground_for_scopes_with_theme(theme, &["entity.name.type", "support.type", "variable"])
            .map_or_else(
                || Style::default().cyan(),
                |color| Style::default().fg(color),
            );
    let edge = foreground_for_scopes_with_theme(theme, &["comment"]).map_or_else(
        || Style::default().dim(),
        |color| Style::default().fg(color),
    );
    diagram
        .into_iter()
        .map(|row| {
            Line::from(
                row.into_iter()
                    .map(|span| {
                        let style = match span.role {
                            Role::Node => node,
                            Role::Edge => edge,
                            Role::Text => Style::default(),
                        };
                        Span::styled(span.text, style)
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

fn fallback(source: &str, width: usize, error: RenderError) -> Vec<Line<'static>> {
    let notice = match error {
        RenderError::Unsupported => {
            "This Mermaid diagram uses features the terminal renderer doesn't support."
        }
        RenderError::TooWide => "This Mermaid diagram doesn't fit the current terminal width.",
        RenderError::Limit => "This Mermaid diagram exceeds the terminal renderer's size limits.",
    };
    let mut lines = textwrap::wrap(notice, width.max(1))
        .into_iter()
        .map(|line| Line::from(line.into_owned().dim()))
        .collect::<Vec<_>>();
    lines.extend(highlight_code_to_lines(source, "mermaid"));
    lines
}

#[cfg(test)]
mod tests;
