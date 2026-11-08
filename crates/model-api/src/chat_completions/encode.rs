use crate::model::{ModelInputItem, ModelRequest, ModelToolChoice};
use crate::port::{ModelError, ModelResult};
use serde_json::{json, Value};

pub(super) fn encode_chat_request(request: &ModelRequest) -> ModelResult<Vec<u8>> {
    if request.model.trim().is_empty() {
        return Err(ModelError::InvalidRequest(
            "model request requires a non-empty model".into(),
        ));
    }
    if request.reasoning.is_some() {
        return Err(ModelError::InvalidRequest(
            "Chat Completions does not support Responses reasoning effort".into(),
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
            "Chat Completions does not support custom or native tool-search history".into(),
        ));
    }
    let mut messages = Vec::new();
    if !request.instructions.is_empty() {
        messages.push(json!({"role": "system", "content": request.instructions}));
    }
    let mut pending_tool_images = Vec::new();
    let mut index = 0;
    while index < request.input.len() {
        if !matches!(request.input[index], ModelInputItem::ToolOutput { .. }) {
            flush_chat_tool_images(&mut messages, &mut pending_tool_images);
        }
        match &request.input[index] {
            ModelInputItem::ContextFragment { role, content, .. } => {
                messages.push(json!({
                    // `developer` is an OpenAI-only role. Chat Completions is
                    // the protocol every compatible server implements, and
                    // those servers accept exactly system/user/assistant/tool
                    // — a `developer` message is rejected outright with
                    // "unknown variant `developer`". OpenAI itself still
                    // accepts `system` and treats it as `developer` for
                    // reasoning models, so the compatible spelling is also the
                    // correct one here. It matches how this encoder already
                    // sends `instructions` above.
                    "role": match role {
                        crate::ModelContextRole::Developer => "system",
                        crate::ModelContextRole::User => "user",
                    },
                    "content": content,
                }));
                index += 1;
            }
            ModelInputItem::UserMessage { content } => {
                let content = content
                    .iter()
                    .map(|part| match part {
                        crate::ModelUserContent::InputText { text } => json!({
                            "type": "text",
                            "text": text
                        }),
                        crate::ModelUserContent::InputImage {
                            mime_type,
                            data,
                            detail,
                        } => json!({
                            "type": "image_url",
                            "image_url": {
                                "url": crate::model::image_data_url(mime_type, data),
                                "detail": crate::model::image_detail_name(*detail)
                            }
                        }),
                    })
                    .collect::<Vec<_>>();
                messages.push(json!({"role": "user", "content": content}));
                index += 1;
            }
            ModelInputItem::AssistantMessage { text, .. } => {
                let mut tool_calls = Vec::new();
                index += 1;
                while let Some(ModelInputItem::ToolCall {
                    call_id,
                    namespace,
                    name,
                    arguments,
                }) = request.input.get(index)
                {
                    tool_calls.push(encode_chat_tool_call(call_id, namespace, name, arguments)?);
                    index += 1;
                }
                if tool_calls.is_empty() {
                    messages.push(json!({"role": "assistant", "content": text}));
                } else {
                    messages.push(json!({
                        "role": "assistant",
                        "content": text,
                        "tool_calls": tool_calls
                    }));
                }
            }
            ModelInputItem::ToolCall { .. } => {
                let mut tool_calls = Vec::new();
                while let Some(ModelInputItem::ToolCall {
                    call_id,
                    namespace,
                    name,
                    arguments,
                }) = request.input.get(index)
                {
                    tool_calls.push(encode_chat_tool_call(call_id, namespace, name, arguments)?);
                    index += 1;
                }
                messages.push(json!({
                    "role": "assistant",
                    "content": null,
                    "tool_calls": tool_calls
                }));
            }
            ModelInputItem::ToolOutput { call_id, output } => {
                let lowered = crate::tool_output_lowering::chat_output(output)?;
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": lowered.tool_content
                }));
                if !lowered.image_parts.is_empty() {
                    pending_tool_images.push(json!({
                        "type": "text",
                        "text": format!("[hanafubuki-agent: image output of tool call {call_id}]")
                    }));
                    pending_tool_images.extend(lowered.image_parts);
                }
                index += 1;
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
                    "content": crate::model::continuation_context_text(
                        provider_id,
                        hook_id,
                        generation,
                        content,
                    )
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
                    "content": crate::model::compaction_summary_text(
                        summary,
                        *source_revision,
                        *compacted_item_count,
                    )
                }));
                index += 1;
            }
            ModelInputItem::CompactionTrigger | ModelInputItem::Compaction { .. } => {
                return Err(ModelError::InvalidRequest(
                    "Chat Completions cannot replay a Responses compaction item".to_string(),
                ));
            }
        }
    }
    flush_chat_tool_images(&mut messages, &mut pending_tool_images);
    let lowered_tools =
        crate::tool_lowering::lower_function_tools(&request.tools, "Chat Completions")?;
    let tools = lowered_tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                    "strict": tool.strict
                }
            })
        })
        .collect::<Vec<_>>();
    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "stream": true,
        "stream_options": {"include_usage": true},
        "store": false
    });
    if let Some(output_format) = &request.output_format {
        body["response_format"] = encode_chat_output_format(output_format)?;
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
        body["parallel_tool_calls"] = Value::Bool(request.parallel_tool_calls);
        body["tool_choice"] = match &request.tool_choice {
            ModelToolChoice::Auto => json!("auto"),
            ModelToolChoice::None => json!("none"),
            ModelToolChoice::Required { name } => {
                if !lowered_tools.iter().any(|tool| tool.name == *name) {
                    return Err(ModelError::InvalidRequest(
                        "required model tool is not present in the request".into(),
                    ));
                }
                json!({"type": "function", "function": {"name": name}})
            }
        };
    } else if matches!(request.tool_choice, ModelToolChoice::Required { .. }) {
        return Err(ModelError::InvalidRequest(
            "required model tool is not present in the request".into(),
        ));
    }
    serde_json::to_vec(&body).map_err(|_| {
        ModelError::InvalidRequest("Chat Completions request could not be encoded".into())
    })
}

fn encode_chat_output_format(output_format: &crate::ModelOutputFormat) -> ModelResult<Value> {
    match output_format {
        crate::ModelOutputFormat::JsonSchema(output) => {
            output.validate().map_err(ModelError::InvalidRequest)?;
            let mut json_schema = json!({
                "name": output.name,
                "schema": output.schema,
                "strict": output.strict,
            });
            if let Some(description) = &output.description {
                json_schema["description"] = Value::String(description.clone());
            }
            Ok(json!({"type": "json_schema", "json_schema": json_schema}))
        }
    }
}

fn flush_chat_tool_images(messages: &mut Vec<Value>, pending_tool_images: &mut Vec<Value>) {
    if pending_tool_images.is_empty() {
        return;
    }
    messages.push(json!({
        "role": "user",
        "content": std::mem::take(pending_tool_images)
    }));
}

fn encode_chat_tool_call(
    call_id: &str,
    namespace: &Option<String>,
    name: &str,
    arguments: &Value,
) -> ModelResult<Value> {
    Ok(json!({
        "id": call_id,
        "type": "function",
        "function": {
            "name": crate::tool_lowering::flattened_tool_call_name(namespace.as_deref(), name),
            "arguments": serde_json::to_string(arguments).map_err(|_| {
                ModelError::InvalidRequest("tool arguments could not be encoded".into())
            })?
        }
    }))
}

pub(super) fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}
