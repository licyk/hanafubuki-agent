mod assistant;
mod encode;
mod thinking;

use encode::{encode_anthropic_request, required_string, required_u64};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde_json::Value;

use crate::model::{
    ModelEvent, ModelOutputItem, ModelReasoningReplay, ModelReasoningTextKind, ModelRequest,
    ModelToolArgumentError, ModelToolCall, ModelUsage,
};
use crate::port::{ModelError, ModelEventStream, ModelFuture, ModelPort, ModelResult, ModelStream};
use crate::provider::ModelProvider;
use crate::streaming::{build_client, Authentication, RawSseStream, StreamingRequestSpec};

const DEFAULT_MAX_OUTPUT_TOKENS: u64 = 16_384;

#[derive(Clone)]
pub struct AnthropicMessagesModelTransport {
    provider: Arc<ModelProvider>,
    client: reqwest::Client,
}

impl AnthropicMessagesModelTransport {
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

impl ModelPort for AnthropicMessagesModelTransport {
    fn tool_capabilities(&self) -> crate::ToolProviderCapabilities {
        crate::ModelApiProtocol::AnthropicMessages.tool_capabilities()
    }

    fn structured_output_capability(&self) -> crate::ModelStructuredOutputCapability {
        crate::ModelApiProtocol::AnthropicMessages.structured_output_capability()
    }

    fn route_key(&self, model: &str) -> Option<crate::ModelRouteKey> {
        Some(crate::ModelRouteKey::new(
            crate::ModelApiProtocol::AnthropicMessages,
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
                protocol: "anthropic_messages",
                endpoint: provider.endpoint().anthropic_messages_url()?,
                authentication: Authentication::Anthropic,
                body: Arc::new(encode_anthropic_request(&request)?),
            };
            crate::streaming::trace_wire_body(
                "anthropic_messages",
                "request",
                &spec.endpoint,
                &spec.body,
            );
            let raw = RawSseStream::connect(provider, client, spec, cancellation).await?;
            Ok(Box::new(AnthropicEventStream::new(raw)) as ModelStream)
        })
    }
}

enum ContentBlock {
    Text {
        item_id: String,
        text: String,
    },
    Tool {
        id: String,
        name: String,
        initial_input: Option<Value>,
        partial_json: String,
    },
    /// A thinking block: its text and its signature (`signature_delta`),
    /// kept whole to be sent back.
    Thinking {
        item_id: String,
        text: String,
        signature: String,
    },
    RedactedThinking {
        item_id: String,
        data: String,
    },
    Ignored,
}

struct AnthropicDecoder {
    response_id: Option<String>,
    /// The uncached input: Anthropic's `input_tokens` excludes the cache.
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
    cache_write_tokens: Option<u64>,
    /// A token count arrived that is not an unsigned integer; the response
    /// then reports no usage rather than a count it cannot trust.
    usage_invalid: bool,
    stop_reason: Option<String>,
    blocks: BTreeMap<u64, ContentBlock>,
    completed: bool,
}

const PROTOCOL: &str = "anthropic_messages";

impl AnthropicDecoder {
    fn new() -> Self {
        Self {
            response_id: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            usage_invalid: false,
            stop_reason: None,
            blocks: BTreeMap::new(),
            completed: false,
        }
    }

    fn decode(
        &mut self,
        frame: &[u8],
        metadata: &crate::ModelResponseMetadata,
    ) -> ModelResult<Vec<ModelEvent>> {
        let value: Value = serde_json::from_slice(frame)
            .map_err(|_| ModelError::Protocol("Anthropic SSE event is not valid JSON".into()))?;
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| ModelError::Protocol("Anthropic event has no type".into()))?;
        match event_type {
            "message_start" => {
                let message = value.get("message").ok_or_else(|| {
                    ModelError::Protocol("Anthropic message_start has no message".into())
                })?;
                let response_id = required_string(message, "id", "message id")?;
                self.response_id = Some(response_id.clone());
                if let Some(usage) = message.get("usage") {
                    self.record_usage(usage);
                }
                Ok(vec![ModelEvent::ResponseStarted {
                    response_id,
                    metadata: metadata.clone(),
                }])
            }
            "content_block_start" => self.start_block(&value),
            "content_block_delta" => self.apply_block_delta(&value),
            "content_block_stop" => self.stop_block(&value),
            "message_delta" => {
                if let Some(reason) = value.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop_reason = Some(reason.to_owned());
                }
                if let Some(usage) = value.get("usage") {
                    self.record_usage(usage);
                }
                Ok(Vec::new())
            }
            "message_stop" => self.finish(),
            "ping" => Ok(Vec::new()),
            "error" => {
                let error = value.get("error").unwrap_or(&value);
                Err(crate::classify_remote_model_error(
                    None,
                    error
                        .get("type")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .map(crate::provider::truncate_diagnostic),
                    crate::provider::truncate_diagnostic(
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("Anthropic stream failed")
                            .to_owned(),
                    ),
                    metadata.request_id.clone(),
                ))
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Records the counts a usage object carries. `message_start` and
    /// `message_delta` each carry some; a later count replaces an earlier
    /// one, and an absent count keeps it. A cache count that does not decode
    /// is dropped on its own.
    fn record_usage(&mut self, usage: &Value) {
        use crate::tolerant_decode::{cache_token_count, token_count};
        match token_count(PROTOCOL, usage, "input_tokens") {
            Ok(Some(input)) => self.input_tokens = Some(input),
            Ok(None) => {}
            Err(_) => self.usage_invalid = true,
        }
        match token_count(PROTOCOL, usage, "output_tokens") {
            Ok(Some(output)) => self.output_tokens = Some(output),
            Ok(None) => {}
            Err(_) => self.usage_invalid = true,
        }
        for (pointer, count) in [
            ("/cache_read_input_tokens", &mut self.cache_read_tokens),
            ("/cache_creation_input_tokens", &mut self.cache_write_tokens),
        ] {
            if !usage.pointer(pointer).is_none_or(Value::is_null) {
                *count = cache_token_count(PROTOCOL, usage, pointer);
            }
        }
    }

    fn start_block(&mut self, value: &Value) -> ModelResult<Vec<ModelEvent>> {
        let index = required_u64(value, "index", "content block index")?;
        let block = value
            .get("content_block")
            .ok_or_else(|| ModelError::Protocol("Anthropic content block has no payload".into()))?;
        let kind = block.get("type").and_then(Value::as_str).unwrap_or("");
        let (content, started) = match kind {
            "text" => {
                let item_id = format!("{}:text:{index}", self.response_id()?);
                let text = block
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                (
                    ContentBlock::Text {
                        item_id: item_id.clone(),
                        text,
                    },
                    Some(ModelEvent::OutputItemAdded {
                        item: ModelOutputItem::AssistantMessage {
                            item_id,
                            phase: None,
                            text: String::new(),
                        },
                    }),
                )
            }
            "tool_use" => {
                let id = required_string(block, "id", "tool use id")?;
                let name = required_string(block, "name", "tool name")?;
                (
                    ContentBlock::Tool {
                        id: id.clone(),
                        name: name.clone(),
                        initial_input: block.get("input").cloned(),
                        partial_json: String::new(),
                    },
                    Some(ModelEvent::OutputItemAdded {
                        item: ModelOutputItem::ToolCall {
                            call: crate::ModelToolCall {
                                item_id: id.clone(),
                                call_id: id,
                                namespace: None,
                                name,
                                arguments: block.get("input").cloned().unwrap_or(Value::Null),
                                argument_error: None,
                            },
                        },
                    }),
                )
            }
            "thinking" => {
                let item_id = format!("{}:thinking:{index}", self.response_id()?);
                let text = block
                    .get("thinking")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                crate::streaming::trace_reasoning(PROTOCOL, "block_started", &item_id, &text);
                let signature = block
                    .get("signature")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                (
                    ContentBlock::Thinking {
                        item_id: item_id.clone(),
                        text,
                        signature,
                    },
                    Some(reasoning_added(item_id)),
                )
            }
            "redacted_thinking" => {
                let item_id = format!("{}:thinking:{index}", self.response_id()?);
                let data = block
                    .get("data")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                (
                    ContentBlock::RedactedThinking {
                        item_id: item_id.clone(),
                        data,
                    },
                    Some(reasoning_added(item_id)),
                )
            }
            _ => (ContentBlock::Ignored, None),
        };
        self.blocks.insert(index, content);
        Ok(started.into_iter().collect())
    }

    fn apply_block_delta(&mut self, value: &Value) -> ModelResult<Vec<ModelEvent>> {
        let index = required_u64(value, "index", "content block index")?;
        let delta = value
            .get("delta")
            .ok_or_else(|| ModelError::Protocol("Anthropic content delta has no payload".into()))?;
        let kind = delta.get("type").and_then(Value::as_str).unwrap_or("");
        // A delta for a block that never started has nothing to extend; it
        // is dropped and the rest of the response stands.
        let Some(block) = self.blocks.get_mut(&index) else {
            log::debug!("model.stream.ignored protocol={PROTOCOL} event=content_block_delta reason=unknown_block");
            return Ok(Vec::new());
        };
        match (block, kind) {
            (ContentBlock::Text { item_id, text }, "text_delta") => {
                let value = required_string(delta, "text", "text delta")?;
                text.push_str(&value);
                Ok(vec![ModelEvent::AssistantTextDelta {
                    item_id: item_id.clone(),
                    delta: value,
                }])
            }
            (ContentBlock::Tool { partial_json, .. }, "input_json_delta") => {
                partial_json.push_str(
                    delta
                        .get("partial_json")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                );
                Ok(Vec::new())
            }
            (ContentBlock::Thinking { signature, .. }, "signature_delta") => {
                signature.push_str(
                    delta
                        .get("signature")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                );
                Ok(Vec::new())
            }
            (ContentBlock::Thinking { item_id, text, .. }, "thinking_delta") => {
                let value = delta
                    .get("thinking")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                crate::streaming::trace_reasoning(PROTOCOL, "delta", item_id, &value);
                if value.is_empty() {
                    return Ok(Vec::new());
                }
                text.push_str(&value);
                Ok(vec![ModelEvent::ReasoningDelta {
                    item_id: item_id.clone(),
                    kind: ModelReasoningTextKind::Raw,
                    index: 0,
                    delta: value,
                }])
            }
            _ => Ok(Vec::new()),
        }
    }

    fn stop_block(&mut self, value: &Value) -> ModelResult<Vec<ModelEvent>> {
        let index = required_u64(value, "index", "content block index")?;
        let Some(block) = self.blocks.remove(&index) else {
            log::debug!("model.stream.ignored protocol={PROTOCOL} event=content_block_stop reason=unknown_block");
            return Ok(Vec::new());
        };
        match block {
            ContentBlock::Text { item_id, text } => Ok(vec![ModelEvent::OutputItemCompleted {
                item: ModelOutputItem::AssistantMessage {
                    item_id,
                    phase: None,
                    text,
                },
            }]),
            ContentBlock::Tool {
                id,
                name,
                initial_input,
                partial_json,
            } => {
                let (arguments, argument_error) = tool_input(initial_input, &partial_json);
                Ok(vec![ModelEvent::OutputItemCompleted {
                    item: ModelOutputItem::ToolCall {
                        call: ModelToolCall {
                            item_id: id.clone(),
                            call_id: id,
                            namespace: None,
                            name,
                            arguments,
                            argument_error,
                        },
                    },
                }])
            }
            ContentBlock::Thinking {
                item_id,
                text,
                signature,
            } => Ok(vec![ModelEvent::OutputItemCompleted {
                item: ModelOutputItem::Reasoning {
                    item_id,
                    summary: Vec::new(),
                    content: vec![text.clone()],
                    replay: ModelReasoningReplay::thinking(text, signature),
                },
            }]),
            ContentBlock::RedactedThinking { item_id, data } => {
                Ok(vec![ModelEvent::OutputItemCompleted {
                    item: ModelOutputItem::Reasoning {
                        item_id,
                        summary: Vec::new(),
                        content: Vec::new(),
                        replay: ModelReasoningReplay::redacted_thinking(data),
                    },
                }])
            }
            ContentBlock::Ignored => Ok(Vec::new()),
        }
    }

    fn response_id(&self) -> ModelResult<&str> {
        self.response_id.as_deref().ok_or_else(|| {
            ModelError::Protocol("Anthropic event arrived before message_start".into())
        })
    }

    fn finish(&mut self) -> ModelResult<Vec<ModelEvent>> {
        if self.completed {
            return Ok(Vec::new());
        }
        self.completed = true;
        if !self.blocks.is_empty() {
            return Err(ModelError::Protocol(
                "Anthropic stream ended with incomplete content blocks".into(),
            ));
        }
        let response_id = self.response_id()?.to_owned();
        // `message_stop` is the terminal event; a missing stop reason only
        // leaves `end_turn` unknown.
        let usage = match (self.usage_invalid, self.input_tokens, self.output_tokens) {
            (true, _, _) | (false, None, None) => None,
            (false, input_tokens, output_tokens) => Some(ModelUsage {
                input_tokens: input_tokens
                    .unwrap_or(0)
                    .saturating_add(self.cache_read_tokens.unwrap_or(0))
                    .saturating_add(self.cache_write_tokens.unwrap_or(0)),
                output_tokens: output_tokens.unwrap_or(0),
                cache_read_tokens: self.cache_read_tokens,
                cache_write_tokens: self.cache_write_tokens,
            }),
        };
        Ok(vec![ModelEvent::ResponseCompleted {
            response_id,
            usage,
            end_turn: match self.stop_reason.as_deref() {
                Some("tool_use") => Some(false),
                Some("end_turn" | "stop_sequence") => Some(true),
                _ => None,
            },
        }])
    }
}

fn reasoning_added(item_id: String) -> ModelEvent {
    ModelEvent::OutputItemAdded {
        item: ModelOutputItem::Reasoning {
            item_id,
            summary: Vec::new(),
            content: Vec::new(),
            replay: None,
        },
    }
}

/// A tool call's input: the streamed `input_json_delta` text when there is
/// any, else the `input` of `content_block_start`, else none (`{}`). Input
/// that is not JSON, or not a JSON object, is `{}` with an argument error:
/// `tool_use.input` must be an object when the call is replayed.
fn tool_input(
    initial_input: Option<Value>,
    partial_json: &str,
) -> (Value, Option<ModelToolArgumentError>) {
    let (arguments, argument_error) = if partial_json.trim().is_empty() {
        match initial_input {
            None | Some(Value::Null) => (crate::tolerant_decode::empty_arguments(), None),
            Some(input) => (input, None),
        }
    } else {
        crate::tolerant_decode::function_arguments(PROTOCOL, partial_json)
    };
    if argument_error.is_some() || arguments.is_object() {
        return (arguments, argument_error);
    }
    let raw = if partial_json.trim().is_empty() {
        arguments.to_string()
    } else {
        partial_json.to_owned()
    };
    (
        crate::tolerant_decode::empty_arguments(),
        Some(crate::tolerant_decode::unusable_arguments(
            PROTOCOL,
            "tool input must be a JSON object".into(),
            &raw,
        )),
    )
}

struct AnthropicEventStream {
    raw: RawSseStream,
    decoder: AnthropicDecoder,
    pending: VecDeque<ModelResult<ModelEvent>>,
    terminal: bool,
}

impl AnthropicEventStream {
    fn new(raw: RawSseStream) -> Self {
        Self {
            raw,
            decoder: AnthropicDecoder::new(),
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
            let frame = match self.raw.next_frame().await {
                Ok(Some(frame)) => frame,
                Ok(None) => {
                    self.terminal = true;
                    return Some(Err(crate::streaming::ended_before_terminal_event()));
                }
                Err(error) => {
                    self.terminal = true;
                    return Some(Err(error));
                }
            };
            let metadata = self.raw.metadata().clone();
            match self.decoder.decode(&frame, &metadata) {
                Ok(events) => self.pending.extend(events.into_iter().map(Ok)),
                Err(error) => self.pending.push_back(Err(error)),
            }
        }
    }
}

impl ModelEventStream for AnthropicEventStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelResult<ModelEvent>>> {
        Box::pin(self.next_event())
    }
}

#[cfg(test)]
mod tests;
