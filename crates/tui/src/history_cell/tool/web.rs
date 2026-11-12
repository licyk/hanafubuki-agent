//! The web tools. A search: `Searched the web for` and the query; expanded,
//! each result's title and address. A page: `Fetched` and its address;
//! expanded, its title and first lines.

use hanafubuki_agent::AgentWebResult;
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::{text_lines, wrapped, Extent, View};
use crate::history_cell::ActivityDisclosure;
use crate::text_formatting::strip_terminal_controls;

/// The lines of a page an expanded entry shows.
const PAGE_LINES: usize = 40;

pub(super) fn search_view(query: &str, results: &[AgentWebResult], extent: Extent) -> View {
    let title = vec![
        "Searched the web for".bold(),
        " ".into(),
        strip_terminal_controls(query).into(),
    ];
    let mut details = Vec::new();
    for result in results.iter().filter(|_| extent.expanded) {
        details.extend(wrapped(
            Line::from(strip_terminal_controls(&result.title).bold()),
            extent.columns,
            "",
        ));
        details.extend(wrapped(
            Line::from(format!("  {}", strip_terminal_controls(&result.url)).dim()),
            extent.columns,
            "  ",
        ));
    }
    View {
        failed: false,
        title,
        preview: Vec::new(),
        details,
        hidden: (!results.is_empty()).then_some(ActivityDisclosure::Generic),
    }
}

pub(super) fn page_view(url: &str, page_title: Option<&str>, text: &str, extent: Extent) -> View {
    let title = vec![
        "Fetched".bold(),
        " ".into(),
        strip_terminal_controls(url).into(),
    ];
    let page_title = page_title.filter(|title| !title.trim().is_empty());
    let lines = text_lines(text);
    let mut details = Vec::new();
    if extent.expanded {
        if let Some(page_title) = page_title {
            details.extend(wrapped(
                Line::from(strip_terminal_controls(page_title).bold()),
                extent.columns,
                "",
            ));
        }
        for line in lines.iter().take(PAGE_LINES).cloned() {
            details.extend(wrapped(Line::from(line).dim(), extent.columns, ""));
        }
        if lines.len() > PAGE_LINES {
            details.push(Line::from(
                format!("… +{} lines", lines.len() - PAGE_LINES).dim(),
            ));
        }
    }
    View {
        failed: false,
        title,
        preview: Vec::new(),
        details,
        hidden: (page_title.is_some() || !lines.is_empty()).then_some(ActivityDisclosure::Generic),
    }
}
