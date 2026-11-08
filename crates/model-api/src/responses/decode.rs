use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMPORARY_TOOL_ITEM_ID: AtomicU64 = AtomicU64::new(1);

const PROTOCOL: &str = "responses";

pub(super) fn decode_sse_event(
    provider: &ModelProvider,
    data: &[u8],
    metadata: &ModelResponseMetadata,
) -> ModelResult<Option<ModelEvent>> {
    if data == b"[DONE]" {
        return Ok(None);
    }
    let value: Value = serde_json::from_slice(data)
        .map_err(|_| ModelError::Protocol("model SSE event is not valid JSON".into()))?;
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| ModelError::Protocol("model SSE event does not contain a type".into()))?;
    match event_type {
        "response.created" => Ok(Some(ModelEvent::ResponseStarted {
            response_id: required_string(&value, &["response", "id"], "response id")?,
            metadata: metadata.clone(),
        })),
        "response.output_item.added" => decode_output_item_added(&value).map(Some),
        "response.output_text.delta" => Ok(Some(ModelEvent::AssistantTextDelta {
            item_id: required_string(&value, &["item_id"], "output item id")?,
            delta: required_string(&value, &["delta"], "output text delta")?,
        })),
        // Completion authority is response.output_item.done. Some compatible
        // endpoints also emit output_text.done; accepting both would duplicate
        // the assistant item.
        "response.output_text.done" => Ok(None),
        "response.reasoning_text.delta" => {
            let item_id = value
                .pointer("/item_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let delta = value
                .pointer("/delta")
                .and_then(Value::as_str)
                .unwrap_or_default();
            crate::streaming::trace_reasoning(PROTOCOL, "delta", item_id, delta);
            Ok(None)
        }
        "response.reasoning_summary_text.delta" => {
            let item_id = required_string(&value, &["item_id"], "reasoning item id")?;
            let delta = required_string(&value, &["delta"], "reasoning summary delta")?;
            // A summary delta without its part index belongs to the first
            // part; a server that never splits the summary omits it.
            let summary_index = value
                .get("summary_index")
                .and_then(Value::as_u64)
                .and_then(|index| u32::try_from(index).ok())
                .unwrap_or(0);
            crate::streaming::trace_reasoning(PROTOCOL, "summary_delta", &item_id, &delta);
            Ok(Some(ModelEvent::ReasoningSummaryDelta {
                item_id,
                summary_index,
                delta,
            }))
        }
        "response.output_item.done" => decode_output_item_done(&value).map(Some),
        "response.completed" => Ok(Some(ModelEvent::ResponseCompleted {
            response_id: required_string(&value, &["response", "id"], "response id")?,
            usage: response_usage(&value),
            end_turn: value.pointer("/response/end_turn").and_then(Value::as_bool),
        })),
        "response.incomplete" => decode_response_incomplete(provider, &value).map(Some),
        "response.failed" | "error" => Err(response_failure(provider, &value, metadata)),
        _ => Ok(None),
    }
}

fn decode_output_item_added(value: &Value) -> ModelResult<ModelEvent> {
    let item_type = value.pointer("/item/type").and_then(Value::as_str);
    let item = match item_type {
        Some("message") => ModelOutputItem::AssistantMessage {
            item_id: required_string(value, &["item", "id"], "output item id")?,
            phase: decode_message_phase(value.pointer("/item/phase").and_then(Value::as_str)),
            text: String::new(),
        },
        Some("function_call") => ModelOutputItem::ToolCall {
            call: ModelToolCall {
                item_id: response_tool_item_id(value),
                call_id: optional_string(value, "/item/call_id"),
                namespace: response_tool_call_namespace(value),
                name: optional_string(value, "/item/name"),
                arguments: Value::Null,
                argument_error: None,
            },
        },
        Some("custom_tool_call") => ModelOutputItem::ToolCall {
            call: ModelToolCall {
                item_id: response_tool_item_id(value),
                call_id: optional_string(value, "/item/call_id"),
                namespace: response_tool_call_namespace(value),
                name: optional_string(value, "/item/name"),
                arguments: Value::String(String::new()),
                argument_error: None,
            },
        },
        Some("tool_search_call") => ModelOutputItem::ToolCall {
            call: ModelToolCall {
                item_id: response_tool_item_id(value),
                call_id: optional_string(value, "/item/call_id"),
                namespace: None,
                name: crate::NATIVE_TOOL_SEARCH_NAME.to_string(),
                arguments: value
                    .pointer("/item/arguments")
                    .cloned()
                    .unwrap_or(Value::Null),
                argument_error: None,
            },
        },
        Some("reasoning") => {
            let item_id = required_string(value, &["item", "id"], "reasoning item id")?;
            let summary = reasoning_summary(value);
            crate::streaming::trace_reasoning(
                PROTOCOL,
                "item_started",
                &item_id,
                &summary.join("\n"),
            );
            ModelOutputItem::Reasoning { item_id, summary }
        }
        Some("compaction") => ModelOutputItem::Other {
            item_id: Some(required_string(
                value,
                &["item", "id"],
                "compaction item id",
            )?),
            item_type: "compaction_pending".to_string(),
        },
        _ => other_output_item(value),
    };
    Ok(ModelEvent::OutputItemAdded { item })
}

/// A completed output item. Each `done` event carries the whole item, so it
/// is decoded on its own, with or without an earlier `added`.
fn decode_output_item_done(value: &Value) -> ModelResult<ModelEvent> {
    let item = match value.pointer("/item/type").and_then(Value::as_str) {
        Some("message") => {
            let text = value
                .pointer("/item/content")
                .and_then(Value::as_array)
                .map(|content| {
                    content
                        .iter()
                        .filter(|part| {
                            part.get("type").and_then(Value::as_str) == Some("output_text")
                        })
                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                        .collect::<String>()
                })
                .unwrap_or_default();
            ModelOutputItem::AssistantMessage {
                item_id: required_string(value, &["item", "id"], "output item id")?,
                phase: decode_message_phase(value.pointer("/item/phase").and_then(Value::as_str)),
                text,
            }
        }
        Some("function_call") => completed_function_call(value)?,
        Some("custom_tool_call") => ModelOutputItem::ToolCall {
            call: ModelToolCall {
                item_id: response_tool_item_id(value),
                call_id: required_string(value, &["item", "call_id"], "tool call id")?,
                namespace: response_tool_call_namespace(value),
                name: response_tool_call_name(value)?,
                arguments: Value::String(required_string(
                    value,
                    &["item", "input"],
                    "custom tool input",
                )?),
                argument_error: None,
            },
        },
        Some("tool_search_call") => {
            let arguments = value
                .pointer("/item/arguments")
                .cloned()
                .ok_or_else(|| ModelError::Protocol("tool search arguments are missing".into()))?;
            if !arguments.is_object() {
                return Err(ModelError::Protocol(
                    "tool search arguments must be a JSON object".into(),
                ));
            }
            ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: response_tool_item_id(value),
                    call_id: required_string(value, &["item", "call_id"], "tool search call id")?,
                    namespace: None,
                    name: crate::NATIVE_TOOL_SEARCH_NAME.to_string(),
                    arguments,
                    argument_error: None,
                },
            }
        }
        Some("reasoning") => ModelOutputItem::Reasoning {
            item_id: required_string(value, &["item", "id"], "reasoning item id")?,
            summary: reasoning_summary(value),
        },
        Some("compaction") => {
            let payload = required_string(
                value,
                &["item", "encrypted_content"],
                "compaction encrypted content",
            )?;
            ModelOutputItem::Compaction {
                item_id: required_string(value, &["item", "id"], "compaction item id")?,
                payload: ModelOpaqueCompactionPayload::parse(payload)
                    .map_err(ModelError::Protocol)?,
            }
        }
        _ => other_output_item(value),
    };
    Ok(ModelEvent::OutputItemCompleted { item })
}

/// A completed function call. Its `arguments` string is decoded tolerantly:
/// missing or empty is `{}`, and text that is not JSON (or a value that is
/// not a string) is `{}` with an argument error, so the call still pairs.
fn completed_function_call(value: &Value) -> ModelResult<ModelOutputItem> {
    let (arguments, argument_error) = match value.pointer("/item/arguments") {
        Some(Value::String(raw)) => crate::tolerant_decode::function_arguments(PROTOCOL, raw),
        None | Some(Value::Null) => (crate::tolerant_decode::empty_arguments(), None),
        Some(other) => (
            crate::tolerant_decode::empty_arguments(),
            Some(crate::tolerant_decode::unusable_arguments(
                PROTOCOL,
                "tool arguments are not a JSON string".into(),
                &other.to_string(),
            )),
        ),
    };
    Ok(ModelOutputItem::ToolCall {
        call: ModelToolCall {
            item_id: response_tool_item_id(value),
            call_id: required_string(value, &["item", "call_id"], "tool call id")?,
            namespace: response_tool_call_namespace(value),
            name: response_tool_call_name(value)?,
            arguments,
            argument_error,
        },
    })
}

fn other_output_item(value: &Value) -> ModelOutputItem {
    ModelOutputItem::Other {
        item_id: value
            .pointer("/item/id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        item_type: value
            .pointer("/item/type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned(),
    }
}

fn response_usage(value: &Value) -> Option<ModelUsage> {
    crate::tolerant_decode::token_usage(
        PROTOCOL,
        value.pointer("/response/usage"),
        "input_tokens",
        "output_tokens",
    )
}

/// `response.incomplete`: the response stopped early (for example at the
/// output token limit). The items completed before it stand.
fn decode_response_incomplete(provider: &ModelProvider, value: &Value) -> ModelResult<ModelEvent> {
    let response_id = required_string(value, &["response", "id"], "response id")?;
    let reason = value
        .pointer("/response/incomplete_details/reason")
        .and_then(Value::as_str)
        .filter(|reason| !reason.is_empty())
        .map_or_else(
            || "incomplete".to_owned(),
            |reason| bounded_metadata(&provider.bound_diagnostic(reason)),
        );
    log::warn!("model.response.incomplete protocol={PROTOCOL} reason={reason}");
    Ok(ModelEvent::ResponseIncomplete {
        response_id,
        usage: response_usage(value),
        reason,
    })
}

fn response_failure(
    provider: &ModelProvider,
    value: &Value,
    metadata: &ModelResponseMetadata,
) -> ModelError {
    let error = value
        .pointer("/response/error")
        .or_else(|| value.get("error"))
        .unwrap_or(value);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("model response failed");
    let code = error
        .get("code")
        .and_then(value_to_string)
        .map(|code| bounded_metadata(&code));
    crate::classify_remote_model_error(
        None,
        code,
        provider.bound_diagnostic(message),
        metadata.request_id.clone(),
    )
}

fn response_tool_call_name(value: &Value) -> ModelResult<String> {
    required_string(value, &["item", "name"], "tool name")
}

/// The call's native namespace exactly as sent, kept apart from its name:
/// the Agent's tool plan resolves the pair.
fn response_tool_call_namespace(value: &Value) -> Option<String> {
    value
        .pointer("/item/namespace")
        .and_then(Value::as_str)
        .filter(|namespace| !namespace.is_empty())
        .map(str::to_owned)
}

fn optional_string(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn response_tool_item_id(value: &Value) -> String {
    value
        .pointer("/item/id")
        .and_then(Value::as_str)
        .filter(|item_id| !item_id.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let sequence = NEXT_TEMPORARY_TOOL_ITEM_ID.fetch_add(1, Ordering::Relaxed);
            format!("temporary-tool-item-{sequence}")
        })
}

pub(super) fn decode_message_phase(phase: Option<&str>) -> Option<ModelMessagePhase> {
    match phase {
        Some("commentary") => Some(ModelMessagePhase::Commentary),
        Some("final_answer" | "final") => Some(ModelMessagePhase::FinalAnswer),
        _ => None,
    }
}

fn reasoning_summary(value: &Value) -> Vec<String> {
    value
        .pointer("/item/summary")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

fn required_string(value: &Value, path: &[&str], label: &str) -> ModelResult<String> {
    let mut current = value;
    for segment in path {
        current = current
            .get(*segment)
            .ok_or_else(|| ModelError::Protocol(format!("model event is missing {label}")))?;
    }
    current
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| ModelError::Protocol(format!("model event has invalid {label}")))
}
