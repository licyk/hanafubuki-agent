use super::assistant::assistant_message;
use super::thinking::Thinking;
use crate::model::{ModelInputItem, ModelRequest, ModelToolChoice};
use crate::port::{ModelError, ModelResult};
use serde_json::{json, Value};

pub(super) fn encode_anthropic_request(request: &ModelRequest) -> ModelResult<Vec<u8>> {
    if request.model.trim().is_empty() {
        return Err(ModelError::InvalidRequest(
            "model request requires a non-empty model".into(),
        ));
    }
    if request.output_format.is_some() {
        return Err(ModelError::InvalidRequest(
            "Anthropic Messages route does not support a native structured final output contract"
                .into(),
        ));
    }
    if request.input.iter().any(|item| {
        matches!(
            item,
            ModelInputItem::CustomToolCall { .. }
                | ModelInputItem::CustomToolOutput { .. }
                | ModelInputItem::ToolSearchCall { .. }
                | ModelInputItem::ToolSearchOutput { .. }
        )
    }) {
        return Err(ModelError::InvalidRequest(
            "Anthropic Messages does not support custom or native tool-search history".into(),
        ));
    }
    let thinking = Thinking::of(request);
    let mut messages = Vec::new();
    let mut index = 0;
    while index < request.input.len() {
        match &request.input[index] {
            ModelInputItem::ContextFragment { content, .. } => {
                // Anthropic Messages has no developer message role. Preserve
                // the fixed system prompt and downgrade dynamic fragments to
                // explicitly-labelled user content.
                messages.push(json!({
                    "role": "user",
                    "content": [{
                        "type": "text",
                        "text": content
                    }]
                }));
                index += 1;
            }
            ModelInputItem::UserMessage { content } => {
                use base64::Engine as _;
                let content = content
                    .iter()
                    .map(|part| match part {
                        crate::ModelUserContent::InputText { text } => json!({
                            "type": "text",
                            "text": text
                        }),
                        crate::ModelUserContent::InputImage {
                            mime_type, data, ..
                        } => json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": mime_type,
                                "data": base64::engine::general_purpose::STANDARD.encode(data)
                            }
                        }),
                    })
                    .collect::<Vec<_>>();
                messages.push(json!({
                    "role": "user",
                    "content": content
                }));
                index += 1;
            }
            ModelInputItem::Reasoning { .. }
            | ModelInputItem::AssistantMessage { .. }
            | ModelInputItem::ToolCall { .. } => {
                messages.extend(assistant_message(
                    &request.input,
                    &mut index,
                    thinking.replay,
                ));
            }
            ModelInputItem::ToolOutput { .. } => {
                let mut content = Vec::new();
                while let Some(ModelInputItem::ToolOutput { call_id, output }) =
                    request.input.get(index)
                {
                    content.push(json!({
                        "type": "tool_result",
                        "tool_use_id": call_id,
                        "content": crate::tool_output_lowering::anthropic_content(output)?,
                        "is_error": output.is_error
                    }));
                    index += 1;
                }
                messages.push(json!({"role": "user", "content": content}));
            }
            ModelInputItem::CustomToolCall { .. }
            | ModelInputItem::CustomToolOutput { .. }
            | ModelInputItem::ToolSearchCall { .. }
            | ModelInputItem::ToolSearchOutput { .. } => {
                // Rejected at the top of this function.
                unreachable!("unsupported tool history was rejected before lowering")
            }
            ModelInputItem::ContinuationContext {
                provider_id,
                hook_id,
                generation,
                content,
            } => {
                messages.push(json!({
                    "role": "user",
                    "content": [{
                        "type": "text",
                        "text": crate::model::continuation_context_text(
                            provider_id,
                            hook_id,
                            generation,
                            content,
                        )
                    }]
                }));
                index += 1;
            }
            ModelInputItem::CompactionSummary {
                summary,
                source_revision,
                compacted_item_count,
            } => {
                messages.push(json!({
                    "role": "user",
                    "content": [{
                        "type": "text",
                        "text": crate::model::compaction_summary_text(
                            summary,
                            *source_revision,
                            *compacted_item_count,
                        )
                    }]
                }));
                index += 1;
            }
            ModelInputItem::CompactionTrigger | ModelInputItem::Compaction { .. } => {
                return Err(ModelError::InvalidRequest(
                    "Anthropic Messages cannot replay a Responses compaction item".to_string(),
                ));
            }
        }
    }
    let lowered_tools =
        crate::tool_lowering::lower_function_tools(&request.tools, "Anthropic Messages")?;
    let tools = lowered_tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema
            })
        })
        .collect::<Vec<_>>();
    let mut body = json!({
        "model": request.model,
        "system": request.instructions,
        "messages": messages,
        "max_tokens": thinking.max_tokens,
        "stream": true
    });
    thinking.apply(&mut body);
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
        body["tool_choice"] = match &request.tool_choice {
            ModelToolChoice::Auto => json!({
                "type": "auto",
                "disable_parallel_tool_use": !request.parallel_tool_calls
            }),
            ModelToolChoice::None => json!({"type": "none"}),
            ModelToolChoice::Required { name } => {
                if !lowered_tools.iter().any(|tool| tool.name == *name) {
                    return Err(ModelError::InvalidRequest(
                        "required model tool is not present in the request".into(),
                    ));
                }
                json!({
                    "type": "tool",
                    "name": name,
                    "disable_parallel_tool_use": true
                })
            }
        };
    }
    serde_json::to_vec(&body).map_err(|_| {
        ModelError::InvalidRequest("Anthropic Messages request could not be encoded".into())
    })
}

pub(super) fn required_string(value: &Value, field: &str, label: &str) -> ModelResult<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ModelError::Protocol(format!("Anthropic event has invalid {label}")))
}

pub(super) fn required_u64(value: &Value, field: &str, label: &str) -> ModelResult<u64> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| ModelError::Protocol(format!("Anthropic event has invalid {label}")))
}
