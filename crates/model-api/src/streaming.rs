use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_foundation::trace_content::bounded_trace_content;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Method, Response, StatusCode, Url};
use serde_json::Value;

use crate::model::{ModelResponseMetadata, RateLimitMetadata};
use crate::port::{ModelError, ModelResult};
use crate::provider::ModelProvider;
use crate::sse::SseDecoder;

const MAX_REMOTE_ERROR_BYTES: usize = 64 * 1024;
const MAX_METADATA_VALUE_BYTES: usize = 1024;
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(10);
pub(crate) const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Record the raw wire payload sent to or received from a model provider at
/// `trace` level only, behind the existing log-level gate. The payload is
/// bounded to `MAX_TRACE_CONTENT_BYTES` and never includes request headers.
pub(crate) fn trace_wire_body(
    protocol: &'static str,
    stage: &'static str,
    endpoint: &Url,
    body: &[u8],
) {
    if !log::log_enabled!(log::Level::Trace) {
        return;
    }
    let body = String::from_utf8_lossy(body);
    log::trace!(
        "model.wire.{stage} protocol={protocol} endpoint={endpoint} body={}",
        bounded_trace_content(&body)
    );
}

/// Record model reasoning/thinking content at `trace` level only, bounded to
/// `MAX_TRACE_CONTENT_BYTES`. The text also reaches the typed reasoning
/// events; no log below `trace` carries it.
pub(crate) fn trace_reasoning(
    protocol: &'static str,
    stage: &'static str,
    item_id: &str,
    content: &str,
) {
    if !log::log_enabled!(log::Level::Trace) {
        return;
    }
    log::trace!(
        "model.reasoning.{stage} protocol={protocol} item_id={item_id} content={}",
        bounded_trace_content(content)
    );
}

type HttpBody = Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Authentication {
    Bearer,
    Anthropic,
}

#[derive(Clone)]
pub(crate) struct StreamingRequestSpec {
    pub(crate) protocol: &'static str,
    pub(crate) endpoint: Url,
    pub(crate) authentication: Authentication,
    pub(crate) body: Arc<Vec<u8>>,
}

pub(crate) fn build_client(provider: &ModelProvider) -> ModelResult<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(provider.timeouts().connect)
        .build()
        .map_err(|_| ModelError::InvalidRequest("failed to build model HTTP client".into()))
}

pub(crate) fn authenticated_request(
    client: &reqwest::Client,
    provider: &ModelProvider,
    method: Method,
    endpoint: Url,
    authentication: Authentication,
) -> ModelResult<reqwest::RequestBuilder> {
    let mut request = client.request(method, endpoint);
    if let Some(credential) = provider.credential() {
        match authentication {
            Authentication::Bearer => {
                let value = HeaderValue::from_str(&format!("Bearer {}", credential.expose()))
                    .map_err(|_| {
                        ModelError::InvalidRequest("invalid model bearer credential".into())
                    })?;
                request = request.header(AUTHORIZATION, value);
            }
            Authentication::Anthropic => {
                let value = HeaderValue::from_str(credential.expose()).map_err(|_| {
                    ModelError::InvalidRequest("invalid Anthropic API credential".into())
                })?;
                request = request
                    .header("x-api-key", value)
                    .header("anthropic-version", ANTHROPIC_VERSION);
            }
        }
    } else if authentication == Authentication::Anthropic {
        request = request.header("anthropic-version", ANTHROPIC_VERSION);
    }
    for header in provider.headers() {
        let (name, value) = header.parts();
        request = request.header(name, value);
    }
    Ok(request)
}

struct EstablishedStream {
    body: HttpBody,
    metadata: ModelResponseMetadata,
    decoder: SseDecoder,
}

/// Shared async HTTP/SSE lifecycle for Chat Completions and Anthropic
/// Messages. Protocol adapters retain ownership of wire encoding and event
/// interpretation; this type owns only request establishment, bounded retry,
/// fragmentation-safe framing, idle timeout, and cancellation.
pub(crate) struct RawSseStream {
    provider: Arc<ModelProvider>,
    cancellation: CancellationToken,
    protocol: &'static str,
    endpoint: Url,
    body: HttpBody,
    metadata: ModelResponseMetadata,
    decoder: SseDecoder,
    pending: VecDeque<Vec<u8>>,
}

impl RawSseStream {
    pub(crate) async fn connect(
        provider: Arc<ModelProvider>,
        client: reqwest::Client,
        spec: StreamingRequestSpec,
        cancellation: CancellationToken,
    ) -> ModelResult<Self> {
        let mut attempts = 0;
        let established =
            establish_stream(&client, &provider, &spec, &cancellation, &mut attempts).await?;
        Ok(Self {
            provider,
            cancellation,
            protocol: spec.protocol,
            endpoint: spec.endpoint,
            body: established.body,
            metadata: established.metadata,
            decoder: established.decoder,
            pending: VecDeque::new(),
        })
    }

    pub(crate) fn metadata(&self) -> &ModelResponseMetadata {
        &self.metadata
    }

    /// The next complete SSE frame, or `None` when the body ended cleanly on
    /// an event boundary. What a clean end means is the protocol's decision.
    /// A body that fails (a dropped connection) is `StreamInterrupted`; one
    /// that ends inside an event is a protocol error, since the cut event
    /// could have carried anything.
    pub(crate) async fn next_frame(&mut self) -> ModelResult<Option<Vec<u8>>> {
        loop {
            if let Some(frame) = self.pending.pop_front() {
                trace_wire_body(self.protocol, "response_frame", &self.endpoint, &frame);
                return Ok(Some(frame));
            }
            if self.cancellation.is_cancelled() {
                return Err(ModelError::Cancelled);
            }
            let next = tokio::select! {
                chunk = self.body.next() => Some(chunk),
                _ = wait_cancelled(&self.cancellation) => None,
                _ = tokio::time::sleep(self.provider.timeouts().stream_idle) => {
                    return Err(ModelError::IdleTimeout {
                        timeout: self.provider.timeouts().stream_idle,
                    });
                }
            };
            let Some(next) = next else {
                return Err(ModelError::Cancelled);
            };
            match next {
                Some(Ok(chunk)) => {
                    self.pending.extend(self.decoder.feed(&chunk)?);
                }
                Some(Err(_)) => {
                    return Err(ModelError::StreamInterrupted(
                        "model response stream failed after establishment".into(),
                    ));
                }
                None => {
                    self.decoder.finish()?;
                    return Ok(None);
                }
            }
        }
    }
}

/// The error for a body that ended cleanly before the protocol's terminal
/// event, when what arrived is not a usable response.
pub(crate) fn ended_before_terminal_event() -> ModelError {
    ModelError::StreamInterrupted("model response stream ended before a terminal event".into())
}

async fn establish_stream(
    client: &reqwest::Client,
    provider: &ModelProvider,
    spec: &StreamingRequestSpec,
    cancellation: &CancellationToken,
    attempts: &mut u32,
) -> ModelResult<EstablishedStream> {
    loop {
        if cancellation.is_cancelled() {
            return Err(ModelError::Cancelled);
        }
        if *attempts >= provider.retry_policy().maximum_attempts {
            return Err(ModelError::RequestRetriesExhausted {
                attempts: *attempts,
                status: None,
                code: None,
                message: "model request exhausted establishment attempts".into(),
                request_id: None,
            });
        }
        *attempts += 1;
        let request = authenticated_request(
            client,
            provider,
            Method::POST,
            spec.endpoint.clone(),
            spec.authentication,
        )?
        .header(ACCEPT, "text/event-stream")
        .header(CONTENT_TYPE, "application/json")
        .body(spec.body.as_ref().clone());
        let sent = tokio::select! {
            result = tokio::time::timeout(provider.timeouts().request_establishment, request.send()) => {
                match result {
                    Ok(result) => result,
                    Err(_) => {
                        log::debug!(
                            "model request establishment timed out; retrying attempt={attempts}"
                        );
                        if retry_establishment(provider, cancellation, *attempts).await? {
                            continue;
                        }
                        return Err(ModelError::RequestRetriesExhausted {
                            attempts: *attempts,
                            status: None,
                            code: None,
                            message: "model request establishment timed out".into(),
                            request_id: None,
                        });
                    }
                }
            }
            _ = wait_cancelled(cancellation) => return Err(ModelError::Cancelled),
        };
        match sent {
            Ok(response) if response.status().is_success() => {
                let decoder = sse_decoder_for(&response);
                let metadata = response_metadata(response.headers(), provider);
                return Ok(EstablishedStream {
                    body: Box::pin(response.bytes_stream()),
                    metadata,
                    decoder,
                });
            }
            Ok(response) => {
                let status = response.status();
                let retryable = retryable_status(status);
                let metadata = response_metadata(response.headers(), provider);
                if retryable && *attempts < provider.retry_policy().maximum_attempts {
                    drop(response);
                    log::debug!(
                        "model request establishment got retryable status {status}; retrying attempt={attempts}"
                    );
                    retry_establishment(provider, cancellation, *attempts).await?;
                    continue;
                }
                let (code, message) = read_remote_error(response, provider, cancellation).await?;
                return Err(if retryable {
                    ModelError::RequestRetriesExhausted {
                        attempts: *attempts,
                        status: Some(status.as_u16()),
                        code,
                        message,
                        request_id: metadata.request_id,
                    }
                } else {
                    crate::classify_remote_model_error(
                        Some(status.as_u16()),
                        code,
                        message,
                        metadata.request_id,
                    )
                });
            }
            Err(error) => {
                let retryable = error.is_connect() || error.is_timeout() || error.is_request();
                if retryable {
                    log::debug!(
                        "model request establishment failed retryably: {error}; retrying attempt={attempts}"
                    );
                    if retry_establishment(provider, cancellation, *attempts).await? {
                        continue;
                    }
                }
                let message = if error.is_connect() {
                    "connection failed"
                } else if error.is_timeout() {
                    "request timed out"
                } else {
                    "request could not be sent"
                };
                return Err(if retryable {
                    ModelError::RequestRetriesExhausted {
                        attempts: *attempts,
                        status: None,
                        code: None,
                        message: message.into(),
                        request_id: None,
                    }
                } else {
                    ModelError::RequestFailed {
                        attempts: *attempts,
                        message: message.into(),
                    }
                });
            }
        }
    }
}

pub(crate) async fn retry_establishment(
    provider: &ModelProvider,
    cancellation: &CancellationToken,
    attempts: u32,
) -> ModelResult<bool> {
    if attempts >= provider.retry_policy().maximum_attempts {
        return Ok(false);
    }
    let shift = attempts.saturating_sub(1).min(31);
    let multiplier = 1u32 << shift;
    let delay = provider
        .retry_policy()
        .initial_backoff
        .saturating_mul(multiplier)
        .min(provider.retry_policy().maximum_backoff);
    log::debug!(
        "model request establishment retry attempt={attempts}/{} delay={delay:?} endpoint={:?}",
        provider.retry_policy().maximum_attempts,
        provider.endpoint()
    );
    tokio::select! {
        _ = tokio::time::sleep(delay) => Ok(true),
        _ = wait_cancelled(cancellation) => Err(ModelError::Cancelled),
    }
}

pub(crate) async fn wait_cancelled(cancellation: &CancellationToken) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(CANCELLATION_POLL_INTERVAL).await;
    }
}

pub(crate) fn response_metadata(
    headers: &HeaderMap,
    provider: &ModelProvider,
) -> ModelResponseMetadata {
    ModelResponseMetadata {
        request_id: first_header(
            headers,
            &["x-request-id", "openai-request-id", "request-id"],
            provider,
        ),
        rate_limit: RateLimitMetadata {
            limit_requests: first_header(headers, &["x-ratelimit-limit-requests"], provider),
            remaining_requests: first_header(
                headers,
                &["x-ratelimit-remaining-requests"],
                provider,
            ),
            reset_requests: first_header(headers, &["x-ratelimit-reset-requests"], provider),
            limit_tokens: first_header(headers, &["x-ratelimit-limit-tokens"], provider),
            remaining_tokens: first_header(headers, &["x-ratelimit-remaining-tokens"], provider),
            reset_tokens: first_header(headers, &["x-ratelimit-reset-tokens"], provider),
        },
    }
}

fn first_header(headers: &HeaderMap, names: &[&str], provider: &ModelProvider) -> Option<String> {
    names.iter().find_map(|name| {
        headers
            .get(*name)
            .and_then(|value| value.to_str().ok())
            .map(|value| bounded_metadata(&provider.bound_diagnostic(value)))
    })
}

fn bounded_metadata(value: &str) -> String {
    if value.len() <= MAX_METADATA_VALUE_BYTES {
        return value.to_owned();
    }
    let mut boundary = MAX_METADATA_VALUE_BYTES;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value[..boundary].to_owned()
}

pub(crate) async fn read_remote_error(
    response: Response,
    provider: &ModelProvider,
    cancellation: &CancellationToken,
) -> ModelResult<(Option<String>, String)> {
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    loop {
        let next = tokio::select! {
            next = tokio::time::timeout(provider.timeouts().stream_idle, stream.next()) => {
                match next {
                    Ok(next) => next,
                    Err(_) => break,
                }
            }
            _ = wait_cancelled(cancellation) => return Err(ModelError::Cancelled),
        };
        match next {
            Some(Ok(chunk)) => {
                let remaining = MAX_REMOTE_ERROR_BYTES.saturating_sub(body.len());
                body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
                if body.len() == MAX_REMOTE_ERROR_BYTES {
                    break;
                }
            }
            _ => break,
        }
    }
    let parsed = serde_json::from_slice::<Value>(&body).ok();
    let error = parsed.as_ref().and_then(|value| value.get("error"));
    let code = error
        .and_then(|error| error.get("code").or_else(|| error.get("type")))
        .and_then(value_to_string)
        .map(|code| bounded_metadata(&provider.bound_diagnostic(code)));
    let message = error
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("model provider returned an unsuccessful response");
    Ok((code, provider.bound_diagnostic(message)))
}

pub(crate) fn retryable_status(status: StatusCode) -> bool {
    status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.as_u16() == 529
        || (status.is_server_error() && status != StatusCode::NOT_IMPLEMENTED)
}

/// The SSE decoder for an established response: a body labelled
/// `text/event-stream` is trusted, any other is checked as it arrives.
pub(crate) fn sse_decoder_for(response: &Response) -> SseDecoder {
    let labelled = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"));
    if labelled {
        SseDecoder::new()
    } else {
        log::debug!("model.stream.unlabelled_body reason=content_type_not_event_stream");
        SseDecoder::for_unlabelled_body()
    }
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}
