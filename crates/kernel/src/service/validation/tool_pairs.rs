use crate::model::{AgentError, ConversationItem};
use serde_json::json;
use std::collections::{BTreeSet, HashMap, HashSet};

pub(in crate::service) fn validate_complete_tool_pairs(
    history: &[ConversationItem],
) -> Result<(), AgentError> {
    let mut calls = HashMap::new();
    let mut outputs = HashSet::new();
    for (index, item) in history.iter().enumerate() {
        match item {
            ConversationItem::ToolCall { call_id, .. } => {
                if calls.insert(call_id.as_str(), index).is_some() {
                    return Err(AgentError::Protocol(
                        "fork source contains duplicate tool call ids".to_string(),
                    ));
                }
            }
            ConversationItem::ToolOutput { call_id, .. } => {
                let Some(call_index) = calls.get(call_id.as_str()) else {
                    return Err(AgentError::Protocol(
                        "fork source contains an unmatched tool output".to_string(),
                    ));
                };
                if *call_index >= index || !outputs.insert(call_id.as_str()) {
                    return Err(AgentError::Protocol(
                        "fork source contains an invalid tool call/output pair".to_string(),
                    ));
                }
            }
            _ => {}
        }
    }
    if calls.len() != outputs.len() {
        return Err(AgentError::Protocol(
            "fork source contains an incomplete tool call/output pair".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn validate_skill_context_groups(
    history: &[ConversationItem],
) -> Result<(), AgentError> {
    const MAXIMUM_SELECTED_SKILLS_PER_TURN: usize = 8;
    let mut selected = BTreeSet::new();
    for item in history {
        match item {
            ConversationItem::TurnStart { .. } => selected.clear(),
            ConversationItem::SkillContext {
                skill,
                name,
                content,
            } if selected.len() >= MAXIMUM_SELECTED_SKILLS_PER_TURN
                || !selected.insert(skill)
                || name.is_empty()
                || name.len() > 256
                || content.is_empty()
                || content.len() > 32 * 1024
                || !content.starts_with("<skill>\n")
                || !content.ends_with("\n</skill>") =>
            {
                return Err(AgentError::Persistence(
                    "Agent history contains an invalid Skill context group".to_string(),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// A Skill context inside a turn is a tool's context fragment, which the
/// kernel commits right after that tool's output.
pub(super) fn skill_context_follows_tool_output(
    history: &[&ConversationItem],
    index: usize,
) -> bool {
    let Some(&ConversationItem::ToolOutput { call_id, .. }) = index
        .checked_sub(1)
        .and_then(|previous| history.get(previous))
    else {
        return false;
    };
    history[..index].iter().any(|item| {
        matches!(
            *item,
            ConversationItem::ToolCall {
                call_id: candidate,
                ..
            } if candidate == call_id
        )
    })
}

pub(in crate::service) fn interrupted_tool_outputs(
    history: &[ConversationItem],
) -> Result<Vec<ConversationItem>, AgentError> {
    let mut calls = Vec::new();
    let mut pending = HashMap::new();
    for item in history {
        match item {
            ConversationItem::ToolCall {
                call_id, origin, ..
            } => {
                let call_index = calls.len();
                if pending
                    .insert(call_id.clone(), (*origin, call_index))
                    .is_some()
                {
                    return Err(AgentError::Persistence(format!(
                        "duplicate unpaired Agent tool call id {call_id}"
                    )));
                }
                calls.push(Some((call_id.clone(), *origin)));
            }
            ConversationItem::ToolOutput {
                call_id, origin, ..
            } => {
                let Some((call_origin, call_index)) = pending.remove(call_id) else {
                    return Err(AgentError::Persistence(format!(
                        "committed Agent tool output {call_id} has no pending call"
                    )));
                };
                if call_origin != *origin {
                    return Err(AgentError::Persistence(format!(
                        "committed Agent tool output {call_id} changed call origin"
                    )));
                }
                calls[call_index] = None;
            }
            ConversationItem::TurnStart { .. }
            | ConversationItem::CanonicalInitialContext { .. }
            | ConversationItem::WorldStateUpdate { .. }
            | ConversationItem::SkillContext { .. }
            | ConversationItem::UserMessage { .. }
            | ConversationItem::AssistantMessage { .. }
            | ConversationItem::Reasoning { .. }
            | ConversationItem::SteeringMessage { .. }
            | ConversationItem::ToolSelection { .. }
            | ConversationItem::PermissionReview { .. }
            | ConversationItem::ContinuationContext { .. }
            | ConversationItem::CompactionSummary { .. }
            | ConversationItem::ProviderCompaction { .. }
            | ConversationItem::UserShellCommand { .. }
            | ConversationItem::WakeEvents { .. } => {}
        }
    }
    Ok(calls
        .into_iter()
        .flatten()
        .map(|(call_id, origin)| interrupted_tool_output(call_id, origin))
        .collect())
}

pub(in crate::service) fn normalized_tool_pairs_for_model(
    history: &[ConversationItem],
) -> Result<Vec<ConversationItem>, AgentError> {
    let mut normalized = history
        .iter()
        .filter(|item| !is_interrupted_tool_output(item))
        .cloned()
        .collect::<Vec<_>>();
    let mut pending = HashMap::new();
    for (index, item) in normalized.iter().enumerate() {
        match item {
            ConversationItem::ToolCall {
                call_id, origin, ..
            } => {
                if pending.insert(call_id.clone(), (*origin, index)).is_some() {
                    return Err(AgentError::Persistence(format!(
                        "duplicate unpaired Agent tool call id {call_id}"
                    )));
                }
            }
            ConversationItem::ToolOutput {
                call_id, origin, ..
            } => {
                let Some((call_origin, _)) = pending.remove(call_id) else {
                    return Err(AgentError::Persistence(format!(
                        "committed Agent tool output {call_id} has no pending call"
                    )));
                };
                if call_origin != *origin {
                    return Err(AgentError::Persistence(format!(
                        "committed Agent tool output {call_id} changed call origin"
                    )));
                }
            }
            _ => {}
        }
    }
    let mut missing = pending
        .into_iter()
        .map(|(call_id, (origin, index))| (index, interrupted_tool_output(call_id, origin)))
        .collect::<Vec<_>>();
    missing.sort_by_key(|(index, _)| std::cmp::Reverse(*index));
    for (index, output) in missing {
        normalized.insert(index.saturating_add(1), output);
    }
    Ok(normalized)
}

pub(super) fn interrupted_tool_output(
    call_id: String,
    origin: hanafubuki_agent_model_api::ModelToolCallOrigin,
) -> ConversationItem {
    ConversationItem::ToolOutput {
        call_id,
        origin,
        output: hanafubuki_agent_tool_contracts::ToolOutput::structured(
            json!({
                "status": "interrupted",
                "replayed": false,
                "error": "tool execution ended before the result was durably committed"
            }),
            true,
        ),
        truncation: None,
        images: Vec::new(),
        tool_search_results: Vec::new(),
        display_metadata: Default::default(),
    }
}

pub(super) fn is_interrupted_tool_output(item: &ConversationItem) -> bool {
    let ConversationItem::ToolOutput { output, .. } = item else {
        return false;
    };
    output.is_error
        && output.structured_content.as_ref().is_some_and(|value| {
            value.get("status").and_then(serde_json::Value::as_str) == Some("interrupted")
                && value.get("replayed").and_then(serde_json::Value::as_bool) == Some(false)
                && value.get("error").and_then(serde_json::Value::as_str)
                    == Some("tool execution ended before the result was durably committed")
        })
}
