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

use crate::{WebSearchError, WebSearchResult, WebSearchService};
use futures_util::{stream, StreamExt};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use reqwest::Url;
use std::time::Duration;

impl WebSearchService {
    pub(super) async fn resolve_signed_links(
        &self,
        results: &mut [WebSearchResult],
        cancellation: &CancellationToken,
    ) -> Result<(), WebSearchError> {
        let pending = results
            .iter()
            .enumerate()
            .filter_map(|(index, result)| {
                let url = Url::parse(&result.url).ok()?;
                signed_link(&url).then_some((index, url))
            })
            .take(6)
            .collect::<Vec<_>>();
        let mut running = stream::iter(pending.into_iter().map(|(index, url)| async move {
            let work = async {
                let _permit = self.inner.concurrency.acquire().await.ok()?;
                self.wait_for_rate_limit(cancellation).await.ok()?;
                if let Some(budget) = &self.search_budget {
                    budget.charge().ok()?;
                }
                // The shared client disables redirects: the signed token is used once,
                // and the destination is returned without fetching it.
                let response = self.inner.client.get(url.clone()).send().await.ok()?;
                let next = url
                    .join(response.headers().get("location")?.to_str().ok()?)
                    .ok()?;
                resolved_destination(next)
            };
            (
                index,
                tokio::time::timeout(Duration::from_millis(2500), work)
                    .await
                    .ok()
                    .flatten(),
            )
        }))
        .buffered(6);
        loop {
            let next = tokio::select! {
                next = running.next() => next,
                _ = super::super::http::wait_cancelled(cancellation) => return Err(WebSearchError::Cancelled),
            };
            let Some((index, resolved)) = next else {
                break;
            };
            if let Some(url) = resolved {
                results[index].url = url;
            }
        }
        Ok(())
    }
}

fn signed_link(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.path() == "/link"
        && matches!(
            url.host_str(),
            Some("weixin.sogou.com" | "www.sogou.com" | "sogou.com" | "www.so.com" | "so.com")
        )
}
fn resolved_destination(url: Url) -> Option<String> {
    (matches!(url.scheme(), "http" | "https")
        && !signed_link(&url)
        && !url.path().to_ascii_lowercase().contains("antispider")
        && !url
            .host_str()
            .unwrap_or("")
            .to_ascii_lowercase()
            .contains("antispider"))
    .then(|| url.to_string())
}

#[cfg(test)]
mod tests;
