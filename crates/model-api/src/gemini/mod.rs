mod generate;
mod interactions;

use std::{collections::VecDeque, sync::Arc};

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde_json::Value;

use crate::streaming::{
    build_client, trace_wire_body, Authentication, RawSseStream, StreamingRequestSpec,
};
use crate::{
    ModelApiProtocol, ModelError, ModelEvent, ModelEventStream, ModelFuture, ModelPort,
    ModelProvider, ModelRequest, ModelResponseMetadata, ModelResult, ModelRouteKey, ModelStream,
    ModelStructuredOutputCapability, ToolProviderCapabilities,
};

#[derive(Clone)]
pub struct GeminiModelTransport {
    provider: Arc<ModelProvider>,
    client: reqwest::Client,
    protocol: ModelApiProtocol,
}

impl GeminiModelTransport {
    pub fn new(provider: ModelProvider, protocol: ModelApiProtocol) -> ModelResult<Self> {
        if !matches!(
            protocol,
            ModelApiProtocol::GeminiGenerateContent | ModelApiProtocol::GeminiInteractions
        ) {
            return Err(ModelError::InvalidRequest(
                "expected a native Gemini protocol".into(),
            ));
        }
        Ok(Self {
            client: build_client(&provider)?,
            provider: Arc::new(provider),
            protocol,
        })
    }
}

impl ModelPort for GeminiModelTransport {
    fn tool_capabilities(&self) -> ToolProviderCapabilities {
        self.protocol.tool_capabilities()
    }

    fn structured_output_capability(&self) -> ModelStructuredOutputCapability {
        self.protocol.structured_output_capability()
    }

    fn route_key(&self, model: &str) -> Option<ModelRouteKey> {
        Some(ModelRouteKey::new(
            self.protocol,
            self.provider.endpoint(),
            model,
        ))
    }

    fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        Box::pin(async move {
            let route = ModelRouteKey::new(self.protocol, self.provider.endpoint(), &request.model);
            let (endpoint, body, decoder) = if self.protocol == ModelApiProtocol::GeminiInteractions
            {
                (
                    self.provider.endpoint().interactions_url()?,
                    interactions::encode(&request, &route)?,
                    Decoder::Interactions(interactions::Decoder::new(route)),
                )
            } else {
                (
                    self.provider
                        .endpoint()
                        .generate_content_url(&request.model)?,
                    generate::encode(&request, &route)?,
                    Decoder::Generate(generate::Decoder::new(route)),
                )
            };
            let protocol = self.protocol.as_str();
            trace_wire_body(protocol, "request", &endpoint, &body);
            let spec = StreamingRequestSpec {
                protocol,
                endpoint,
                authentication: Authentication::Google,
                body: Arc::new(body),
            };
            let raw = RawSseStream::connect(
                self.provider.clone(),
                self.client.clone(),
                spec,
                cancellation,
            )
            .await?;
            Ok(Box::new(EventStream {
                raw,
                provider: self.provider.clone(),
                decoder,
                pending: VecDeque::new(),
                terminal: false,
            }) as ModelStream)
        })
    }
}

fn thinking_level(effort: &crate::ModelReasoningEffort) -> &str {
    match effort.as_str() {
        "light" => "low",
        "balanced" => "medium",
        "deep" => "high",
        "max" => "xhigh",
        level => level,
    }
}

enum Decoder {
    Generate(generate::Decoder),
    Interactions(interactions::Decoder),
}

struct EventStream {
    raw: RawSseStream,
    provider: Arc<ModelProvider>,
    decoder: Decoder,
    pending: VecDeque<ModelEvent>,
    terminal: bool,
}

impl EventStream {
    async fn read_events(&mut self) -> ModelResult<Vec<ModelEvent>> {
        let frame = self.raw.next_frame().await?;
        let Some(frame) = frame.filter(|frame| frame.trim_ascii() != b"[DONE]") else {
            return match &mut self.decoder {
                Decoder::Generate(decoder) => decoder.finish(),
                Decoder::Interactions(_) => Err(crate::streaming::ended_before_terminal_event()),
            };
        };
        let value: Value = serde_json::from_slice(&frame)
            .map_err(|_| ModelError::Protocol("Gemini SSE event is not valid JSON".into()))?;
        let metadata = self.raw.metadata();
        let error = match &self.decoder {
            Decoder::Generate(_) => value.get("error").filter(|e| e.is_object()),
            Decoder::Interactions(_) => interactions::stream_error(&value),
        };
        if let Some(error) = error {
            return Err(remote_error(error, metadata, &self.provider));
        }
        match &mut self.decoder {
            Decoder::Generate(decoder) => decoder.decode(&value, metadata),
            Decoder::Interactions(decoder) => decoder.decode(&value, metadata),
        }
    }

    async fn next_event(&mut self) -> Option<ModelResult<ModelEvent>> {
        while !self.terminal {
            if let Some(event) = self.pending.pop_front() {
                self.terminal = matches!(
                    event,
                    ModelEvent::ResponseCompleted { .. } | ModelEvent::ResponseIncomplete { .. }
                );
                return Some(Ok(event));
            }
            match self.read_events().await {
                Ok(events) => self.pending.extend(events),
                Err(error) => {
                    self.terminal = true;
                    return Some(Err(error));
                }
            }
        }
        None
    }
}

fn remote_error(
    error: &Value,
    metadata: &ModelResponseMetadata,
    provider: &ModelProvider,
) -> ModelError {
    let status = error["code"]
        .as_u64()
        .and_then(|code| u16::try_from(code).ok());
    let code = error["status"].as_str().or_else(|| error["code"].as_str());
    crate::classify_remote_model_error(
        status,
        code.map(|c| provider.bound_diagnostic(c)),
        provider.bound_diagnostic(error["message"].as_str().unwrap_or("Gemini stream failed")),
        metadata.request_id.clone(),
    )
}

impl ModelEventStream for EventStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelResult<ModelEvent>>> {
        Box::pin(self.next_event())
    }
}
