mod assistant;
mod encode;
mod reasoning;

use encode::{encode_chat_request, value_to_string};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde_json::Value;

use crate::model::{
    ModelEvent, ModelOutputItem, ModelReasoningReplay, ModelReasoningTextKind, ModelRequest,
    ModelToolCall, ModelUsage,
};
use crate::port::{ModelError, ModelEventStream, ModelFuture, ModelPort, ModelResult, ModelStream};
use crate::provider::ModelProvider;
use crate::streaming::{build_client, Authentication, RawSseStream, StreamingRequestSpec};

#[derive(Clone)]
pub struct ChatCompletionsModelTransport {
    provider: Arc<ModelProvider>,
    client: reqwest::Client,
}

impl ChatCompletionsModelTransport {
    pub fn new(provider: ModelProvider) -> Result<Self, ModelError> {
        let client = build_client(&provider)?;
        Ok(Self {
            provider: Arc::new(provider),
            client,
        })
    }

    pub fn provider(&self) -> &ModelProvider {
        &self.provider
    }
}

impl ModelPort for ChatCompletionsModelTransport {
    fn tool_capabilities(&self) -> crate::ToolProviderCapabilities {
        crate::ModelApiProtocol::ChatCompletions.tool_capabilities()
    }

    fn structured_output_capability(&self) -> crate::ModelStructuredOutputCapability {
        crate::ModelApiProtocol::ChatCompletions.structured_output_capability()
    }

    fn route_key(&self, model: &str) -> Option<crate::ModelRouteKey> {
        Some(crate::ModelRouteKey::new(
            crate::ModelApiProtocol::ChatCompletions,
            self.provider.endpoint(),
            model,
        ))
    }

    fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        let provider = self.provider.clone();
        let client = self.client.clone();
        Box::pin(async move {
            let spec = StreamingRequestSpec {
                protocol: "chat_completions",
                endpoint: provider.endpoint().chat_completions_url()?,
                authentication: Authentication::Bearer,
                body: Arc::new(encode_chat_request(&request)?),
            };
            crate::streaming::trace_wire_body(
                "chat_completions",
                "request",
                &spec.endpoint,
                &spec.body,
            );
            let raw = RawSseStream::connect(provider, client, spec, cancellation).await?;
            Ok(Box::new(ChatEventStream::new(raw)) as ModelStream)
        })
    }
}

struct PartialToolCall {
    item_id: String,
    id: String,
    name: String,
    arguments: String,
}

/// The stretch of reasoning text streaming now. Chat has no reasoning item:
/// a stretch starts with its first delta and ends when the answer or a tool
/// call starts, or the response ends.
struct OpenReasoning {
    item_id: String,
    text: String,
    /// Whether every delta came as `reasoning_content`, which the server
    /// takes back in that field; `reasoning` (OpenRouter's) is not sent back.
    sent_back: bool,
}

struct ChatDecoder {
    response_id: Option<String>,
    reasoning: Option<OpenReasoning>,
    reasoning_items: u32,
    text: String,
    text_started: bool,
    tools: BTreeMap<u64, PartialToolCall>,
    usage: Option<ModelUsage>,
    finish_reason: Option<String>,
    completed: bool,
}

impl ChatDecoder {
    fn new() -> Self {
        Self {
            response_id: None,
            reasoning: None,
            reasoning_items: 0,
            text: String::new(),
            text_started: false,
            tools: BTreeMap::new(),
            usage: None,
            finish_reason: None,
            completed: false,
        }
    }

    fn decode(
        &mut self,
        frame: &[u8],
        metadata: &crate::ModelResponseMetadata,
    ) -> ModelResult<Vec<ModelEvent>> {
        if frame == b"[DONE]" {
            return self.finish();
        }
        let value: Value = serde_json::from_slice(frame).map_err(|_| {
            ModelError::Protocol("Chat Completions SSE event is not valid JSON".into())
        })?;
        if let Some(error) = value.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Chat Completions request failed");
            let code = error
                .get("code")
                .or_else(|| error.get("type"))
                .and_then(value_to_string);
            return Err(crate::classify_remote_model_error(
                None,
                code.map(crate::provider::truncate_diagnostic),
                crate::provider::truncate_diagnostic(message.to_owned()),
                metadata.request_id.clone(),
            ));
        }

        let mut events = Vec::new();
        if self.response_id.is_none() {
            if let Some(id) = value.get("id").and_then(Value::as_str) {
                self.response_id = Some(id.to_owned());
                events.push(ModelEvent::ResponseStarted {
                    response_id: id.to_owned(),
                    metadata: metadata.clone(),
                });
            }
        }
        if let Some(usage) = crate::tolerant_decode::token_usage(
            "chat_completions",
            value.get("usage"),
            "prompt_tokens",
            "completion_tokens",
        ) {
            self.usage = Some(usage);
        }
        // A usage-only chunk (`stream_options.include_usage`) may omit
        // `choices` entirely; it carries nothing else.
        let choices = value
            .get("choices")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for choice in choices {
            if choice.get("index").and_then(Value::as_u64).unwrap_or(0) != 0 {
                continue;
            }
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                self.finish_reason = Some(reason.to_owned());
            }
            let Some(delta) = choice.get("delta").and_then(Value::as_object) else {
                continue;
            };
            if let Some((field, reasoning)) = reasoning_delta(delta) {
                self.push_reasoning(reasoning, field == REASONING_CONTENT, &mut events)?;
            }
            if let Some(content) = delta.get("content").and_then(Value::as_str) {
                if !content.is_empty() {
                    self.close_reasoning(&mut events);
                    self.push_text(content, &mut events)?;
                }
            }
            if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for tool in tool_calls {
                    self.close_reasoning(&mut events);
                    self.push_tool_delta(tool, &mut events)?;
                }
            }
        }
        Ok(events)
    }

    fn push_reasoning(
        &mut self,
        delta: &str,
        sent_back: bool,
        events: &mut Vec<ModelEvent>,
    ) -> ModelResult<()> {
        if self.reasoning.is_none() {
            let response_id = self.response_id.as_deref().ok_or_else(|| {
                ModelError::Protocol("Chat Completions reasoning arrived before response id".into())
            })?;
            let item_id = format!("{response_id}:reasoning:{}", self.reasoning_items);
            self.reasoning_items += 1;
            events.push(ModelEvent::OutputItemAdded {
                item: ModelOutputItem::Reasoning {
                    item_id: item_id.clone(),
                    summary: Vec::new(),
                    content: Vec::new(),
                    replay: None,
                },
            });
            self.reasoning = Some(OpenReasoning {
                item_id,
                text: String::new(),
                sent_back: true,
            });
        }
        let reasoning = self.reasoning.as_mut().expect("reasoning opened above");
        reasoning.sent_back &= sent_back;
        crate::streaming::trace_reasoning("chat_completions", "delta", &reasoning.item_id, delta);
        reasoning.text.push_str(delta);
        events.push(ModelEvent::ReasoningDelta {
            item_id: reasoning.item_id.clone(),
            kind: ModelReasoningTextKind::Raw,
            index: 0,
            delta: delta.to_owned(),
        });
        Ok(())
    }

    fn close_reasoning(&mut self, events: &mut Vec<ModelEvent>) {
        if let Some(reasoning) = self.reasoning.take() {
            let replay = reasoning
                .sent_back
                .then(|| ModelReasoningReplay::reasoning_content(reasoning.text.clone()))
                .flatten();
            events.push(ModelEvent::OutputItemCompleted {
                item: ModelOutputItem::Reasoning {
                    item_id: reasoning.item_id,
                    summary: Vec::new(),
                    content: vec![reasoning.text],
                    replay,
                },
            });
        }
    }

    fn push_text(&mut self, content: &str, events: &mut Vec<ModelEvent>) -> ModelResult<()> {
        let item_id = self.text_item_id()?;
        if !self.text_started {
            self.text_started = true;
            events.push(ModelEvent::OutputItemAdded {
                item: ModelOutputItem::AssistantMessage {
                    item_id: item_id.clone(),
                    phase: None,
                    text: String::new(),
                },
            });
        }
        self.text.push_str(content);
        events.push(ModelEvent::AssistantTextDelta {
            item_id,
            delta: content.to_owned(),
        });
        Ok(())
    }

    fn push_tool_delta(&mut self, tool: &Value, events: &mut Vec<ModelEvent>) -> ModelResult<()> {
        let index = self.tool_delta_index(tool);
        if !self.tools.contains_key(&index) {
            let response_id = self.response_id.as_deref().ok_or_else(|| {
                ModelError::Protocol("Chat Completions tool arrived before response id".into())
            })?;
            let item_id = format!("{response_id}:tool:{index}");
            events.push(ModelEvent::OutputItemAdded {
                item: ModelOutputItem::ToolCall {
                    call: crate::ModelToolCall {
                        item_id: item_id.clone(),
                        call_id: String::new(),
                        namespace: None,
                        name: String::new(),
                        arguments: Value::Null,
                        argument_error: None,
                    },
                },
            });
            self.tools.insert(
                index,
                PartialToolCall {
                    item_id,
                    id: String::new(),
                    name: String::new(),
                    arguments: String::new(),
                },
            );
        }
        let partial = self.tools.get_mut(&index).expect("tool inserted above");
        // Ids may arrive in fragments; a server that repeats the whole id on
        // every delta of the call sends the id the call already has.
        if let Some(id) = tool.get("id").and_then(Value::as_str) {
            if partial.id != id {
                partial.id.push_str(id);
            }
        }
        if let Some(function) = tool.get("function").and_then(Value::as_object) {
            if let Some(name) = function.get("name").and_then(Value::as_str) {
                partial.name.push_str(name);
            }
            if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                partial.arguments.push_str(arguments);
            }
        }
        Ok(())
    }

    /// The call a tool delta belongs to. A delta with an `index` names it.
    /// Some compatible servers omit `index`; such a delta continues the
    /// latest call, unless it carries a call `id` other than the one that
    /// call already has, which makes it the start of a new call (servers
    /// that omit `index` send each call whole, with its own id). The first
    /// delta without an index is call 0.
    fn tool_delta_index(&self, tool: &Value) -> u64 {
        if let Some(index) = tool.get("index").and_then(Value::as_u64) {
            return index;
        }
        let Some((&latest, partial)) = self.tools.last_key_value() else {
            return 0;
        };
        let starts_new_call = tool
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty() && !partial.id.is_empty() && partial.id != id);
        if starts_new_call {
            latest + 1
        } else {
            latest
        }
    }

    fn text_item_id(&self) -> ModelResult<String> {
        self.response_id
            .as_ref()
            .map(|id| format!("{id}:message"))
            .ok_or_else(|| {
                ModelError::Protocol("Chat Completions text arrived before response id".into())
            })
    }

    /// The body ended cleanly without `[DONE]`. Some compatible servers end
    /// the stream that way; when a response with content (text or a tool
    /// call) arrived, it is complete. With no content at all, nothing usable
    /// arrived and the stream was cut short.
    fn finish_at_eof(&mut self) -> ModelResult<Vec<ModelEvent>> {
        if self.response_id.is_some() && (!self.text.is_empty() || !self.tools.is_empty()) {
            log::debug!("model.stream.completed_at_eof protocol=chat_completions");
            return self.finish();
        }
        Err(crate::streaming::ended_before_terminal_event())
    }

    fn finish(&mut self) -> ModelResult<Vec<ModelEvent>> {
        if self.completed {
            return Ok(Vec::new());
        }
        self.completed = true;
        let response_id = self.response_id.clone().ok_or_else(|| {
            ModelError::Protocol("Chat Completions stream ended without response id".into())
        })?;
        let mut events = Vec::new();
        self.close_reasoning(&mut events);
        if !self.text.is_empty() {
            events.push(ModelEvent::OutputItemCompleted {
                item: ModelOutputItem::AssistantMessage {
                    item_id: format!("{response_id}:message"),
                    phase: None,
                    text: self.text.clone(),
                },
            });
        }
        for partial in self.tools.values() {
            if partial.id.is_empty() || partial.name.is_empty() {
                return Err(ModelError::Protocol(
                    "Chat Completions tool call is missing id or name".into(),
                ));
            }
            // `""` is a call without arguments; Chat servers send it for a
            // tool that takes none.
            let (arguments, argument_error) =
                crate::tolerant_decode::function_arguments("chat_completions", &partial.arguments);
            events.push(ModelEvent::OutputItemCompleted {
                item: ModelOutputItem::ToolCall {
                    call: ModelToolCall {
                        item_id: partial.item_id.clone(),
                        call_id: partial.id.clone(),
                        namespace: None,
                        name: partial.name.clone(),
                        arguments,
                        argument_error,
                    },
                },
            });
        }
        // A missing finish reason (some servers never send one) leaves
        // `end_turn` unknown; the Agent then follows up on tool calls alone.
        let end_turn = match self.finish_reason.as_deref() {
            Some("tool_calls" | "function_call") => Some(false),
            Some("stop") => Some(true),
            _ => None,
        };
        events.push(ModelEvent::ResponseCompleted {
            response_id,
            usage: self.usage,
            end_turn,
        });
        Ok(events)
    }
}

const REASONING_CONTENT: &str = "reasoning_content";

/// A delta's reasoning text and the field it came in: `reasoning_content`
/// (DeepSeek, Qwen, vLLM) or `reasoning` (OpenRouter, newer vLLM). A server
/// that sends both, during that rename, sends the same text in each, so only
/// the first is read.
fn reasoning_delta(delta: &serde_json::Map<String, Value>) -> Option<(&'static str, &str)> {
    [REASONING_CONTENT, "reasoning"]
        .into_iter()
        .filter_map(|field| Some((field, delta.get(field).and_then(Value::as_str)?)))
        .find(|(_, text)| !text.is_empty())
}

struct ChatEventStream {
    raw: RawSseStream,
    decoder: ChatDecoder,
    pending: VecDeque<ModelResult<ModelEvent>>,
    terminal: bool,
}

impl ChatEventStream {
    fn new(raw: RawSseStream) -> Self {
        Self {
            raw,
            decoder: ChatDecoder::new(),
            pending: VecDeque::new(),
            terminal: false,
        }
    }

    async fn next_event(&mut self) -> Option<ModelResult<ModelEvent>> {
        if self.terminal {
            return None;
        }
        loop {
            if let Some(result) = self.pending.pop_front() {
                match result {
                    Ok(event) => {
                        if matches!(event, ModelEvent::ResponseCompleted { .. }) {
                            self.terminal = true;
                        }
                        return Some(Ok(event));
                    }
                    Err(error) => {
                        self.terminal = true;
                        return Some(Err(error));
                    }
                }
            }
            let decoded = match self.raw.next_frame().await {
                Ok(Some(frame)) => {
                    let metadata = self.raw.metadata().clone();
                    self.decoder.decode(&frame, &metadata)
                }
                Ok(None) => self.decoder.finish_at_eof(),
                Err(error) => {
                    self.terminal = true;
                    return Some(Err(error));
                }
            };
            match decoded {
                Ok(events) => self.pending.extend(events.into_iter().map(Ok)),
                Err(error) => self.pending.push_back(Err(error)),
            }
        }
    }
}

impl ModelEventStream for ChatEventStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelResult<ModelEvent>>> {
        Box::pin(self.next_event())
    }
}

#[cfg(test)]
mod tests;
