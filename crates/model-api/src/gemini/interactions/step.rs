use serde_json::Value;

use crate::tolerant_decode::{empty_arguments, function_arguments, unusable_arguments};
use crate::{
    ModelEvent, ModelOutputItem, ModelReasoningReplay, ModelReasoningTextKind, ModelResult,
    ModelRouteKey, ModelToolCall,
};

pub(super) enum Step {
    Message {
        item_id: String,
        text: String,
    },
    Thought {
        item_id: String,
        signature: Option<String>,
        summary: Option<Vec<Value>>,
    },
    Call {
        call: ModelToolCall,
        arguments: String,
    },
}

impl Step {
    pub(super) fn start(
        step: &Value,
        item_id: String,
        events: &mut Vec<ModelEvent>,
    ) -> ModelResult<Option<Self>> {
        let (item, started) = match step["type"].as_str() {
            Some("model_output") => (
                Self::Message {
                    item_id: item_id.clone(),
                    text: String::new(),
                },
                ModelOutputItem::AssistantMessage {
                    item_id,
                    phase: None,
                    text: String::new(),
                },
            ),
            Some("thought") => (
                Self::Thought {
                    item_id: item_id.clone(),
                    signature: signature(&step["signature"]),
                    summary: step["summary"].as_array().map(|_| Vec::new()),
                },
                ModelOutputItem::Reasoning {
                    item_id,
                    summary: Vec::new(),
                    content: Vec::new(),
                    replay: None,
                },
            ),
            Some("function_call") => {
                let call_id = super::decode::identity(step, "id")?.to_owned();
                let name = super::decode::identity(step, "name")?.to_owned();
                let call = ModelToolCall {
                    item_id: call_id.clone(),
                    call_id,
                    name,
                    namespace: None,
                    arguments: step
                        .get("arguments")
                        .cloned()
                        .unwrap_or_else(empty_arguments),
                    argument_error: None,
                };
                let started = ModelOutputItem::Other {
                    item_id: Some(call.item_id.clone()),
                    item_type: "function_call".into(),
                };
                (
                    Self::Call {
                        call,
                        arguments: String::new(),
                    },
                    started,
                )
            }
            _ => return Ok(None),
        };
        events.push(ModelEvent::OutputItemAdded { item: started });
        let mut item = item;
        // The start frame can already contain the first text or thought summary.
        let content = match item {
            Self::Message { .. } => step["content"].as_array(),
            Self::Thought { .. } => step["summary"].as_array(),
            Self::Call { .. } => None,
        };
        for part in content.into_iter().flatten() {
            if let Some(event) = item.content(part) {
                events.push(event);
            }
        }
        Ok(Some(item))
    }

    pub(super) fn apply(&mut self, delta: &Value) -> Option<ModelEvent> {
        match (&mut *self, delta["type"].as_str()?) {
            (Self::Message { .. }, "text") => self.content(delta),
            (Self::Thought { .. }, "thought_summary") => self.content(&delta["content"]),
            (
                Self::Thought {
                    signature: current, ..
                },
                "thought_signature",
            ) => {
                if let Some(signature) = signature(&delta["signature"]) {
                    *current = Some(signature);
                }
                None
            }
            (Self::Call { arguments, call }, "arguments_delta") => {
                if let Some(value) = delta.get("arguments") {
                    if let Some(text) = value.as_str() {
                        arguments.push_str(text);
                    } else if !value.is_null() {
                        call.argument_error = Some(unusable_arguments(
                            "gemini_interactions",
                            "function argument delta must be JSON text".into(),
                            &value.to_string(),
                        ));
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn content(&mut self, part: &Value) -> Option<ModelEvent> {
        match self {
            Self::Thought {
                item_id, summary, ..
            } if part.is_object() => {
                let parts = summary.get_or_insert_default();
                let index = parts.len() as u32;
                parts.push(part.clone());
                Some(ModelEvent::ReasoningDelta {
                    item_id: item_id.clone(),
                    kind: ModelReasoningTextKind::Summary,
                    index,
                    delta: part_text(part)?.to_owned(),
                })
            }
            Self::Message { item_id, text } => {
                let next = part_text(part)?;
                text.push_str(next);
                Some(ModelEvent::AssistantTextDelta {
                    item_id: item_id.clone(),
                    delta: next.to_owned(),
                })
            }
            _ => None,
        }
    }

    pub(super) fn finish(self, route: &ModelRouteKey) -> ModelOutputItem {
        match self {
            Self::Message { item_id, text } => ModelOutputItem::AssistantMessage {
                item_id,
                text,
                phase: None,
            },
            Self::Thought {
                item_id,
                summary,
                signature,
            } => {
                let text = summary
                    .iter()
                    .flatten()
                    .filter_map(part_text)
                    .map(str::to_owned)
                    .collect();
                let replay =
                    signature.map(
                        |signature| ModelReasoningReplay::GeminiInteractionsThought {
                            route: route.clone(),
                            signature,
                            summary,
                        },
                    );
                ModelOutputItem::Reasoning {
                    item_id,
                    summary: text,
                    content: Vec::new(),
                    replay,
                }
            }
            Self::Call {
                mut call,
                arguments,
            } => {
                if call.argument_error.is_some() {
                    call.arguments = empty_arguments();
                } else if !arguments.is_empty() {
                    (call.arguments, call.argument_error) =
                        function_arguments("gemini_interactions", &arguments);
                }
                if !call.arguments.is_object() {
                    call.argument_error = Some(unusable_arguments(
                        "gemini_interactions",
                        "function arguments must be an object".into(),
                        &call.arguments.to_string(),
                    ));
                    call.arguments = empty_arguments();
                }
                ModelOutputItem::ToolCall { call }
            }
        }
    }
}

fn signature(value: &Value) -> Option<String> {
    value.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

fn part_text(part: &Value) -> Option<&str> {
    part["text"].as_str().filter(|_| part["type"] == "text")
}
