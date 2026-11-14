// Ported from dsh-free-search 6ab5168 (MIT); adapted to hanafubuki-agent.
//
// The MIT License (MIT)
//
// Copyright (c) 2026 dsh-free-search contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

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
        self.send_bounded_limit(request, cancellation, MAX_WEB_SEARCH_BODY_BYTES)
            .await
    }

    pub(super) async fn send_bounded_limit(
        &self,
        request: RequestBuilder,
        cancellation: &CancellationToken,
        maximum_bytes: usize,
    ) -> Result<Vec<u8>, WebSearchError> {
        let _permit = tokio::select! {
            permit = self.inner.concurrency.acquire() => permit.map_err(|_| WebSearchError::Cancelled)?,
            _ = wait_cancelled(cancellation) => return Err(WebSearchError::Cancelled),
        };
        self.wait_for_rate_limit(cancellation).await?;
        if let Some(budget) = &self.search_budget {
            budget.charge()?;
        }
        let sent = tokio::select! {
            result = request.send() => result,
            _ = wait_cancelled(cancellation) => return Err(WebSearchError::Cancelled),
        }
        .map_err(map_reqwest_error)?;
        let status = sent.status();
        let retry_after = sent
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| {
                s.parse::<u64>()
                    .ok()
                    .map(|s| std::time::Duration::from_secs(s.clamp(5, 7 * 86400)))
                    .or_else(|| {
                        chrono::DateTime::parse_from_rfc2822(s).ok().map(|d| {
                            std::time::Duration::from_secs(
                                (d.timestamp() - chrono::Utc::now().timestamp()).clamp(5, 7 * 86400)
                                    as u64,
                            )
                        })
                    })
            });
        if sent
            .content_length()
            .is_some_and(|length| length > maximum_bytes as u64)
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
            if body.len().saturating_add(chunk.len()) > maximum_bytes {
                return Err(WebSearchError::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            let (class, delay) =
                crate::engines::backoff::classify(status.as_u16(), &String::from_utf8_lossy(&body));
            if let Some(delay) = retry_after.or(delay) {
                return Err(WebSearchError::ProviderBackoff { class, delay });
            }
            if matches!(
                class,
                crate::FailureClass::Quota
                    | crate::FailureClass::BotWall
                    | crate::FailureClass::Region
            ) {
                return Err(WebSearchError::ProviderFailure(class));
            }
            return Err(WebSearchError::HttpStatus(status.as_u16()));
        }
        Ok(body)
    }

    pub(super) async fn wait_for_rate_limit(
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
