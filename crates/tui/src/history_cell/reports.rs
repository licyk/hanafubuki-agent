// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! What `/mcp` and `/skills` write into the transcript. Codex's `/mcp`
//! lists its configured servers with their tools (`new_mcp_tools_output`);
//! here the servers and their state come from the Agent's MCP snapshot.
//! Codex's `/skills` opens a menu; here it writes the catalog the way
//! `/mcp` writes the servers.

use hanafubuki_agent::{
    McpApplicationSnapshot, McpConnectionPhase, McpTransportConfig, SkillCatalogSnapshot,
};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};

use super::PlainHistoryCell;

pub(crate) fn new_mcp_output(snapshot: &McpApplicationSnapshot) -> PlainHistoryCell {
    let mut lines: Vec<Line<'static>> = vec![
        "/mcp".magenta().into(),
        "".into(),
        vec!["🔌  ".into(), "MCP Servers".bold()].into(),
        "".into(),
    ];

    if snapshot.settings.servers.is_empty() {
        lines.push("  • No MCP servers configured.".italic().into());
        lines.push("".into());
    }

    for server in &snapshot.settings.servers {
        let mut header: Vec<Span<'static>> =
            vec!["  • ".into(), server.id.as_str().to_string().into()];
        if !server.enabled {
            header.push(" ".into());
            header.push("(disabled)".red());
            lines.push(header.into());
            lines.push(Line::from(""));
            continue;
        }
        lines.push(header.into());
        let status = snapshot
            .servers
            .iter()
            .find(|status| status.server_id == server.id);
        let phase = status.map_or(McpConnectionPhase::Disabled, |status| status.phase);
        let phase_span = match phase {
            McpConnectionPhase::Ready => "ready".green(),
            McpConnectionPhase::Failed => "failed".red(),
            McpConnectionPhase::Starting => "starting".into(),
            McpConnectionPhase::Refreshing => "refreshing".into(),
            McpConnectionPhase::Stopping => "stopping".into(),
            McpConnectionPhase::Disabled => "not connected".dim(),
        };
        lines.push(vec!["    • Status: ".into(), phase_span].into());
        if let Some(error) = status.and_then(|status| status.last_error.as_ref()) {
            lines.push(vec!["    • Error: ".into(), error.clone().red()].into());
        }

        match &server.transport {
            McpTransportConfig::Stdio(stdio) => {
                let args_suffix = if stdio.argv.is_empty() {
                    String::new()
                } else {
                    format!(" {}", stdio.argv.join(" "))
                };
                let cmd_display = format!("{}{args_suffix}", stdio.program);
                lines.push(vec!["    • Command: ".into(), cmd_display.into()].into());
            }
            McpTransportConfig::StreamableHttp(http) => {
                lines.push(vec!["    • URL: ".into(), http.url.clone().into()].into());
            }
        }

        let mut names: Vec<String> = snapshot
            .tools
            .iter()
            .filter(|tool| tool.server_id == server.id)
            .map(|tool| tool.remote_name.clone())
            .collect();
        names.sort();
        if names.is_empty() {
            lines.push("    • Tools: (none)".into());
        } else {
            lines.push(vec!["    • Tools: ".into(), names.join(", ").into()].into());
        }
        lines.push(Line::from(""));
    }

    PlainHistoryCell::new(lines)
}

pub(crate) fn new_skills_output(catalog: &SkillCatalogSnapshot) -> PlainHistoryCell {
    let mut lines: Vec<Line<'static>> = vec![
        "/skills".magenta().into(),
        "".into(),
        vec!["📚  ".into(), "Skills".bold()].into(),
        "".into(),
    ];

    if catalog.entries.is_empty() {
        lines.push("  • No Skills installed.".italic().into());
    }
    for entry in &catalog.entries {
        let mut line: Vec<Span<'static>> = vec!["  • ".into(), entry.name.clone().bold()];
        if !entry.enabled {
            line.push(" ".into());
            line.push("(disabled)".red());
        }
        let description = entry
            .short_description
            .as_ref()
            .unwrap_or(&entry.description);
        if !description.is_empty() {
            line.push("  ".into());
            line.push(description.clone().dim());
        }
        lines.push(line.into());
    }
    for issue in &catalog.issues {
        lines.push(
            vec![
                "  ⚠ ".yellow(),
                format!("{}: ", issue.path).into(),
                issue.message.clone().dim(),
            ]
            .into(),
        );
    }

    PlainHistoryCell::new(lines)
}
