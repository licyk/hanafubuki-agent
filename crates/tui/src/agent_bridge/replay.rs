//! A session's committed history as the chat shows it: the transcript a
//! reopened session replays, and the output of a tool call that just
//! finished (tool events carry neither its output nor its arguments).

use hanafubuki_agent::{
    AgentHandle, AgentSessionId, AgentSessionMode, ConversationItem, ToolOutputContent,
};

use crate::history_cell::ToolResultView;

/// The tool whose arguments are a patch the transcript shows as a diff.
const APPLY_PATCH: &str = "apply_patch";

/// What the interface shows from now on: a reopened session, or no
/// session yet (at the start, after `/new`), which the next message creates.
#[derive(Debug)]
pub(crate) struct OpenedSession {
    pub(crate) session_id: Option<AgentSessionId>,
    /// The session's model; with none yet, the one it will get.
    pub(crate) model: String,
    pub(crate) mode: AgentSessionMode,
    /// Its latest transcript items, oldest first; empty for a new session.
    pub(crate) transcript: Vec<ReplayedItem>,
    /// Whether older items were left out.
    pub(crate) earlier_items: bool,
}

/// One entry of a reopened session's transcript.
#[derive(Debug)]
pub(crate) enum ReplayedItem {
    User(String),
    /// What the model thought, as Markdown, and for how long.
    Reasoning {
        text: String,
        duration_millis: u64,
        truncated: bool,
    },
    /// An assistant message's markdown.
    Assistant(String),
    /// A proposed plan's markdown.
    Plan(String),
    Tool {
        call_id: String,
        name: String,
        is_error: bool,
        result: ToolResultView,
    },
    Compacted,
}

/// The transcript items a person reads, oldest first, from a page of
/// `items` read latest first. A call's output fills in its call; an output
/// whose call is older than the page is left out.
pub(crate) fn transcript(agent: &AgentHandle, items: Vec<ConversationItem>) -> Vec<ReplayedItem> {
    let mut transcript = Vec::new();
    for content in items.into_iter().rev() {
        let call = call_id(&content).map(str::to_string);
        match content {
            ConversationItem::UserMessage { text, .. }
            | ConversationItem::SteeringMessage { text, .. } => {
                transcript.push(ReplayedItem::User(text))
            }
            ConversationItem::Reasoning {
                text,
                duration_millis,
                truncated,
                ..
            } => transcript.push(ReplayedItem::Reasoning {
                text,
                duration_millis,
                truncated,
            }),
            ConversationItem::AssistantMessage {
                text,
                proposed_plan,
                ..
            } => {
                if !text.trim().is_empty() {
                    transcript.push(ReplayedItem::Assistant(text));
                }
                if let Some(plan) = proposed_plan {
                    transcript.push(ReplayedItem::Plan(plan.text));
                }
            }
            ConversationItem::ToolCall { ref name, .. } => {
                let mut result = ToolResultView::default();
                let name = name.clone();
                read_into(agent, content, &mut result);
                transcript.push(ReplayedItem::Tool {
                    call_id: call.unwrap_or_default(),
                    name,
                    is_error: false,
                    result,
                });
            }
            ConversationItem::ToolOutput { ref output, .. } => {
                let failed = output.is_error;
                let call_entry = transcript.iter_mut().rev().find_map(|entry| match entry {
                    ReplayedItem::Tool {
                        call_id,
                        is_error,
                        result,
                        ..
                    } if Some(call_id.as_str()) == call.as_deref() => Some((is_error, result)),
                    _ => None,
                });
                if let Some((is_error, result)) = call_entry {
                    *is_error = failed;
                    read_into(agent, content, result);
                }
            }
            ConversationItem::CompactionSummary { .. } => transcript.push(ReplayedItem::Compacted),
            _ => {}
        }
    }
    transcript
}

/// The tool call an item belongs to.
pub(super) fn call_id(item: &ConversationItem) -> Option<&str> {
    match item {
        ConversationItem::ToolCall { call_id, .. }
        | ConversationItem::ToolOutput { call_id, .. } => Some(call_id),
        _ => None,
    }
}

/// What a call's item says of its result: an `apply_patch` call's patch,
/// read as a diff, and an output's text and exit code.
pub(super) fn read_into(agent: &AgentHandle, item: ConversationItem, view: &mut ToolResultView) {
    match item {
        ConversationItem::ToolCall {
            name, arguments, ..
        } if name == APPLY_PATCH => {
            let patch = match &arguments {
                serde_json::Value::String(patch) => Some(patch.as_str()),
                value => value.get("patch").and_then(serde_json::Value::as_str),
            };
            view.patch = patch.map(|patch| agent.preview_patch(patch));
        }
        ConversationItem::ToolOutput { output, .. } => {
            for content in output.content {
                match content {
                    ToolOutputContent::Text { text } => view.text = Some(text),
                    ToolOutputContent::Structured { value } => {
                        if let Some(output) = value.get("output").and_then(|v| v.as_str()) {
                            view.text = Some(output.to_string());
                        }
                        view.exit_code = value
                            .get("exitCode")
                            .and_then(serde_json::Value::as_i64)
                            .and_then(|code| i32::try_from(code).ok());
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}
