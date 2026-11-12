//! A text search (`search_text`). Collapsed: `Searched`, the query and
//! where, and how many matches. Expanded: `path:line: text` per match, the
//! query highlighted where it matched literally (it is a regular
//! expression, which the interface does not evaluate).

use hanafubuki_agent::AgentSearchMatch;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};

use super::{counted, wrapped, Extent, ToolCall, View};
use crate::history_cell::ActivityDisclosure;
use crate::text_formatting::strip_terminal_controls;

pub(super) fn view(
    call: &ToolCall,
    query: &str,
    path: Option<&str>,
    matches: &[AgentSearchMatch],
    truncated: bool,
    extent: Extent,
) -> View {
    let mut amount = match matches.len() {
        0 => "no matches".to_string(),
        count => counted(count, "match", "matches"),
    };
    if truncated {
        amount.push_str(", more not shown");
    }
    let subject = match path {
        Some(path) => format!("\"{query}\" in {path}"),
        None => format!("\"{query}\""),
    };
    let title = vec![
        "Searched".bold(),
        " ".into(),
        call.subject_or(&subject).into(),
        format!(" ({amount})").dim(),
    ];
    let query = strip_terminal_controls(query);
    let mut details = Vec::new();
    for found in matches.iter().filter(|_| extent.expanded) {
        let mut spans =
            vec![format!("{}:{}: ", strip_terminal_controls(&found.path), found.line).dim()];
        spans.extend(highlighted(&strip_terminal_controls(&found.text), &query));
        details.extend(wrapped(Line::from(spans), extent.columns, "  "));
    }
    View {
        failed: false,
        title,
        preview: Vec::new(),
        details,
        hidden: (!matches.is_empty()).then_some(ActivityDisclosure::OutputLines(matches.len())),
    }
}

/// `text` with each literal occurrence of `query` in bold.
fn highlighted(text: &str, query: &str) -> Vec<Span<'static>> {
    if query.is_empty() {
        return vec![text.to_string().into()];
    }
    let mut spans = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(query) {
        if start > 0 {
            spans.push(rest[..start].to_string().into());
        }
        spans.push(query.to_string().bold());
        rest = &rest[start + query.len()..];
    }
    if !rest.is_empty() {
        spans.push(rest.to_string().into());
    }
    spans
}
