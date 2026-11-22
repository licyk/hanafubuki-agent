use std::collections::HashMap;

use base64::Engine as _;
use serde_json::{json, Value};

use crate::model::{compaction_summary_text, continuation_context_text, image_detail_name};
use crate::tool_lowering::{flattened_tool_call_name, lower_function_tools};
use crate::{
    ModelApiProtocol, ModelError, ModelImageDetail, ModelInputItem, ModelOutputFormat,
    ModelReasoningReplay, ModelRequest, ModelResult, ModelRouteKey, ModelToolChoice,
    ModelUserContent,
};

pub(in crate::gemini) fn encode(
    request: &ModelRequest,
    route: &ModelRouteKey,
) -> ModelResult<Vec<u8>> {
    let model = ModelApiProtocol::GeminiInteractions.model_id(&request.model);
    if model.trim().is_empty() {
        return Err(ModelError::InvalidRequest(
            "model request requires a non-empty model".into(),
        ));
    }
    let mut body = json!({
        "model":model, "stream":true, "store":false,
        "system_instruction":request.instructions, "input":steps(&request.input, route)?
    });
    let tools = lower_function_tools(&request.tools, "Gemini Interactions")?;
    let mut config = serde_json::Map::new();
    if !tools.is_empty() || matches!(request.tool_choice, ModelToolChoice::Required { .. }) {
        let choice = match &request.tool_choice {
            ModelToolChoice::Auto => json!("auto"),
            ModelToolChoice::None => json!("none"),
            ModelToolChoice::Required { name } => {
                if !tools.iter().any(|tool| tool.name == *name) {
                    return Err(ModelError::InvalidRequest(
                        "required tool was not advertised".into(),
                    ));
                }
                json!({"allowed_tools":{"mode":"any","tools":[name]}})
            }
        };
        config.insert("tool_choice".into(), choice);
        body["tools"] = Value::Array(tools.into_iter().map(|tool| json!({
            "type":"function", "name":tool.name, "description":tool.description, "parameters":tool.input_schema
        })).collect());
    }
    if let Some(reasoning) = &request.reasoning {
        let level = crate::gemini::thinking_level(&reasoning.effort);
        config.insert("thinking_level".into(), json!(level));
        config.insert("thinking_summaries".into(), json!("auto"));
    }
    if !config.is_empty() {
        body["generation_config"] = Value::Object(config);
    }
    if let Some(ModelOutputFormat::JsonSchema(format)) = &request.output_format {
        format.validate().map_err(ModelError::InvalidRequest)?;
        body["response_format"] =
            json!({"type":"text","mime_type":"application/json","schema":format.schema});
    }
    serde_json::to_vec(&body).map_err(|_| {
        ModelError::InvalidRequest("Gemini Interactions request could not be encoded".into())
    })
}

fn steps(input: &[ModelInputItem], route: &ModelRouteKey) -> ModelResult<Vec<Value>> {
    let mut steps = Vec::with_capacity(input.len());
    let mut calls = HashMap::new();
    for item in input {
        let step = match item {
            ModelInputItem::UserMessage { content } => {
                json!({"type":"user_input","content":content.iter().map(user_part).collect::<Vec<_>>() })
            }
            ModelInputItem::ContextFragment { content, .. } => text_step("user_input", content),
            ModelInputItem::ContinuationContext {
                provider_id,
                hook_id,
                generation,
                content,
            } => text_step(
                "user_input",
                &continuation_context_text(provider_id, hook_id, generation, content),
            ),
            ModelInputItem::CompactionSummary {
                summary,
                source_revision,
                compacted_item_count,
            } => text_step(
                "user_input",
                &compaction_summary_text(summary, *source_revision, *compacted_item_count),
            ),
            ModelInputItem::AssistantMessage { text, .. } => text_step("model_output", text),
            ModelInputItem::Reasoning {
                replay:
                    ModelReasoningReplay::GeminiInteractionsThought {
                        route: originating_route,
                        signature,
                        summary,
                    },
            } if originating_route.endpoint() == route.endpoint() => {
                let mut thought = json!({"type":"thought","signature":signature});
                if let Some(summary) = summary {
                    thought["summary"] = json!(summary);
                }
                thought
            }
            ModelInputItem::Reasoning { .. } => continue,
            ModelInputItem::ToolCall {
                call_id,
                namespace,
                name,
                arguments,
            } => {
                let name = flattened_tool_call_name(namespace.as_deref(), name);
                let step =
                    json!({"type":"function_call","id":call_id,"name":name,"arguments":arguments});
                calls.insert(call_id, name);
                step
            }
            ModelInputItem::ToolOutput { call_id, output } => {
                let name = calls.remove(call_id).ok_or_else(|| ModelError::InvalidRequest("Gemini Interactions tool result has no matching function call in local history".into()))?;
                json!({"type":"function_result","call_id":call_id,"name":name,"is_error":output.is_error,"result":super::output::content(output)?})
            }
            ModelInputItem::CustomToolCall { .. }
            | ModelInputItem::CustomToolOutput { .. }
            | ModelInputItem::ToolSearchCall { .. }
            | ModelInputItem::ToolSearchOutput { .. } => {
                return Err(ModelError::InvalidRequest(
                    "Gemini Interactions cannot replay native custom or tool-search history".into(),
                ))
            }
            ModelInputItem::CompactionTrigger | ModelInputItem::Compaction { .. } => {
                return Err(ModelError::InvalidRequest(
                    "Gemini Interactions cannot replay a Responses compaction item".into(),
                ))
            }
        };
        steps.push(step);
    }
    Ok(steps)
}

fn text_step(kind: &str, text: &str) -> Value {
    json!({"type":kind,"content":[{"type":"text","text":text}]})
}

fn user_part(part: &ModelUserContent) -> Value {
    match part {
        ModelUserContent::InputText { text } => json!({"type":"text","text":text}),
        ModelUserContent::InputImage {
            mime_type,
            data,
            detail,
        } => {
            let mut image = json!({"type":"image","mime_type":mime_type,"data":base64::engine::general_purpose::STANDARD.encode(data)});
            if *detail != ModelImageDetail::Auto {
                image["resolution"] = json!(image_detail_name(*detail));
            }
            image
        }
    }
}
