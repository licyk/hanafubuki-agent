//! A file read (`read_file`). Collapsed: `Read`, the path and the lines it
//! read. Expanded: the text behind a right-aligned line-number gutter,
//! highlighted by the file's language.

use ratatui::style::Stylize;
use ratatui::text::{Line, Span};

use super::{counted, expand_tabs, wrapped, Extent, ToolCall, View};
use crate::diff_render::{highlight_lines, line_number_width};
use crate::history_cell::ActivityDisclosure;
use crate::text_formatting::strip_terminal_controls;

pub(super) fn view(
    call: &ToolCall,
    path: &str,
    start_line: Option<u64>,
    text: &str,
    truncated: bool,
    extent: Extent,
) -> View {
    let text = strip_terminal_controls(text);
    let lines: Vec<String> = text.lines().map(expand_tabs).collect();
    let mut amount = match (start_line, lines.len()) {
        (_, 0) => "empty".to_string(),
        (Some(first), 1) => format!("line {first}"),
        (Some(first), count) => format!("lines {first}–{}", first + count as u64 - 1),
        (None, count) => counted(count, "line", "lines"),
    };
    if truncated {
        amount.push_str(", more follows");
    }
    let title = vec![
        "Read".bold(),
        " ".into(),
        call.subject_or(path).into(),
        format!(" ({amount})").dim(),
    ];

    View {
        failed: false,
        title,
        preview: Vec::new(),
        hidden: (!lines.is_empty()).then_some(ActivityDisclosure::OutputLines(lines.len())),
        details: if extent.expanded {
            numbered(path, start_line, &lines, extent.columns)
        } else {
            Vec::new()
        },
    }
}

/// The lines behind a right-aligned gutter of their numbers, when the
/// first one's is known, highlighted by `path`'s language.
fn numbered(
    path: &str,
    start_line: Option<u64>,
    lines: &[String],
    columns: usize,
) -> Vec<Line<'static>> {
    let first = start_line.unwrap_or(1);
    let gutter =
        start_line.map(|first| line_number_width(first + lines.len().saturating_sub(1) as u64));
    let mut highlighted = highlight_lines(path, lines).map(Vec::into_iter);
    let mut details = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let indent = match gutter {
            Some(width) => {
                spans.push(format!("{:>width$} ", first + index as u64).dim());
                " ".repeat(width + 1)
            }
            None => String::new(),
        };
        match highlighted.as_mut().and_then(Iterator::next) {
            Some(code) => spans.extend(code),
            None => spans.push(line.clone().into()),
        }
        details.extend(wrapped(Line::from(spans), columns, indent));
    }
    details
}
