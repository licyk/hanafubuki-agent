use super::*;

pub(super) async fn establish_response(
    client: &reqwest::Client,
    provider: &ModelProvider,
    wire_body: &[u8],
    cancellation: &CancellationToken,
    attempts: &mut u32,
) -> ModelResult<EstablishedResponse> {
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
        let request = build_request(client, provider, wire_body)?;
        let sent = tokio::select! {
            result = tokio::time::timeout(provider.timeouts().request_establishment, request.send()) => {
                match result {
                    Ok(result) => result,
                    Err(_) => {
                        log::debug!(
                            "model response establishment timed out; retrying attempt={attempts}"
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
                let decoder = crate::streaming::sse_decoder_for(&response);
                let metadata = response_metadata(response.headers(), provider);
                return Ok(EstablishedResponse {
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
                        "model response establishment got retryable status {status}; retrying attempt={attempts}"
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
                        "model response establishment failed retryably: {error}; retrying attempt={attempts}"
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

fn build_request(
    client: &reqwest::Client,
    provider: &ModelProvider,
    wire_body: &[u8],
) -> ModelResult<reqwest::RequestBuilder> {
    let mut request = client
        .post(provider.endpoint().responses_url()?)
        .header(ACCEPT, "text/event-stream")
        .header(CONTENT_TYPE, "application/json")
        .body(wire_body.to_vec());
    if let Some(credential) = provider.credential() {
        let value = HeaderValue::from_str(&format!("Bearer {}", credential.expose()))
            .map_err(|_| ModelError::InvalidRequest("invalid model bearer credential".into()))?;
        request = request.header(AUTHORIZATION, value);
    }
    for header in provider.headers() {
        let (name, value) = header.parts();
        request = request.header(name, value);
    }
    Ok(request)
}

fn build_compact_request(
    client: &reqwest::Client,
    provider: &ModelProvider,
    wire_body: &[u8],
) -> ModelResult<reqwest::RequestBuilder> {
    let mut request = client
        .post(provider.endpoint().responses_compact_url()?)
        .header(ACCEPT, "application/json")
        .header(CONTENT_TYPE, "application/json")
        .body(wire_body.to_vec());
    if let Some(credential) = provider.credential() {
        let value = HeaderValue::from_str(&format!("Bearer {}", credential.expose()))
            .map_err(|_| ModelError::InvalidRequest("invalid model bearer credential".into()))?;
        request = request.header(AUTHORIZATION, value);
    }
    for header in provider.headers() {
        let (name, value) = header.parts();
        request = request.header(name, value);
    }
    Ok(request)
}

pub(super) async fn execute_remote_compaction(
    client: &reqwest::Client,
    provider: &ModelProvider,
    wire_body: &[u8],
    cancellation: &CancellationToken,
) -> ModelResult<ModelCompactionResult> {
    let mut attempts = 0u32;
    loop {
        if cancellation.is_cancelled() {
            return Err(ModelError::Cancelled);
        }
        attempts = attempts.saturating_add(1);
        let operation = async {
            let response = build_compact_request(client, provider, wire_body)?
                .send()
                .await
                .map_err(|_| ModelError::RequestFailed {
                    attempts,
                    message: "remote compaction request could not be sent".into(),
                })?;
            if !response.status().is_success() {
                let status = response.status();
                let metadata = response_metadata(response.headers(), provider);
                let (code, message) = read_remote_error(response, provider, cancellation).await?;
                return Err(if retryable_status(status) {
                    ModelError::RequestRetriesExhausted {
                        attempts,
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
            let mut body = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| {
                    ModelError::StreamInterrupted(
                        "remote compaction response body was interrupted".into(),
                    )
                })?;
                if body.len().saturating_add(chunk.len()) > MAX_REMOTE_COMPACTION_RESPONSE_BYTES {
                    return Err(ModelError::Protocol(
                        "remote compaction response exceeded the bounded body size".into(),
                    ));
                }
                body.extend_from_slice(&chunk);
            }
            decode_compact_response(&body)
        };
        let timeout = provider.timeouts().stream_idle.saturating_mul(2);
        let result = tokio::select! {
            result = tokio::time::timeout(timeout, operation) => match result {
                Ok(result) => result,
                Err(_) => Err(ModelError::RequestRetriesExhausted {
                    attempts,
                    status: None,
                    code: None,
                    message: "remote compaction unary request timed out".into(),
                    request_id: None,
                }),
            },
            _ = wait_cancelled(cancellation) => return Err(ModelError::Cancelled),
        };
        match result {
            Ok(result) => return Ok(result),
            Err(error @ ModelError::RequestRetriesExhausted { .. })
                if attempts < provider.retry_policy().maximum_attempts =>
            {
                retry_establishment(provider, cancellation, attempts).await?;
                log::warn!(
                    "model response compaction retrying after exhausted-attempt error attempt={attempts}: {error}"
                );
            }
            Err(error @ ModelError::RequestFailed { .. })
                if attempts < provider.retry_policy().maximum_attempts =>
            {
                retry_establishment(provider, cancellation, attempts).await?;
                log::warn!(
                    "model response compaction retrying after request failure attempt={attempts}: {error}"
                );
            }
            Err(ModelError::RequestFailed { message, .. }) => {
                return Err(ModelError::RequestRetriesExhausted {
                    attempts,
                    status: None,
                    code: None,
                    message,
                    request_id: None,
                });
            }
            Err(ModelError::StreamInterrupted(message))
                if attempts < provider.retry_policy().maximum_attempts =>
            {
                retry_establishment(provider, cancellation, attempts).await?;
                log::warn!(
                    "model response compaction retrying after stream interrupt attempt={attempts}: {message}"
                );
            }
            Err(ModelError::StreamInterrupted(message)) => {
                return Err(ModelError::RequestRetriesExhausted {
                    attempts,
                    status: None,
                    code: None,
                    message,
                    request_id: None,
                });
            }
            Err(error) => return Err(error),
        }
    }
}

async fn retry_establishment(
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
        "model response request retry attempt={attempts}/{} delay={delay:?} endpoint={:?}",
        provider.retry_policy().maximum_attempts,
        provider.endpoint()
    );
    tokio::select! {
        _ = tokio::time::sleep(delay) => Ok(true),
        _ = wait_cancelled(cancellation) => Err(ModelError::Cancelled),
    }
}

pub(super) async fn wait_cancelled(cancellation: &CancellationToken) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(CANCELLATION_POLL_INTERVAL).await;
    }
}

fn retryable_status(status: StatusCode) -> bool {
    status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.as_u16() == 529
        || (status.is_server_error() && status != StatusCode::NOT_IMPLEMENTED)
}

fn response_metadata(headers: &HeaderMap, provider: &ModelProvider) -> ModelResponseMetadata {
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

pub(super) fn bounded_metadata(value: &str) -> String {
    if value.len() <= MAX_METADATA_VALUE_BYTES {
        return value.to_owned();
    }
    let mut boundary = MAX_METADATA_VALUE_BYTES;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value[..boundary].to_owned()
}

async fn read_remote_error(
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
        .and_then(|error| error.get("code"))
        .and_then(value_to_string)
        .map(|code| bounded_metadata(&provider.bound_diagnostic(code)));
    let message = error
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("model provider returned an unsuccessful response");
    Ok((code, provider.bound_diagnostic(message)))
}

pub(super) fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}
