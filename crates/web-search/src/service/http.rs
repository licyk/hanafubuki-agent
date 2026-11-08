use super::{WebSearchService, CANCELLATION_POLL_INTERVAL};
use crate::model::{WebSearchError, MAX_WEB_SEARCH_BODY_BYTES};
use futures_util::StreamExt;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use reqwest::{RequestBuilder, Url};
use std::time::Instant;

impl WebSearchService {
    pub(super) async fn get_bounded(
        &self,
        url: Url,
        cancellation: &CancellationToken,
    ) -> Result<Vec<u8>, WebSearchError> {
        self.send_bounded(self.inner.client.get(url), cancellation)
            .await
    }

    pub(super) async fn send_bounded(
        &self,
        request: RequestBuilder,
        cancellation: &CancellationToken,
    ) -> Result<Vec<u8>, WebSearchError> {
        self.wait_for_rate_limit(cancellation).await?;
        let sent = tokio::select! {
            result = request.send() => result,
            _ = wait_cancelled(cancellation) => return Err(WebSearchError::Cancelled),
        }
        .map_err(map_reqwest_error)?;
        let status = sent.status();
        if !status.is_success() {
            return Err(WebSearchError::HttpStatus(status.as_u16()));
        }
        if sent
            .content_length()
            .is_some_and(|length| length > MAX_WEB_SEARCH_BODY_BYTES as u64)
        {
            return Err(WebSearchError::ResponseTooLarge);
        }
        let mut stream = sent.bytes_stream();
        let mut body = Vec::new();
        loop {
            let next = tokio::select! {
                next = stream.next() => next,
                _ = wait_cancelled(cancellation) => return Err(WebSearchError::Cancelled),
            };
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(map_reqwest_error)?;
            if body.len().saturating_add(chunk.len()) > MAX_WEB_SEARCH_BODY_BYTES {
                return Err(WebSearchError::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    async fn wait_for_rate_limit(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<(), WebSearchError> {
        let delay = {
            let mut next = self.inner.next_request_at.lock().await;
            let now = Instant::now();
            let scheduled = (*next).max(now);
            *next = scheduled + self.inner.config.minimum_request_interval;
            scheduled.saturating_duration_since(now)
        };
        if delay.is_zero() {
            return Ok(());
        }
        tokio::select! {
            _ = tokio::time::sleep(delay) => Ok(()),
            _ = wait_cancelled(cancellation) => Err(WebSearchError::Cancelled),
        }
    }
}

/// Parses a search endpoint: the settings validated a SearXNG address
/// (`normalize_search_endpoint`) and DuckDuckGo's is a constant, so the
/// URL policy is not checked again here.
pub(super) fn parse_search_endpoint(value: &str) -> Result<Url, WebSearchError> {
    let mut url = Url::parse(value)
        .map_err(|_| WebSearchError::InvalidConfiguration("search endpoint is not a URL"))?;
    if url.path().is_empty() {
        url.set_path("/");
    }
    Ok(url)
}

fn map_reqwest_error(error: reqwest::Error) -> WebSearchError {
    if error.is_timeout() {
        WebSearchError::Timeout
    } else {
        WebSearchError::RequestFailed
    }
}

pub(super) async fn wait_cancelled(cancellation: &CancellationToken) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(CANCELLATION_POLL_INTERVAL).await;
    }
}
