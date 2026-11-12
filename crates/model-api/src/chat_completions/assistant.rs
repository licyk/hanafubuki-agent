//! An assistant message: its text, the tool calls after it, and the
//! `reasoning_content` of the reasoning before them.

use serde_json::{json, Value};

use super::encode::encode_chat_tool_call;
use crate::model::{ModelInputItem, ModelReasoningReplay};
use crate::port::ModelResult;

/// The assistant message that starts at `input[*index]`, which is a
/// reasoning block, an assistant message or a tool call, and the index
/// after it. It holds the text (`null` without one) and the tool calls that
/// follow it, and, as `reasoning_content`, the `reasoning_content` texts of
/// the reasoning before its text or its first tool call, joined as they
/// came, with nothing between them: the text a response not streamed
/// carries whole. Another protocol's reasoning (Anthropic's blocks) is left
/// out. A second text, reasoning after a tool call, or reasoning after the
/// text that no tool call follows, starts the next message. `None` when it
/// holds no text and no call: reasoning that preceded no message in the
/// request is not sent.
pub(super) fn assistant_message(
    input: &[ModelInputItem],
    index: &mut usize,
) -> ModelResult<Option<Value>> {
    let mut reasoning = String::new();
    let mut text = None;
    let mut tool_calls = Vec::new();
    while let Some(item) = input.get(*index) {
        match item {
            ModelInputItem::Reasoning { .. }
                if !tool_calls.is_empty()
                    || (text.is_some() && !precedes_tool_call(&input[*index..])) =>
            {
                break
            }
            ModelInputItem::Reasoning { replay } => {
                if let ModelReasoningReplay::ReasoningContent { text } = replay {
                    reasoning.push_str(text);
                }
            }
            ModelInputItem::AssistantMessage { .. } if text.is_some() || !tool_calls.is_empty() => {
                break
            }
            ModelInputItem::AssistantMessage { text: spoken, .. } => text = Some(spoken),
            ModelInputItem::ToolCall {
                call_id,
                namespace,
                name,
                arguments,
            } => tool_calls.push(encode_chat_tool_call(call_id, namespace, name, arguments)?),
            _ => break,
        }
        *index += 1;
    }
    if text.is_none() && tool_calls.is_empty() {
        return Ok(None);
    }
    let mut message = json!({"role": "assistant", "content": text});
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(tool_calls);
    }
    if !reasoning.is_empty() {
        message["reasoning_content"] = Value::String(reasoning);
    }
    Ok(Some(message))
}

/// Whether the first item of `items` that is not reasoning is a tool call.
fn precedes_tool_call(items: &[ModelInputItem]) -> bool {
    matches!(
        items
            .iter()
            .find(|item| !matches!(item, ModelInputItem::Reasoning { .. })),
        Some(ModelInputItem::ToolCall { .. })
    )
}
