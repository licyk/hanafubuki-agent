// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Consecutive calls of one kind in a compact entry, after Codex's
//! exploration cell. Expanded, each call keeps its own view.

use hanafubuki_agent::AgentToolDisplay;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::{counted, frame, wrapped, Extent, ToolCall, LIST_DIRECTORY, READ_FILE, SEARCH_TEXT};
use crate::render::line_utils::prefix_lines;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolGroup {
    Exploration,
    Images,
    Skills,
    Mcp,
    Discovery,
    Web,
    Goals,
    Questions,
    Plan,
}

impl ToolGroup {
    pub(crate) fn for_name(name: &str) -> Option<Self> {
        match name {
            READ_FILE | LIST_DIRECTORY | SEARCH_TEXT => Some(Self::Exploration),
            "view_image" => Some(Self::Images),
            "skills_list" | "skills_read" | "enable_skill" | "disable_skill" => Some(Self::Skills),
            "list_mcp_resources" | "list_mcp_resource_templates" | "read_mcp_resource" => {
                Some(Self::Mcp)
            }
            name if name.starts_with("mcp__") => Some(Self::Mcp),
            // Native Responses search keeps its private name in replayed history.
            "tool_search" | "tool_list" | "__hanafubuki_native_tool_search" => {
                Some(Self::Discovery)
            }
            "web_search" | "advanced_search" | "multi_search" | "fetch_web_page"
            | "platform_search" | "video_search" | "domain_search" | "weather" | "research"
            | "cache_search" | "free_search_test" => Some(Self::Web),
            "create_goal" | "get_goal" | "update_goal" => Some(Self::Goals),
            "request_user_input" => Some(Self::Questions),
            "update_plan" => Some(Self::Plan),
            _ => None,
        }
    }

    fn title(self, running: bool, count: usize) -> &'static str {
        match (self, running) {
            (Self::Exploration, true) => "Exploring",
            (Self::Exploration, false) => "Explored",
            (Self::Images, true) if count == 1 => "Viewing image",
            (Self::Images, false) if count == 1 => "Viewed image",
            (Self::Images, true) => "Viewing images",
            (Self::Images, false) => "Viewed images",
            (Self::Skills, true) => "Using skills",
            (Self::Skills, false) => "Used skills",
            (Self::Mcp, true) => "Calling MCP",
            (Self::Mcp, false) => "Called MCP",
            (Self::Discovery, true) => "Discovering tools",
            (Self::Discovery, false) => "Discovered tools",
            (Self::Web, true) => "Searching the web",
            (Self::Web, false) => "Searched the web",
            (Self::Goals, true) => "Managing goal",
            (Self::Goals, false) => "Managed goal",
            (Self::Questions, true) => "Asking user",
            (Self::Questions, false) => "Asked user",
            (Self::Plan, true) => "Updating plan",
            (Self::Plan, false) => "Updated plan",
        }
    }
}

pub(super) fn lines(
    group: ToolGroup,
    calls: &[ToolCall],
    width: u16,
    extent: Extent,
) -> Vec<Line<'static>> {
    let running = calls.iter().find(|call| call.outcome.is_none());
    let failed = calls.iter().filter(|call| failed(call)).count();
    let mut title = vec![group.title(running.is_some(), calls.len()).bold()];
    if failed > 0 {
        title.push(format!(" · {failed} failed").red());
    }
    let bullet = match running {
        Some(call) => call.bullet(false),
        None => calls[0].bullet(failed > 0),
    };
    let body = if extent.expanded {
        calls
            .iter()
            .flat_map(|call| expanded_call(call, extent))
            .collect()
    } else {
        rows(group, calls, extent)
    };
    frame(bullet, title, None, body, width)
}

fn failed(call: &ToolCall) -> bool {
    call.outcome
        .as_ref()
        .is_some_and(|outcome| outcome.is_error)
}

/// A row per call, consecutive reads that did not fail in one.
fn rows(group: ToolGroup, calls: &[ToolCall], view_extent: Extent) -> Vec<Line<'static>> {
    let columns = view_extent.columns;
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
        let mut row = vec![verb.clone().cyan()];
        if !subjects.is_empty() {
            row.push(if group == ToolGroup::Mcp { ": " } else { " " }.into());
            row.push(subjects.join(", ").into());
        }
        if reads == 1 {
            row.extend(extent(call));
            if group != ToolGroup::Exploration && call.outcome.is_none() {
                row.push(" (running)".dim());
            }
        }
        rows.extend(wrapped(
            Line::from(row),
            columns,
            " ".repeat(if group == ToolGroup::Exploration {
                verb.width() + 1
            } else {
                2
            }),
        ));
        if group != ToolGroup::Exploration && failed(call) {
            rows.extend(prefix_lines(
                call.view(view_extent.nested()).preview,
                "  ".into(),
                "  ".into(),
            ));
        }
    }
    rows
}

fn verb(name: &str) -> String {
    match name {
        READ_FILE => "Read",
        LIST_DIRECTORY => "List",
        SEARCH_TEXT => "Search",
        "view_image" => "View",
        "skills_list" => "List skills",
        "skills_read" => "Read skill",
        "enable_skill" => "Enable skill",
        "disable_skill" => "Disable skill",
        "list_mcp_resources" => "List resources",
        "list_mcp_resource_templates" => "List resource templates",
        "read_mcp_resource" => "Read resource",
        "tool_search" | "__hanafubuki_native_tool_search" => "Search tools",
        "tool_list" => "List tools",
        "web_search" => "Search",
        "advanced_search" => "Advanced search",
        "multi_search" => "Multi-engine search",
        "fetch_web_page" => "Fetch page",
        "platform_search" => "Search platform",
        "video_search" => "Search videos",
        "domain_search" => "Search domain",
        "weather" => "Check weather",
        "research" => "Research",
        "cache_search" => "Search cache",
        "free_search_test" => "Test search providers",
        "create_goal" => "Create goal",
        "get_goal" => "Read goal",
        "update_goal" => "Update goal",
        "request_user_input" => "Ask",
        "update_plan" => "Update plan",
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
        AgentToolDisplay::WebSearch { results, .. } if results.is_empty() => {
            "no results".to_string()
        }
        AgentToolDisplay::WebSearch { results, .. } => counted(results.len(), "result", "results"),
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
