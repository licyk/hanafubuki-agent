// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/tui/src/app/transcript_export.rs` (the Markdown layout of an
// exported conversation) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: built from the session's committed
// history instead of the interface's cells; reasoning is left out; a tool
// call is summarized as its name and arguments with the first lines of its
// output.

//! A session's whole conversation as Markdown, for a person to keep: its
//! user and assistant messages, proposed plans and tool calls, oldest
//! first. Nothing is committed.

use hanafubuki_agent_kernel::{
    AgentError, AgentHistorySortDirection, AgentSessionId, AgentTranscriptPageRequest,
    ConversationItem, ToolOutputContent, AGENT_TRANSCRIPT_PAGE_SIZE_MAX,
};
use serde_json::Value;

use super::AgentApplicationService;

const HEADING: &str = "# Hanafubuki Agent conversation\n";
/// The most characters of a call's arguments its summary line shows.
const ARGUMENTS_CHARS: usize = 200;
/// The most lines of a call's output the export keeps.
const OUTPUT_LINES: usize = 10;

impl AgentApplicationService {
    /// `session_id`'s conversation as Markdown: a `## User`, `## Assistant`,
    /// `## Plan` or `## Activity` section per message, plan or tool call,
    /// oldest first. A conversation with nothing to show is `InvalidInput`.
    pub fn export_session_markdown(
        &self,
        session_id: AgentSessionId,
    ) -> Result<String, AgentError> {
        self.activate_session(session_id)?;
        let mut items = Vec::new();
        let mut cursor = None;
        let mut through_revision = None;
        loop {
            let page = self.core.read_transcript_page(AgentTranscriptPageRequest {
                session_id,
                page_size: AGENT_TRANSCRIPT_PAGE_SIZE_MAX,
                cursor,
                direction: AgentHistorySortDirection::Ascending,
                through_revision,
            })?;
            items.extend(page.items.into_iter().map(|item| item.content));
            let Some(newer) = page.newer_cursor else {
                break;
            };
            cursor = Some(newer);
            // A commit between two pages must not move the window.
            through_revision = Some(page.through_revision);
        }
        render_markdown(&items)
            .ok_or_else(|| AgentError::InvalidInput("No conversation content to export".into()))
    }
}

/// The Markdown of `items`, oldest first; `None` when none of them is
/// shown.
pub(super) fn render_markdown(items: &[ConversationItem]) -> Option<String> {
    let mut markdown = String::from(HEADING);
    for (index, item) in items.iter().enumerate() {
        match item {
            ConversationItem::UserMessage { text, images }
            | ConversationItem::SteeringMessage { text, images, .. } => {
                let mut body = text.trim_end().to_string();
                if !images.is_empty() && !body.is_empty() {
                    body.push_str("\n\n");
                }
                let labels: Vec<String> = (1..=images.len())
                    .map(|number| format!("[Image #{number}]"))
                    .collect();
                body.push_str(&labels.join("\n"));
                section(&mut markdown, "User", &body, false);
            }
            ConversationItem::AssistantMessage {
                text,
                proposed_plan,
                ..
            } => {
                section(&mut markdown, "Assistant", text.trim_end(), false);
                if let Some(plan) = proposed_plan {
                    section(&mut markdown, "Plan", plan.text.trim_end(), false);
                }
            }
            ConversationItem::ToolCall {
                call_id,
                name,
                arguments,
                ..
            } => {
                let output = items[index + 1..].iter().find_map(|later| match later {
                    ConversationItem::ToolOutput {
                        call_id: output_call,
                        output,
                        ..
                    } if output_call == call_id => Some(output),
                    _ => None,
                });
                let mut body = format!("{name}({})", summarize_arguments(arguments));
                if let Some(output) = output {
                    if output.is_error {
                        body.push_str(" · failed");
                    }
                    body.push_str(&output_excerpt(&output_text(&output.content)));
                }
                section(&mut markdown, "Activity", &body, true);
            }
            ConversationItem::UserShellCommand {
                command,
                exit_code,
                output,
                ..
            } => {
                let mut body = format!("!{}", summarize_arguments(&Value::from(command.as_str())));
                match exit_code {
                    Some(0) => {}
                    Some(code) => body.push_str(&format!(" · exit {code}")),
                    None => body.push_str(" · failed"),
                }
                body.push_str(&output_excerpt(output));
                section(&mut markdown, "Activity", &body, true);
            }
            ConversationItem::CompactionSummary { .. } => {
                section(&mut markdown, "Activity", "Context compacted", true)
            }
            _ => {}
        }
    }
    (markdown != HEADING).then_some(markdown)
}

/// Appends `## heading` and `body`; an activity's lines are indented, so
/// they read as a code block.
fn section(markdown: &mut String, heading: &str, body: &str, indent: bool) {
    if body.trim().is_empty() {
        return;
    }
    markdown.push_str(&format!("\n## {heading}\n\n"));
    for line in body.lines() {
        if indent {
            markdown.push_str("    ");
        }
        markdown.push_str(line);
        markdown.push('\n');
    }
}

/// The arguments on one line, cut at [`ARGUMENTS_CHARS`] characters.
fn summarize_arguments(arguments: &Value) -> String {
    let text = match arguments {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        value => value.to_string(),
    };
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= ARGUMENTS_CHARS {
        return line;
    }
    let cut: String = line.chars().take(ARGUMENTS_CHARS).collect();
    format!("{cut}…")
}

/// A call's text output: its text parts and a structured part's `output`.
fn output_text(content: &[ToolOutputContent]) -> String {
    content
        .iter()
        .filter_map(|content| match content {
            ToolOutputContent::Text { text } => Some(text.as_str()),
            ToolOutputContent::Structured { value } => value.get("output").and_then(Value::as_str),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The first [`OUTPUT_LINES`] lines of `text`, each on a line of its own
/// after the summary, and how many were left out.
fn output_excerpt(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut excerpt = String::new();
    for line in lines.iter().take(OUTPUT_LINES) {
        excerpt.push('\n');
        excerpt.push_str(line);
    }
    if lines.len() > OUTPUT_LINES {
        excerpt.push_str(&format!("\n… {} more lines", lines.len() - OUTPUT_LINES));
    }
    excerpt
}

#[cfg(test)]
mod tests;
