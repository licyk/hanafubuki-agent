// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A call shown by its name: while it runs, when it has no display, and
//! for text or fields (an MCP or plugin tool's, the fallback's), after
//! Codex's MCP cell: `Called` and the tool, then the first lines; expanded,
//! all of them. A `compress` call is folded: its ranges and topics in the
//! header, the first line of its receipt below; expanded, the rest of the
//! receipt and the summaries.

use ratatui::style::Stylize;
use ratatui::text::{Line, Span};

use super::{clipped, wrapped, Extent, ToolCall, View, COMPRESS, EXEC_COMMAND, PREVIEW_LINES};
use crate::history_cell::ActivityDisclosure;

/// `Running` and the command, or the tool and its subject.
pub(super) fn running(call: &ToolCall) -> View {
    let mut title = vec!["Running".bold(), " ".into()];
    if call.name == EXEC_COMMAND && call.subject.is_some() {
        title.push(call.subject_or("").into());
    } else {
        title.extend(named(call));
    }
    View::title(title, false)
}

/// `Called` (or `Failed`) and the tool, with nothing more to show; a
/// command `Ran` and its command line, as it ran.
pub(super) fn called(call: &ToolCall, failed: bool) -> View {
    if call.name == EXEC_COMMAND && call.subject.is_some() {
        return View::title(
            vec!["Ran".bold(), " ".into(), call.subject_or("").into()],
            failed,
        );
    }
    View::title(called_title(call, failed), failed)
}

pub(super) fn view(call: &ToolCall, failed: bool, lines: Vec<String>, extent: Extent) -> View {
    let shown = if call.name == COMPRESS {
        1
    } else {
        PREVIEW_LINES
    };
    let mut preview = Vec::new();
    let mut cut = false;
    for line in lines.iter().take(shown) {
        let (row, was_cut) = clipped(Line::from(line.clone()).dim(), extent.columns);
        cut |= was_cut;
        preview.push(row);
    }
    let mut details = Vec::new();
    if extent.expanded {
        for line in lines.iter().cloned() {
            details.extend(wrapped(Line::from(line).dim(), extent.columns, ""));
        }
    }
    let hidden = match lines.len().checked_sub(shown) {
        Some(more) if more > 0 && call.name == COMPRESS => Some(ActivityDisclosure::Generic),
        Some(more) if more > 0 => Some(ActivityDisclosure::OutputLines(more)),
        _ => cut.then_some(ActivityDisclosure::Generic),
    };
    View {
        failed,
        title: called_title(call, failed),
        preview,
        details,
        hidden,
    }
}

fn called_title(call: &ToolCall, failed: bool) -> Vec<Span<'static>> {
    let mut title = vec![if failed { "Failed" } else { "Called" }.bold(), " ".into()];
    title.extend(named(call));
    title
}

/// `name: subject`.
fn named(call: &ToolCall) -> Vec<Span<'static>> {
    let mut spans = vec![call.name.clone().cyan()];
    if call.subject.is_some() {
        spans.push(": ".dim());
        spans.push(call.subject_or("").into());
    }
    spans
}
