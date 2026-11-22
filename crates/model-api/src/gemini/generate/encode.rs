use base64::Engine as _;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;

use super::output::function_response;
use crate::{
    ModelError, ModelInputItem, ModelOutputFormat, ModelRequest, ModelResult, ModelRouteKey,
    ModelToolChoice, ModelUserContent,
};

#[derive(Serialize)]
struct Content {
    role: &'static str,
    parts: Vec<Value>,
}

#[derive(Default)]
struct Contents {
    messages: Vec<Content>,
    calls: HashMap<String, Call>,
}

struct Call {
    name: String,
    provider_id: Option<String>,
    order: usize,
}

impl Contents {
    fn assistant(
        &mut self,
        items: &[ModelInputItem],
        route: &ModelRouteKey,
        current_turn: bool,
    ) -> ModelResult<()> {
        let signed_calls = current_turn && model_generation(route).is_some_and(|major| major >= 3);
        let mut first_call = true;
        // Each replay follows the ordinary history items produced by that candidate.
        for group in items.split_inclusive(|item| {
            matches!(
                item,
                ModelInputItem::Reasoning {
                    replay: crate::ModelReasoningReplay::GeminiGenerate { .. }
                }
            )
        }) {
            let replay = match group.last() {
                Some(ModelInputItem::Reasoning {
                    replay:
                        crate::ModelReasoningReplay::GeminiGenerate {
                            route: source,
                            parts,
                        },
                }) if source == route => Some(parts),
                _ => None,
            };
            if let Some(parts) = replay {
                // A compressed view attaches a surviving call's wire metadata
                // directly to it. Keep any projected text preceding that call.
                if let [crate::GeminiReplayPart::Call { call_id, .. }] = parts.as_slice() {
                    if let Some(at) = group.iter().position(|item| {
                        matches!(item, ModelInputItem::ToolCall { call_id: id, .. } if id == call_id)
                    }) {
                        self.plain(&group[..at], signed_calls, &mut first_call)?;
                    }
                }
                for part in parts {
                    match part {
                        crate::GeminiReplayPart::Text {
                            text,
                            thought,
                            signature,
                        } => {
                            let mut part = json!({"text":text});
                            if *thought {
                                part["thought"] = json!(true);
                            }
                            if let Some(signature) = signature {
                                part["thoughtSignature"] = json!(signature);
                            }
                            self.push("model", part);
                        }
                        crate::GeminiReplayPart::Call {
                            call_id,
                            provider_id,
                            signature,
                        } => {
                            let item = group.iter().find(|item| {
                                matches!(item, ModelInputItem::ToolCall { call_id: id, .. } if id == call_id)
                            }).ok_or_else(|| ModelError::InvalidRequest(
                                "Gemini replay is missing its original function call; start a new turn or restore its history".into()
                            ))?;
                            self.call(
                                item,
                                provider_id.as_deref(),
                                signature.as_deref(),
                                signed_calls && first_call,
                            )?;
                            first_call = false;
                        }
                    }
                }
            } else {
                self.plain(group, signed_calls, &mut first_call)?;
            }
        }
        Ok(())
    }

    fn plain(
        &mut self,
        group: &[ModelInputItem],
        signed_calls: bool,
        first_call: &mut bool,
    ) -> ModelResult<()> {
        for item in group {
            match item {
                ModelInputItem::AssistantMessage { text, .. } => {
                    self.push("model", json!({"text":text}))
                }
                ModelInputItem::ToolCall { .. } => {
                    self.call(item, None, None, signed_calls && *first_call)?;
                    *first_call = false;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn call(
        &mut self,
        item: &ModelInputItem,
        provider_id: Option<&str>,
        signature: Option<&str>,
        needs_signature: bool,
    ) -> ModelResult<()> {
        let ModelInputItem::ToolCall {
            call_id,
            namespace,
            name,
            arguments,
        } = item
        else {
            unreachable!()
        };
        if needs_signature && signature.is_none() {
            return Err(ModelError::InvalidRequest("Gemini needs the original thought signature to continue this function call. Return to the original model or start a new conversation.".into()));
        }
        let name = crate::flattened_tool_call_name(namespace.as_deref(), name).into_owned();
        let mut part = json!({"functionCall":{"name":name,"args":arguments}});
        if let Some(id) = provider_id {
            part["functionCall"]["id"] = json!(id);
        }
        if let Some(signature) = signature {
            part["thoughtSignature"] = json!(signature);
        }
        self.calls.insert(
            call_id.to_owned(),
            Call {
                name,
                provider_id: provider_id.map(str::to_owned),
                order: self.calls.len(),
            },
        );
        self.push("model", part);
        Ok(())
    }
    fn push(&mut self, role: &'static str, part: Value) {
        if let Some(last) = self.messages.last_mut().filter(|last| last.role == role) {
            last.parts.push(part);
        } else {
            self.messages.push(Content {
                role,
                parts: vec![part],
            });
        }
    }

    fn outputs(&mut self, input: &[ModelInputItem], separate_media: bool) -> ModelResult<()> {
        let mut responses = Vec::new();
        for item in input {
            if let ModelInputItem::ToolOutput { call_id, output } = item {
                let call = self.calls.get(call_id).ok_or_else(|| {
                    ModelError::InvalidRequest(
                        "Gemini function result has no preceding call in local history".into(),
                    )
                })?;
                responses.push((
                    call.order,
                    function_response(
                        output,
                        &call.name,
                        call.provider_id.as_deref(),
                        call.order,
                        separate_media,
                    )?,
                ));
            }
        }
        // Google matches parallel calls without IDs by their original order.
        responses.sort_by_key(|(order, _)| *order);
        let mut images = Vec::new();
        for (_, (part, media)) in responses {
            self.push("user", part);
            images.extend(media);
        }
        for image in images {
            self.push("user", image);
        }
        Ok(())
    }
}

pub(in crate::gemini) fn encode(
    request: &ModelRequest,
    route: &ModelRouteKey,
) -> ModelResult<Vec<u8>> {
    let mut body = json!({"contents":history(&request.input, route)?});
    if !request.instructions.is_empty() {
        body["systemInstruction"] = json!({"parts":[{"text":request.instructions}]});
    }
    let tools = crate::tool_lowering::lower_function_tools(&request.tools, "GenerateContent")?;
    if !tools.is_empty() || matches!(request.tool_choice, ModelToolChoice::Required { .. }) {
        let choice = match &request.tool_choice {
            ModelToolChoice::Auto => json!({"mode":"AUTO"}),
            ModelToolChoice::None => json!({"mode":"NONE"}),
            ModelToolChoice::Required { name } => {
                if !tools.iter().any(|tool| &tool.name == name) {
                    return Err(ModelError::InvalidRequest(
                        "required model tool is not present in the request".into(),
                    ));
                }
                json!({"mode":"ANY","allowedFunctionNames":[name]})
            }
        };
        body["toolConfig"] = json!({"functionCallingConfig":choice});
        let declarations = tools
            .into_iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parametersJsonSchema": tool.input_schema,
                })
            })
            .collect::<Vec<_>>();
        body["tools"] = json!([{"functionDeclarations":declarations}]);
    }
    if let Some(ModelOutputFormat::JsonSchema(format)) = &request.output_format {
        format.validate().map_err(ModelError::InvalidRequest)?;
        body["generationConfig"]["responseMimeType"] = json!("application/json");
        body["generationConfig"]["responseJsonSchema"] = format.schema.clone();
    }
    if let Some(reasoning) = &request.reasoning {
        let effort = crate::gemini::thinking_level(&reasoning.effort);
        let thinking = if route.model().starts_with("gemini-2.5-") {
            // This generation accepts a token budget; Flash and Pro both support
            // this range. A provider-specific numeric effort can name a budget.
            let budget = match effort {
                "minimal" => 512,
                "low" => 1024,
                "medium" => 8192,
                "high" => 16384,
                "xhigh" => 24576,
                _ => effort.parse::<i32>().map_err(|_| {
                    ModelError::InvalidRequest(format!(
                        "Gemini 2.5 cannot map reasoning effort {effort} to a thinking budget"
                    ))
                })?,
            };
            json!({"includeThoughts":true,"thinkingBudget":budget})
        } else {
            json!({"includeThoughts":true,"thinkingLevel":effort.to_ascii_uppercase()})
        };
        body["generationConfig"]["thinkingConfig"] = thinking;
    }
    serde_json::to_vec(&body).map_err(|e| ModelError::InvalidRequest(e.to_string()))
}

fn history(all: &[ModelInputItem], route: &ModelRouteKey) -> ModelResult<Vec<Content>> {
    let mut contents = Contents::default();
    let legacy_images = model_generation(route).is_some_and(|major| major < 3);
    let last_prompt = all
        .iter()
        .rposition(|item| matches!(item, ModelInputItem::UserMessage { .. }));
    let mut input = all;
    while let Some((item, rest)) = input.split_first() {
        match item {
            ModelInputItem::UserMessage { content } => {
                for part in content {
                    let part = match part {
                        ModelUserContent::InputText { text } => json!({"text": text}),
                        ModelUserContent::InputImage { mime_type, data, detail } => {
                            let data = base64::engine::general_purpose::STANDARD.encode(data);
                            let mut image = json!({"inlineData": {"mimeType": mime_type, "data": data}});
                            if !legacy_images && *detail != crate::ModelImageDetail::Auto {
                                let level = crate::model::image_detail_name(*detail).to_ascii_uppercase();
                                image["mediaResolution"] = json!({"level":format!("MEDIA_RESOLUTION_{level}")});
                            }
                            image
                        }
                    };
                    contents.push("user", part);
                }
            }
            ModelInputItem::ToolOutput { .. } => {
                let count = input.iter().take_while(|item| matches!(item, ModelInputItem::ToolOutput { .. })).count();
                // Older Gemini models take tool images as top-level user parts.
                contents.outputs(&input[..count], legacy_images)?;
                input = &input[count..];
                continue;
            }
            item if is_assistant(item) => {
                let count = input.iter().take_while(|item| is_assistant(item)).count();
                let current_turn = last_prompt.is_none_or(|last| all.len() - input.len() > last);
                contents.assistant(&input[..count], route, current_turn)?;
                input = &input[count..];
                continue;
            }
            ModelInputItem::ContextFragment { content, .. } => {
                contents.push("user", json!({"text": content}));
            }
            ModelInputItem::ContinuationContext { provider_id, hook_id, generation, content } => {
                let text = crate::model::continuation_context_text(provider_id,hook_id,generation,content);
                contents.push("user", json!({"text": text}));
            }
            ModelInputItem::CompactionSummary { summary, source_revision, compacted_item_count } => {
                let text = crate::model::compaction_summary_text(summary,*source_revision,*compacted_item_count);
                contents.push("user", json!({"text": text}));
            }
            _ => return Err(ModelError::InvalidRequest(
                "GenerateContent cannot replay provider-specific compaction, custom or native tool-search history".into()
            )),
        }
        input = rest;
    }
    Ok(contents.messages)
}

fn model_generation(route: &ModelRouteKey) -> Option<u32> {
    route
        .model()
        .strip_prefix("gemini-")?
        .split(['.', '-'])
        .next()?
        .parse()
        .ok()
}

fn is_assistant(item: &ModelInputItem) -> bool {
    matches!(
        item,
        ModelInputItem::AssistantMessage { .. }
            | ModelInputItem::ToolCall { .. }
            | ModelInputItem::Reasoning { .. }
    )
}
