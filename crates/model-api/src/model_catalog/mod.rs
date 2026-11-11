//! A Provider's model listing (`GET {base}/models`), read tolerantly: an
//! entry the Agent cannot use is left out and counted, a field it cannot
//! read is unknown, and only a page without a `data` array fails.

use std::collections::BTreeMap;
use std::sync::Arc;

use futures_util::StreamExt;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::port::{ModelError, ModelResult};
use crate::protocol::ModelApiProtocol;
use crate::provider::ModelProvider;
use crate::streaming::{
    authenticated_request, build_client, read_remote_error, response_metadata, retry_establishment,
    retryable_status, wait_cancelled, Authentication,
};
use crate::ModelReasoningEffort;

mod entry;
mod shapes;

const MAX_CATALOG_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_CATALOG_ENTRIES: usize = 1_000;
const MAX_ANTHROPIC_PAGES: usize = 10;

/// One model of a listing. Every field but the id is what the Provider
/// reports, or unknown (`None`, empty) when it reports nothing usable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogEntry {
    pub id: String,
    pub display_name: Option<String>,
    pub default_reasoning_effort: Option<ModelReasoningEffort>,
    pub supported_reasoning_efforts: Vec<ModelReasoningEffort>,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    /// Whether the model reads images.
    pub image_input: Option<bool>,
    /// Whether the model takes a reasoning parameter; reported levels mean
    /// it does.
    pub reasoning: Option<bool>,
}

/// A Provider's listing: its entries, sorted by name, and how many entries
/// it sent that had no usable id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelCatalogListing {
    pub entries: Vec<ModelCatalogEntry>,
    pub skipped: usize,
}

#[derive(Clone)]
pub struct ModelCatalogClient {
    provider: Arc<ModelProvider>,
    client: reqwest::Client,
}

impl ModelCatalogClient {
    pub fn new(provider: ModelProvider) -> ModelResult<Self> {
        let client = build_client(&provider)?;
        Ok(Self {
            provider: Arc::new(provider),
            client,
        })
    }

    pub async fn list(
        &self,
        protocol: ModelApiProtocol,
        cancellation: CancellationToken,
    ) -> ModelResult<ModelCatalogListing> {
        let authentication = match protocol {
            ModelApiProtocol::Responses | ModelApiProtocol::ChatCompletions => {
                Authentication::Bearer
            }
            ModelApiProtocol::AnthropicMessages => Authentication::Anthropic,
        };
        let mut endpoint = self.provider.endpoint().models_url()?;
        if protocol == ModelApiProtocol::AnthropicMessages {
            set_query_parameter(&mut endpoint, "limit", "100");
        }
        let mut entries = BTreeMap::<String, ModelCatalogEntry>::new();
        let mut skipped = 0usize;
        let mut pages = 0usize;
        loop {
            pages += 1;
            let value = self
                .fetch_page(endpoint.clone(), authentication, &cancellation)
                .await?;
            let data = value
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| ModelError::Protocol("model catalog has no data array".into()))?;
            for item in data {
                let Some(entry) = entry::decode_entry(item) else {
                    skipped += 1;
                    continue;
                };
                entries.entry(entry.id.clone()).or_insert(entry);
                if entries.len() > MAX_CATALOG_ENTRIES {
                    return Err(ModelError::Protocol(format!(
                        "model catalog exceeded {MAX_CATALOG_ENTRIES} entries"
                    )));
                }
            }
            if protocol != ModelApiProtocol::AnthropicMessages
                || !value
                    .get("has_more")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            {
                break;
            }
            if pages >= MAX_ANTHROPIC_PAGES {
                return Err(ModelError::Protocol(
                    "Anthropic model catalog exceeded the page limit".into(),
                ));
            }
            // `last_id` is nullable: without it the listing is what was read.
            let Some(after_id) = value
                .get("last_id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            else {
                break;
            };
            set_query_parameter(&mut endpoint, "after_id", after_id);
        }
        if skipped > 0 {
            log::warn!("model.catalog.entries_skipped count={skipped} reason=no_usable_id");
        }
        let mut entries = entries.into_values().collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            left.display_name
                .as_deref()
                .unwrap_or(&left.id)
                .to_lowercase()
                .cmp(
                    &right
                        .display_name
                        .as_deref()
                        .unwrap_or(&right.id)
                        .to_lowercase(),
                )
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(ModelCatalogListing { entries, skipped })
    }

    async fn fetch_page(
        &self,
        endpoint: reqwest::Url,
        authentication: Authentication,
        cancellation: &CancellationToken,
    ) -> ModelResult<Value> {
        let mut attempts = 0;
        loop {
            if cancellation.is_cancelled() {
                return Err(ModelError::Cancelled);
            }
            if attempts >= self.provider.retry_policy().maximum_attempts {
                return Err(ModelError::RequestRetriesExhausted {
                    attempts,
                    status: None,
                    code: None,
                    message: "model catalog exhausted establishment attempts".into(),
                    request_id: None,
                });
            }
            attempts += 1;
            let request = authenticated_request(
                &self.client,
                &self.provider,
                Method::GET,
                endpoint.clone(),
                authentication,
            )?;
            let sent = tokio::select! {
                result = tokio::time::timeout(
                    self.provider.timeouts().request_establishment,
                    request.send(),
                ) => match result {
                    Ok(sent) => sent,
                    Err(_) => {
                        if attempts < self.provider.retry_policy().maximum_attempts
                            && retry_establishment(&self.provider, cancellation, attempts).await?
                        {
                            continue;
                        }
                        return Err(ModelError::RequestRetriesExhausted {
                            attempts,
                            status: None,
                            code: None,
                            message: "model catalog request timed out".into(),
                            request_id: None,
                        });
                    }
                },
                _ = wait_cancelled(cancellation) => return Err(ModelError::Cancelled),
            };
            match sent {
                Ok(response) if response.status().is_success() => {
                    let body = read_bounded_body(response, &self.provider, cancellation).await?;
                    return serde_json::from_slice(&body).map_err(|_| {
                        ModelError::Protocol("model catalog response is not valid JSON".into())
                    });
                }
                Ok(response) => {
                    let status = response.status();
                    let retryable = retryable_status(status);
                    let metadata = response_metadata(response.headers(), &self.provider);
                    if retryable
                        && attempts < self.provider.retry_policy().maximum_attempts
                        && retry_establishment(&self.provider, cancellation, attempts).await?
                    {
                        continue;
                    }
                    let (code, message) =
                        read_remote_error(response, &self.provider, cancellation).await?;
                    return Err(if retryable {
                        ModelError::RequestRetriesExhausted {
                            attempts,
                            status: Some(status.as_u16()),
                            code,
                            message,
                            request_id: metadata.request_id,
                        }
                    } else {
                        ModelError::Remote {
                            status: Some(status.as_u16()),
                            code,
                            message,
                            request_id: metadata.request_id,
                        }
                    });
                }
                Err(error) => {
                    let retryable = error.is_connect() || error.is_timeout() || error.is_request();
                    if retryable
                        && retry_establishment(&self.provider, cancellation, attempts).await?
                    {
                        continue;
                    }
                    let message = if error.is_connect() {
                        "model catalog connection failed"
                    } else if error.is_timeout() {
                        "model catalog request timed out"
                    } else {
                        "model catalog request could not be sent"
                    };
                    return Err(if retryable {
                        ModelError::RequestRetriesExhausted {
                            attempts,
                            status: None,
                            code: None,
                            message: message.into(),
                            request_id: None,
                        }
                    } else {
                        ModelError::RequestFailed {
                            attempts,
                            message: message.into(),
                        }
                    });
                }
            }
        }
    }
}

async fn read_bounded_body(
    response: reqwest::Response,
    provider: &ModelProvider,
    cancellation: &CancellationToken,
) -> ModelResult<Vec<u8>> {
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    loop {
        let next = tokio::select! {
            next = tokio::time::timeout(provider.timeouts().stream_idle, stream.next()) => {
                next.map_err(|_| ModelError::IdleTimeout {
                    timeout: provider.timeouts().stream_idle,
                })?
            }
            _ = wait_cancelled(cancellation) => return Err(ModelError::Cancelled),
        };
        match next {
            Some(Ok(chunk)) => {
                if body.len().saturating_add(chunk.len()) > MAX_CATALOG_BODY_BYTES {
                    return Err(ModelError::Protocol(format!(
                        "model catalog response exceeded {MAX_CATALOG_BODY_BYTES} bytes"
                    )));
                }
                body.extend_from_slice(&chunk);
            }
            Some(Err(_)) => {
                return Err(ModelError::StreamInterrupted(
                    "model catalog response body failed".into(),
                ));
            }
            None => return Ok(body),
        }
    }
}

fn set_query_parameter(endpoint: &mut reqwest::Url, name: &str, value: &str) {
    let retained = endpoint
        .query_pairs()
        .filter(|(current, _)| current != name)
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    endpoint.set_query(None);
    let mut query = endpoint.query_pairs_mut();
    for (name, value) in retained {
        query.append_pair(&name, &value);
    }
    query.append_pair(name, value);
}
