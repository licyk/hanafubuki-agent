use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Response, StatusCode, Url};
use serde_json::{json, Value};

use crate::model::{
    ModelCompactionCapability, ModelCompactionResult, ModelContextRole, ModelEvent, ModelInputItem,
    ModelMessagePhase, ModelOpaqueCompactionPayload, ModelOutputItem, ModelRequest,
    ModelResponseMetadata, ModelToolCall, ModelToolChoice, ModelUsage, ModelUserContent,
    RateLimitMetadata,
};
use crate::port::{ModelError, ModelEventStream, ModelFuture, ModelPort, ModelResult, ModelStream};
use crate::provider::ModelProvider;
use crate::sse::SseDecoder;

mod decode;
mod encode;
mod transport;

use decode::{decode_message_phase, decode_sse_event};
use encode::{decode_compact_response, encode_responses_compact_request, encode_responses_request};
use transport::{
    bounded_metadata, establish_response, execute_remote_compaction, value_to_string,
    wait_cancelled,
};

const MAX_REMOTE_ERROR_BYTES: usize = 64 * 1024;
const MAX_REMOTE_COMPACTION_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_METADATA_VALUE_BYTES: usize = 1024;
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(10);

type HttpBody = Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>;

#[derive(Clone)]
pub struct ResponsesModelTransport {
    provider: Arc<ModelProvider>,
    client: reqwest::Client,
    compaction_capability: ModelCompactionCapability,
}

impl ResponsesModelTransport {
    pub fn new(provider: ModelProvider) -> Result<Self, ModelError> {
        let client = reqwest::Client::builder()
            .connect_timeout(provider.timeouts().connect)
            .build()
            .map_err(|_| ModelError::InvalidRequest("failed to build model HTTP client".into()))?;
        Ok(Self {
            provider: Arc::new(provider),
            client,
            compaction_capability: ModelCompactionCapability::LocalSummary,
        })
    }

    pub fn with_compaction_capability(mut self, capability: ModelCompactionCapability) -> Self {
        self.compaction_capability = capability;
        self
    }

    pub fn provider(&self) -> &ModelProvider {
        &self.provider
    }
}

impl ModelPort for ResponsesModelTransport {
    fn compaction_capability(&self) -> ModelCompactionCapability {
        self.compaction_capability
    }

    fn tool_capabilities(&self) -> crate::ToolProviderCapabilities {
        crate::ModelApiProtocol::Responses.tool_capabilities()
    }

    fn structured_output_capability(&self) -> crate::ModelStructuredOutputCapability {
        crate::ModelApiProtocol::Responses.structured_output_capability()
    }

    fn route_key(&self, model: &str) -> Option<crate::ModelRouteKey> {
        Some(crate::ModelRouteKey::new(
            crate::ModelApiProtocol::Responses,
            self.provider.endpoint(),
            model,
        ))
    }

    fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        let client = self.client.clone();
        let provider = self.provider.clone();
        Box::pin(async move {
            let wire_body = Arc::new(encode_responses_request(&request)?);
            let endpoint = provider.endpoint().responses_url()?;
            crate::streaming::trace_wire_body("responses", "request", &endpoint, &wire_body);
            let mut attempts = 0;
            let established =
                establish_response(&client, &provider, &wire_body, &cancellation, &mut attempts)
                    .await?;
            Ok(Box::new(ResponsesEventStream::new(
                client,
                provider,
                wire_body,
                cancellation,
                attempts,
                established,
                endpoint,
            )) as ModelStream)
        })
    }

    fn compact(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelCompactionResult>> {
        let client = self.client.clone();
        let provider = self.provider.clone();
        let capability = self.compaction_capability;
        Box::pin(async move {
            if capability != ModelCompactionCapability::ResponsesCompact {
                return Err(ModelError::InvalidRequest(
                    "Responses compact endpoint was not enabled for this provider".into(),
                ));
            }
            if request.input.is_empty() {
                return Ok(ModelCompactionResult {
                    replacement_history: Vec::new(),
                    usage: None,
                });
            }
            let wire_body = encode_responses_compact_request(&request)?;
            let endpoint = provider.endpoint().responses_compact_url()?;
            crate::streaming::trace_wire_body(
                "responses",
                "compact_request",
                &endpoint,
                &wire_body,
            );
            execute_remote_compaction(&client, &provider, &wire_body, &cancellation).await
        })
    }
}

struct EstablishedResponse {
    body: HttpBody,
    metadata: ModelResponseMetadata,
    decoder: SseDecoder,
}

struct ResponsesEventStream {
    provider: Arc<ModelProvider>,
    cancellation: CancellationToken,
    endpoint: Url,
    body: HttpBody,
    metadata: ModelResponseMetadata,
    decoder: SseDecoder,
    pending: VecDeque<ModelResult<ModelEvent>>,
    terminal: bool,
}

impl ResponsesEventStream {
    fn new(
        _client: reqwest::Client,
        provider: Arc<ModelProvider>,
        _wire_body: Arc<Vec<u8>>,
        cancellation: CancellationToken,
        _attempts: u32,
        established: EstablishedResponse,
        endpoint: Url,
    ) -> Self {
        Self {
            provider,
            cancellation,
            endpoint,
            body: established.body,
            metadata: established.metadata,
            decoder: established.decoder,
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
                        if matches!(
                            event,
                            ModelEvent::ResponseCompleted { .. }
                                | ModelEvent::ResponseIncomplete { .. }
                        ) {
                            self.terminal = true;
                        }
                        return Some(Ok(event));
                    }
                    Err(error) => return self.fail(error),
                }
            }
            if self.cancellation.is_cancelled() {
                return self.fail(ModelError::Cancelled);
            }

            let next = tokio::select! {
                chunk = self.body.next() => Some(chunk),
                _ = wait_cancelled(&self.cancellation) => None,
                _ = tokio::time::sleep(self.provider.timeouts().stream_idle) => {
                    return self.fail(ModelError::IdleTimeout {
                        timeout: self.provider.timeouts().stream_idle,
                    });
                }
            };
            let Some(next) = next else {
                return self.fail(ModelError::Cancelled);
            };
            match next {
                Some(Ok(chunk)) => {
                    let frames = match self.decoder.feed(&chunk) {
                        Ok(frames) => frames,
                        Err(error) => return self.fail(error),
                    };
                    for frame in frames {
                        crate::streaming::trace_wire_body(
                            "responses",
                            "response_frame",
                            &self.endpoint,
                            &frame,
                        );
                        match decode_sse_event(&self.provider, &frame, &self.metadata) {
                            Ok(Some(event)) => self.pending.push_back(Ok(event)),
                            Ok(None) => {}
                            Err(error) => {
                                self.pending.push_back(Err(error));
                                break;
                            }
                        }
                    }
                }
                Some(Err(_)) => {
                    return self.fail(ModelError::StreamInterrupted(
                        "model response stream failed after establishment".into(),
                    ));
                }
                None => {
                    if let Err(error) = self.decoder.finish() {
                        return self.fail(error);
                    }
                    return self.fail(ModelError::StreamInterrupted(
                        "model response stream ended before a terminal event".into(),
                    ));
                }
            }
        }
    }

    fn fail(&mut self, error: ModelError) -> Option<ModelResult<ModelEvent>> {
        self.terminal = true;
        Some(Err(error))
    }
}

impl ModelEventStream for ResponsesEventStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelResult<ModelEvent>>> {
        Box::pin(self.next_event())
    }
}
