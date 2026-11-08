use hanafubuki_agent_history::AgentMemoryEvidenceRole;
use hanafubuki_agent_memory::AgentMemoryEvidence;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn render_history_evidence(
    evidence: hanafubuki_agent_history::AgentMemoryEvidence,
    maximum_tokens: usize,
) -> AgentMemoryEvidence {
    let mut complete_text = String::new();
    let mut task_ids = std::collections::BTreeSet::new();
    for item in evidence.items {
        if let Some(turn_id) = item.turn_id {
            task_ids.insert(turn_id.value());
        }
        let role = match item.role {
            AgentMemoryEvidenceRole::User => "user",
            AgentMemoryEvidenceRole::Assistant => "assistant",
            AgentMemoryEvidenceRole::Steering => "steering",
            AgentMemoryEvidenceRole::Tool => "tool_output",
        };
        complete_text.push_str(&format!(
            "[{role}; itemId={}; turnId={}; callId={}]\n",
            item.item_id.value(),
            item.turn_id.map_or(0, |turn_id| turn_id.value()),
            item.call_id.as_deref().unwrap_or("none"),
        ));
        if let Some(name) = item.tool_name {
            complete_text.push_str(&format!("toolName: {name}\n"));
        }
        if let Some(arguments) = item.tool_arguments {
            complete_text.push_str("toolArguments: ");
            complete_text.push_str(&redact_tool_arguments(arguments).to_string());
            complete_text.push('\n');
        }
        if !item.text.is_empty() {
            complete_text.push_str(&item.text);
            complete_text.push('\n');
        }
    }
    let (bounded_text, token_truncated) = head_tail_token_truncate(&complete_text, maximum_tokens);
    AgentMemoryEvidence {
        physical_session_id: evidence.session_id.to_string(),
        source_revision: evidence.source_revision.value(),
        updated_at_millis: evidence.updated_at,
        task_ids: task_ids.into_iter().collect(),
        bounded_text,
        truncated: evidence.truncated || token_truncated,
    }
}

fn redact_tool_arguments(mut value: serde_json::Value) -> serde_json::Value {
    const SECRET_KEYS: &[&str] = &[
        "authorization",
        "credential",
        "password",
        "secret",
        "token",
        "api_key",
        "apiKey",
        "private_key",
        "privateKey",
    ];
    match &mut value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                if SECRET_KEYS
                    .iter()
                    .any(|candidate| key.eq_ignore_ascii_case(candidate))
                {
                    *child = serde_json::Value::String("[REDACTED]".to_string());
                } else {
                    *child = redact_tool_arguments(std::mem::take(child));
                }
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                *child = redact_tool_arguments(std::mem::take(child));
            }
        }
        _ => {}
    }
    value
}

fn head_tail_token_truncate(value: &str, maximum_tokens: usize) -> (String, bool) {
    let maximum_bytes = hanafubuki_agent_contracts::token_budget_bytes(maximum_tokens as u64);
    if value.len() <= maximum_bytes {
        return (value.to_string(), false);
    }
    let mut head_end = maximum_bytes.saturating_mul(3) / 5;
    while !value.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let tail_bytes = maximum_bytes.saturating_sub(head_end);
    let mut tail_start = value.len().saturating_sub(tail_bytes);
    while !value.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    let head = &value[..head_end];
    let tail = &value[tail_start..];
    (
        format!("{head}\n--- EVIDENCE MIDDLE TRUNCATED ---\n{tail}"),
        true,
    )
}

pub(super) fn current_millis() -> Result<u64, String> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))?
        .as_millis();
    u64::try_from(millis).map_err(|_| "system millisecond timestamp is out of range".to_string())
}
