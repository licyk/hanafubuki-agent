// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A run of read, list and search calls in one entry, after Codex's
//! exploration cell: `Exploring` while one runs, then `Explored`, with a
//! row per call (`Read a, b` for consecutive reads). Expanded, each call's
//! own view under its header.

use hanafubuki_agent::AgentToolDisplay;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::{counted, frame, wrapped, Extent, ToolCall, LIST_DIRECTORY, READ_FILE, SEARCH_TEXT};
use crate::render::line_utils::prefix_lines;

pub(super) fn lines(calls: &[ToolCall], width: u16, expanded: bool) -> Vec<Line<'static>> {
    let running = calls.iter().find(|call| call.outcome.is_none());
    let failed = calls.iter().filter(|call| failed(call)).count();
    let mut title = vec![if running.is_some() {
        "Exploring"
    } else {
        "Explored"
    }
    .bold()];
    if failed > 0 {
        title.push(format!(" · {failed} failed").red());
    }
    let bullet = match running {
        Some(call) => call.bullet(false),
        None => calls[0].bullet(failed > 0),
    };
    let extent = Extent::new(width, expanded);
    let body = if expanded {
        calls
            .iter()
            .flat_map(|call| expanded_call(call, extent))
            .collect()
    } else {
        rows(calls, extent.columns)
    };
    frame(bullet, title, None, body, width)
}

fn failed(call: &ToolCall) -> bool {
    call.outcome
        .as_ref()
        .is_some_and(|outcome| outcome.is_error)
}

/// A row per call, consecutive reads that did not fail in one.
fn rows(calls: &[ToolCall], columns: usize) -> Vec<Line<'static>> {
    let mut rows = Vec::new();
    let mut rest = calls;
    while let Some((call, after)) = rest.split_first() {
        let reads = if call.name == READ_FILE && !failed(call) {
            1 + after
                .iter()
                .take_while(|next| next.name == READ_FILE && !failed(next))
                .count()
        } else {
            1
        };
        let (run, after) = rest.split_at(reads);
        rest = after;
        let mut subjects: Vec<String> = Vec::new();
        for call in run {
            let subject = call.subject_or("");
            if !subject.is_empty() && !subjects.contains(&subject) {
                subjects.push(subject);
            }
        }
        let verb = verb(&call.name);
        let mut row = vec![verb.clone().cyan(), " ".into(), subjects.join(", ").into()];
        if reads == 1 {
            row.extend(extent(call));
        }
        rows.extend(wrapped(
            Line::from(row),
            columns,
            " ".repeat(verb.width() + 1),
        ));
    }
    rows
}

fn verb(name: &str) -> String {
    match name {
        READ_FILE => "Read",
        LIST_DIRECTORY => "List",
        SEARCH_TEXT => "Search",
        name => name,
    }
    .to_string()
}

/// How much a call found, or that it failed.
fn extent(call: &ToolCall) -> Option<Span<'static>> {
    let outcome = call.outcome.as_ref()?;
    if outcome.is_error {
        return Some(" (failed)".red());
    }
    let extent = match outcome.display.as_ref()? {
        AgentToolDisplay::Listing { entries, .. } => counted(entries.len(), "entry", "entries"),
        AgentToolDisplay::Search { matches, .. } if matches.is_empty() => "no matches".to_string(),
        AgentToolDisplay::Search { matches, .. } => counted(matches.len(), "match", "matches"),
        _ => return None,
    };
    Some(format!(" ({extent})").dim())
}

/// The call's header, then its details under it.
fn expanded_call(call: &ToolCall, extent: Extent) -> Vec<Line<'static>> {
    let view = call.view(extent.nested());
    let mut lines = wrapped(Line::from(view.title), extent.columns, "  ");
    lines.extend(prefix_lines(view.details, "  ".into(), "  ".into()));
    lines
}
