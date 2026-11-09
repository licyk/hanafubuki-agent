use super::*;

pub(super) fn encode_responses_request(request: &ModelRequest) -> ModelResult<Vec<u8>> {
    if request.model.trim().is_empty() {
        return Err(ModelError::InvalidRequest(
            "model request requires a non-empty model".into(),
        ));
    }
    let input = request
        .input
        .iter()
        .map(encode_input_item)
        .collect::<ModelResult<Vec<_>>>()?;
    let tools = coalesce_response_namespaces(&request.tools)?
        .iter()
        .map(encode_responses_tool)
        .collect::<ModelResult<Vec<_>>>()?;
    let tool_choice = match &request.tool_choice {
        ModelToolChoice::Auto => json!("auto"),
        ModelToolChoice::None => json!("none"),
        ModelToolChoice::Required { name } => {
            if !request
                .tools
                .iter()
                .any(|tool| responses_tool_supports_required_name(tool, name))
            {
                return Err(ModelError::InvalidRequest(
                    "required model tool is not present in the request".into(),
                ));
            }
            json!({"type": "function", "name": name})
        }
    };
    let mut body = json!({
        "model": request.model,
        "instructions": request.instructions,
        "input": input,
        "tools": tools,
        "tool_choice": tool_choice,
        "parallel_tool_calls": request.parallel_tool_calls,
        "stream": true,
        "store": false
    });
    if let Some(reasoning) = &request.reasoning {
        body["reasoning"] = encode_reasoning(reasoning);
    }
    if let Some(output_format) = &request.output_format {
        body["text"] = json!({"format": encode_responses_output_format(output_format)?});
    }
    serde_json::to_vec(&body)
        .map_err(|_| ModelError::InvalidRequest("model request could not be encoded".into()))
}

pub(super) fn encode_responses_compact_request(request: &ModelRequest) -> ModelResult<Vec<u8>> {
    if request.model.trim().is_empty() {
        return Err(ModelError::InvalidRequest(
            "model request requires a non-empty model".into(),
        ));
    }
    if request.output_format.is_some() {
        return Err(ModelError::InvalidRequest(
            "Responses compact does not support a structured final output contract".into(),
        ));
    }
    let input = request
        .input
        .iter()
        .map(encode_input_item)
        .collect::<ModelResult<Vec<_>>>()?;
    let tools = coalesce_response_namespaces(&request.tools)?
        .iter()
        .map(encode_responses_tool)
        .collect::<ModelResult<Vec<_>>>()?;
    let mut body = json!({
        "model": request.model,
        "instructions": request.instructions,
        "input": input,
        "tools": tools,
        "parallel_tool_calls": request.parallel_tool_calls,
    });
    if let Some(reasoning) = &request.reasoning {
        body["reasoning"] = encode_reasoning(reasoning);
    }
    serde_json::to_vec(&body)
        .map_err(|_| ModelError::InvalidRequest("compact request could not be encoded".into()))
}

/// `summary: "auto"` asks for the model's summary of its reasoning, which
/// OpenAI otherwise does not send.
fn encode_reasoning(reasoning: &crate::ModelReasoningConfig) -> Value {
    json!({"effort": reasoning.effort.as_str(), "summary": "auto"})
}

fn encode_responses_output_format(output_format: &crate::ModelOutputFormat) -> ModelResult<Value> {
    match output_format {
        crate::ModelOutputFormat::JsonSchema(output) => {
            output.validate().map_err(ModelError::InvalidRequest)?;
            let mut value = json!({
                "type": "json_schema",
                "name": output.name,
                "schema": output.schema,
                "strict": output.strict,
            });
            if let Some(description) = &output.description {
                value["description"] = Value::String(description.clone());
            }
            Ok(value)
        }
    }
}

pub(super) fn decode_compact_response(body: &[u8]) -> ModelResult<ModelCompactionResult> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|_| ModelError::Protocol("remote compaction response is not valid JSON".into()))?;
    let output = value
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ModelError::Protocol("remote compaction response is missing output".into())
        })?;
    let mut replacement_history = Vec::new();
    for item in output {
        let Some(item_type) = item.get("type").and_then(Value::as_str) else {
            continue;
        };
        match item_type {
            "message" => {
                let role = item.get("role").and_then(Value::as_str).unwrap_or_default();
                let parts = item
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten();
                let text = parts
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<String>();
                match role {
                    "user" => replacement_history.push(ModelInputItem::UserMessage {
                        content: vec![ModelUserContent::InputText { text }],
                    }),
                    "assistant" => replacement_history.push(ModelInputItem::AssistantMessage {
                        phase: decode_message_phase(item.get("phase").and_then(Value::as_str)),
                        text,
                    }),
                    "developer" | "system" => {
                        replacement_history.push(ModelInputItem::ContextFragment {
                            role: ModelContextRole::Developer,
                            kind: "remote_compaction".to_string(),
                            content: text,
                        });
                    }
                    _ => {}
                }
            }
            "compaction" => {
                let payload = item
                    .get("encrypted_content")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ModelError::Protocol(
                            "remote compaction item is missing encrypted_content".into(),
                        )
                    })?;
                replacement_history.push(ModelInputItem::Compaction {
                    payload: ModelOpaqueCompactionPayload::parse(payload.to_string())
                        .map_err(ModelError::Protocol)?,
                });
            }
            _ => {}
        }
    }
    Ok(ModelCompactionResult {
        replacement_history,
        usage: None,
    })
}

fn encode_responses_tool(tool: &hanafubuki_agent_tool_contracts::ToolSpec) -> ModelResult<Value> {
    use hanafubuki_agent_tool_contracts::ToolSpec;
    Ok(match tool {
        ToolSpec::Function(tool) => {
            let mut value = json!({
                "type": "function",
                "name": tool.name.flattened_alias(),
                "description": tool.description,
                "parameters": tool.input_schema,
                "strict": tool.strict,
            });
            if tool.deferred {
                value["defer_loading"] = Value::Bool(true);
            }
            value
        }
        ToolSpec::Namespace(namespace) => json!({
            "type": "namespace",
            "name": namespace.name.flattened_alias(),
            "description": namespace.description,
            "tools": namespace.tools.iter().map(|tool| {
                let mut value = json!({
                    "type": "function",
                    "name": tool.name.local_name(),
                    "description": tool.description,
                    "parameters": tool.input_schema,
                    "strict": tool.strict,
                });
                if tool.deferred {
                    value["defer_loading"] = Value::Bool(true);
                }
                value
            }).collect::<Vec<_>>(),
        }),
        ToolSpec::ToolSearch(search) => json!({
            "type": "tool_search",
            "execution": search.execution,
            "description": search.description,
            "parameters": search.parameters,
        }),
        ToolSpec::HostedWebSearch(search) => {
            let mut value = json!({"type": "web_search"});
            if let Some(enabled) = search.external_web_access {
                value["external_web_access"] = Value::Bool(enabled);
            }
            if let Some(enabled) = search.indexed_web_access {
                value["indexed_web_access"] = Value::Bool(enabled);
            }
            if let Some(domains) = &search.allowed_domains {
                value["filters"] = json!({"allowed_domains": domains});
            }
            if let Some(size) = &search.search_context_size {
                value["search_context_size"] = Value::String(size.clone());
            }
            value
        }
        ToolSpec::Freeform(tool) => json!({
            "type": "custom",
            "name": tool.name.flattened_alias(),
            "description": tool.description,
            "format": {
                "type": tool.format.format_type,
                "syntax": tool.format.syntax,
                "definition": tool.format.definition,
            },
        }),
    })
}

fn coalesce_response_namespaces(
    tools: &[hanafubuki_agent_tool_contracts::ToolSpec],
) -> ModelResult<Vec<hanafubuki_agent_tool_contracts::ToolSpec>> {
    use hanafubuki_agent_tool_contracts::ToolSpec;
    let mut output = Vec::<ToolSpec>::new();
    for tool in tools {
        if let ToolSpec::Namespace(namespace) = tool {
            if let Some(ToolSpec::Namespace(existing)) = output.iter_mut().find(|candidate| {
                matches!(candidate, ToolSpec::Namespace(value) if value.name == namespace.name)
            }) {
                existing.tools.extend(namespace.tools.clone());
                continue;
            }
        }
        output.push(tool.clone());
    }
    for tool in &output {
        tool.validate()
            .map_err(|error| ModelError::InvalidRequest(error.to_string()))?;
    }
    Ok(output)
}

fn responses_tool_supports_required_name(
    tool: &hanafubuki_agent_tool_contracts::ToolSpec,
    required: &str,
) -> bool {
    use hanafubuki_agent_tool_contracts::ToolSpec;
    match tool {
        ToolSpec::Function(tool) => tool.name.flattened_alias() == required,
        ToolSpec::Freeform(tool) => tool.name.flattened_alias() == required,
        ToolSpec::Namespace(_) | ToolSpec::ToolSearch(_) | ToolSpec::HostedWebSearch(_) => false,
    }
}

fn encode_input_item(item: &ModelInputItem) -> ModelResult<Value> {
    Ok(match item {
        ModelInputItem::ContextFragment { role, content, .. } => json!({
            "role": match role {
                crate::ModelContextRole::Developer => "developer",
                crate::ModelContextRole::User => "user",
            },
            "content": [{
                "type": "input_text",
                "text": content
            }]
        }),
        ModelInputItem::UserMessage { content } => json!({
            "role": "user",
            "content": content.iter().map(|part| match part {
                crate::ModelUserContent::InputText { text } => json!({
                    "type": "input_text",
                    "text": text
                }),
                crate::ModelUserContent::InputImage { mime_type, data, detail } => json!({
                    "type": "input_image",
                    "image_url": crate::model::image_data_url(mime_type, data),
                    "detail": crate::model::image_detail_name(*detail)
                }),
            }).collect::<Vec<_>>()
        }),
        ModelInputItem::AssistantMessage { phase, text } => json!({
            "role": "assistant",
            "phase": phase.map(encode_message_phase),
            "content": [{ "type": "output_text", "text": text }]
        }),
        ModelInputItem::ToolCall {
            call_id,
            namespace,
            name,
            arguments,
        } => {
            let mut call = json!({
                "type": "function_call",
                "call_id": call_id,
                "name": name,
                "arguments": serde_json::to_string(arguments).map_err(|_| {
                    ModelError::InvalidRequest("tool arguments could not be encoded".into())
                })?
            });
            if let Some(namespace) = namespace {
                call["namespace"] = Value::String(namespace.clone());
            }
            call
        }
        ModelInputItem::CustomToolCall {
            call_id,
            name,
            input,
        } => json!({
            "type": "custom_tool_call",
            "call_id": call_id,
            "name": name,
            "input": input,
        }),
        ModelInputItem::ToolOutput { call_id, output } => json!({
            "type": "function_call_output",
            "call_id": call_id,
            "output": crate::tool_output_lowering::responses_output(output)?
        }),
        ModelInputItem::CustomToolOutput { call_id, output } => json!({
            "type": "custom_tool_call_output",
            "call_id": call_id,
            "output": crate::tool_output_lowering::responses_output(output)?
        }),
        ModelInputItem::ToolSearchCall { call_id, arguments } => json!({
            "type": "tool_search_call",
            "call_id": call_id,
            "execution": "client",
            "arguments": arguments,
        }),
        ModelInputItem::ToolSearchOutput { call_id, tools } => json!({
            "type": "tool_search_output",
            "call_id": call_id,
            "status": "completed",
            "execution": "client",
            "tools": tools.iter().map(encode_responses_tool).collect::<ModelResult<Vec<_>>>()?,
        }),
        ModelInputItem::ContinuationContext {
            provider_id,
            hook_id,
            generation,
            content,
        } => json!({
            "role": "user",
            "content": [{
                "type": "input_text",
                "text": crate::model::continuation_context_text(
                    provider_id,
                    hook_id,
                    generation,
                    content,
                )
            }]
        }),
        ModelInputItem::CompactionSummary {
            summary,
            source_revision,
            compacted_item_count,
        } => json!({
            "role": "user",
            "content": [{
                "type": "input_text",
                "text": crate::model::compaction_summary_text(
                    summary,
                    *source_revision,
                    *compacted_item_count,
                )
            }]
        }),
        ModelInputItem::CompactionTrigger => json!({
            "type": "compaction_trigger"
        }),
        ModelInputItem::Compaction { payload } => json!({
            "type": "compaction",
            "encrypted_content": payload.as_str(),
        }),
    })
}

fn encode_message_phase(phase: ModelMessagePhase) -> &'static str {
    match phase {
        ModelMessagePhase::Commentary => "commentary",
        ModelMessagePhase::FinalAnswer => "final_answer",
    }
}
