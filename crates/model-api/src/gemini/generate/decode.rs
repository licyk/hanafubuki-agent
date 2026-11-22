use serde_json::{json, Value};

use crate::{
    GeminiReplayPart, ModelError, ModelEvent, ModelOutputItem, ModelReasoningReplay,
    ModelReasoningTextKind, ModelResponseMetadata, ModelResult, ModelRouteKey,
    ModelToolArgumentError, ModelToolCall, ModelUsage,
};

pub(in crate::gemini) struct Decoder {
    route: ModelRouteKey,
    local_id: String,
    response_id: Option<String>,
    sequence: usize,
    text: Option<Text>,
    parts: Vec<GeminiReplayPart>,
    calls: Vec<ModelToolCall>,
    usage: Usage,
    finish_reason: Option<String>,
    finished: bool,
}

struct Text {
    id: String,
    thought: bool,
    value: String,
}

impl Text {
    fn item(&self, completed: bool) -> ModelOutputItem {
        if self.thought {
            ModelOutputItem::Reasoning {
                item_id: self.id.clone(),
                summary: if completed {
                    vec![self.value.clone()]
                } else {
                    Vec::new()
                },
                content: Vec::new(),
                replay: None,
            }
        } else {
            ModelOutputItem::AssistantMessage {
                item_id: self.id.clone(),
                phase: None,
                text: if completed {
                    self.value.clone()
                } else {
                    String::new()
                },
            }
        }
    }
}

impl Decoder {
    pub(in crate::gemini) fn new(route: ModelRouteKey) -> Self {
        // Some servers omit both responseId and functionCall.id. The local
        // call identity also has to survive a process restart in saved history.
        Self {
            route,
            local_id: format!("gemini-{}", uuid::Uuid::new_v4()),
            response_id: None,
            sequence: 0,
            text: None,
            parts: Vec::new(),
            calls: Vec::new(),
            usage: Usage::default(),
            finish_reason: None,
            finished: false,
        }
    }

    pub(in crate::gemini) fn decode(
        &mut self,
        value: &Value,
        metadata: &ModelResponseMetadata,
    ) -> ModelResult<Vec<ModelEvent>> {
        let mut events = Vec::new();
        if self.finished {
            return Ok(events);
        }
        if let Some(usage) = value.get("usageMetadata") {
            self.usage.update(usage);
        }
        let candidate = value
            .get("candidates")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
            .find_map(|(index, c)| {
                (c.is_object()
                    && c.get("index")
                        .and_then(Value::as_u64)
                        .unwrap_or(index as u64)
                        == 0)
                    .then_some(c)
            });
        let blocked = value
            .pointer("/promptFeedback/blockReason")
            .and_then(Value::as_str)
            .filter(|r| !r.is_empty() && *r != "BLOCK_REASON_UNSPECIFIED");
        if candidate.is_none() && blocked.is_none() {
            return Ok(events);
        }
        if self.response_id.is_none() {
            let response_id = value
                .get("responseId")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .unwrap_or(&self.local_id)
                .to_owned();
            self.response_id = Some(response_id.clone());
            events.push(ModelEvent::ResponseStarted {
                response_id,
                metadata: metadata.clone(),
            });
        }
        if self.finish_reason.is_some() {
            return Ok(events);
        }
        if let Some(reason) = blocked {
            let reason = crate::provider::truncate_diagnostic(reason.to_owned());
            return Err(ModelError::Remote {
                status: None,
                code: Some(reason.clone()),
                message: format!("Gemini blocked the prompt: {reason}"),
                request_id: metadata.request_id.clone(),
            });
        } else if let Some(candidate) = candidate {
            if let Some(parts) = candidate
                .pointer("/content/parts")
                .and_then(Value::as_array)
            {
                for part in parts {
                    self.part(part, &mut events);
                }
            }
            self.finish_reason = candidate
                .get("finishReason")
                .and_then(Value::as_str)
                .filter(|r| !r.is_empty() && *r != "FINISH_REASON_UNSPECIFIED")
                .map(str::to_owned);
        }
        Ok(events)
    }

    fn next_id(&mut self) -> String {
        self.sequence += 1;
        format!("{}-{}", self.local_id, self.sequence)
    }

    fn end_text(&mut self, events: &mut Vec<ModelEvent>) {
        if let Some(text) = self.text.take() {
            events.push(ModelEvent::OutputItemCompleted {
                item: text.item(true),
            });
        }
    }

    fn part(&mut self, part: &Value, events: &mut Vec<ModelEvent>) {
        let signature = part
            .get("thoughtSignature")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if let Some(call) = part.get("functionCall") {
            let Some(name) = call
                .get("name")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            else {
                return;
            };
            self.end_text(events);
            let item_id = self.next_id();
            let provider_id = call
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let call_id = provider_id.clone().unwrap_or_else(|| item_id.clone());
            let (arguments, argument_error) = match call.get("args") {
                None | Some(Value::Null) => (json!({}), None),
                Some(args @ Value::Object(_)) => (args.clone(), None),
                Some(other) => (
                    json!({}),
                    Some(ModelToolArgumentError::new(
                        "Gemini function arguments must be an object",
                        &other.to_string(),
                    )),
                ),
            };
            self.parts.push(GeminiReplayPart::Call {
                call_id: call_id.clone(),
                provider_id,
                signature,
            });
            let call = ModelToolCall {
                item_id,
                call_id,
                name: name.into(),
                namespace: None,
                arguments,
                argument_error,
            };
            // Reserve the position now. Only a completed candidate authorizes the
            // call, since safety or an output limit may still interrupt this stream.
            events.push(ModelEvent::OutputItemAdded {
                item: ModelOutputItem::Other {
                    item_id: Some(call.item_id.clone()),
                    item_type: "function_call".into(),
                },
            });
            self.calls.push(call);
        } else if let Some(text) = part.get("text").and_then(Value::as_str) {
            let thought = part
                .get("thought")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            self.parts.push(GeminiReplayPart::Text {
                text: text.into(),
                thought,
                signature,
            });
            if !text.is_empty() {
                if self
                    .text
                    .as_ref()
                    .is_none_or(|current| current.thought != thought)
                {
                    self.end_text(events);
                    let text = Text {
                        id: self.next_id(),
                        thought,
                        value: String::new(),
                    };
                    events.push(ModelEvent::OutputItemAdded {
                        item: text.item(false),
                    });
                    self.text = Some(text);
                }
                let current = self.text.as_mut().expect("text block started");
                current.value.push_str(text);
                let item_id = current.id.clone();
                let delta = text.to_owned();
                events.push(if thought {
                    ModelEvent::ReasoningDelta {
                        item_id,
                        kind: ModelReasoningTextKind::Summary,
                        index: 0,
                        delta,
                    }
                } else {
                    ModelEvent::AssistantTextDelta { item_id, delta }
                });
            }
        }
    }

    pub(in crate::gemini) fn finish(&mut self) -> ModelResult<Vec<ModelEvent>> {
        if self.finished {
            return Ok(Vec::new());
        }
        let reason = self.finish_reason.take().ok_or_else(|| {
            ModelError::StreamInterrupted(
                "Gemini stream ended before the candidate finished".into(),
            )
        })?;
        self.finished = true;
        let response_id = self.response_id.take().expect("finish follows a response");
        let mut events = Vec::new();
        self.end_text(&mut events);
        let has_calls = !self.calls.is_empty();
        if reason == "STOP" {
            events.extend(std::mem::take(&mut self.calls).into_iter().map(|call| {
                ModelEvent::OutputItemCompleted {
                    item: ModelOutputItem::ToolCall { call },
                }
            }));
        } else if reason != "MAX_TOKENS" {
            let reason = crate::provider::truncate_diagnostic(reason);
            return Err(ModelError::Remote {
                status: None,
                code: Some(reason.clone()),
                message: format!("Gemini candidate stopped: {reason}"),
                request_id: None,
            });
        }
        if reason == "STOP"
            && (has_calls
                || self.parts.iter().any(|part| {
                    matches!(
                        part,
                        GeminiReplayPart::Text {
                            signature: Some(_),
                            ..
                        }
                    )
                }))
        {
            let replay = ModelReasoningReplay::GeminiGenerate {
                route: self.route.clone(),
                parts: std::mem::take(&mut self.parts),
            };
            events.push(ModelEvent::OutputItemCompleted {
                item: ModelOutputItem::Reasoning {
                    item_id: self.next_id(),
                    summary: Vec::new(),
                    content: Vec::new(),
                    replay: Some(replay),
                },
            });
        }
        let usage = self.usage.total();
        events.push(if reason == "STOP" {
            ModelEvent::ResponseCompleted {
                response_id,
                usage,
                end_turn: Some(!has_calls),
            }
        } else {
            ModelEvent::ResponseIncomplete {
                response_id,
                usage,
                reason: crate::provider::truncate_diagnostic(reason),
            }
        });
        Ok(events)
    }
}

#[derive(Default)]
struct Usage {
    prompt: Option<u64>,
    output: Option<u64>,
    thoughts: Option<u64>,
    tools: Option<u64>,
    cached: Option<u64>,
}

impl Usage {
    // Each count is cumulative. An omitted count in the last frame can
    // already have been reported in an earlier one.
    fn update(&mut self, data: &Value) {
        for (field, count) in [
            ("promptTokenCount", &mut self.prompt),
            ("candidatesTokenCount", &mut self.output),
            ("thoughtsTokenCount", &mut self.thoughts),
            ("toolUsePromptTokenCount", &mut self.tools),
            ("cachedContentTokenCount", &mut self.cached),
        ] {
            if let Ok(Some(value)) =
                crate::tolerant_decode::token_count("GenerateContent", data, field)
            {
                *count = Some(value);
            }
        }
    }

    fn total(&self) -> Option<ModelUsage> {
        [self.prompt, self.output, self.thoughts, self.tools]
            .iter()
            .any(Option::is_some)
            .then(|| ModelUsage {
                input_tokens: self
                    .prompt
                    .unwrap_or(0)
                    .saturating_add(self.tools.unwrap_or(0)),
                output_tokens: self
                    .output
                    .unwrap_or(0)
                    .saturating_add(self.thoughts.unwrap_or(0)),
                cache_read_tokens: self.cached,
                cache_write_tokens: None,
            })
    }
}
