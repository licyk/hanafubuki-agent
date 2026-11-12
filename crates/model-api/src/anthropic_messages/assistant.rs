//! An assistant message: the thinking blocks sent back, its text and the
//! tool calls after it.

use serde_json::{json, Value};

use crate::model::{ModelInputItem, ModelReasoningReplay};

/// The assistant message that starts at `input[*index]`, which is a
/// reasoning block, an assistant message or a tool call, and the index
/// after it. It holds the reasoning blocks before its text or first tool
/// call (when `replay`; otherwise they are left out), then that text, then
/// the tool calls that follow; another reasoning block or text after those
/// starts the next message. Another protocol's reasoning (Chat
/// Completions' `reasoning_content`) is left out wherever it is. `None` when
/// it holds nothing to send: reasoning blocks left out.
pub(super) fn assistant_message(
    input: &[ModelInputItem],
    index: &mut usize,
    replay: bool,
) -> Option<Value> {
    let mut content = Vec::new();
    let mut spoke = false;
    while let Some(item) = input.get(*index) {
        match item {
            ModelInputItem::Reasoning { replay: block } => match thinking_block(block) {
                None => {}
                Some(_) if spoke => break,
                Some(block) if replay => content.push(block),
                Some(_) => {}
            },
            ModelInputItem::AssistantMessage { text, .. } if !spoke => {
                content.push(json!({"type": "text", "text": text}));
                spoke = true;
            }
            ModelInputItem::ToolCall {
                call_id,
                namespace,
                name,
                arguments,
            } => {
                content.push(json!({
                    "type": "tool_use",
                    "id": call_id,
                    "name": crate::tool_lowering::flattened_tool_call_name(namespace.as_deref(), name),
                    "input": arguments
                }));
                spoke = true;
            }
            _ => break,
        }
        *index += 1;
    }
    (!content.is_empty()).then(|| json!({"role": "assistant", "content": content}))
}

/// Anthropic's block of `block`; `None` for another protocol's reasoning.
fn thinking_block(block: &ModelReasoningReplay) -> Option<Value> {
    match block {
        ModelReasoningReplay::Thinking {
            thinking,
            signature,
        } => Some(json!({"type": "thinking", "thinking": thinking, "signature": signature})),
        ModelReasoningReplay::RedactedThinking { data } => {
            Some(json!({"type": "redacted_thinking", "data": data}))
        }
        ModelReasoningReplay::ReasoningContent { .. } => None,
    }
}
