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

use super::super::cache::CachedPage;
use super::*;
use crate::{WebFetchBackend, WebFetchError, WebFetchRequest, MAX_WEB_FETCH_CONTENT_BYTES};
use futures_util::{stream, StreamExt};
use std::collections::VecDeque;

impl Run<'_> {
    pub(super) async fn fetch_candidates(&mut self, candidates: Vec<Candidate>, round: u8) {
        let mut queue: VecDeque<_> = candidates
            .into_iter()
            .filter(|c| !self.bodies.iter().any(|b| b.candidate.key == c.key))
            .collect();
        while !queue.is_empty()
            && self.bodies.len() < self.depth * 2
            && !self.budget.fetch_exhausted()
        {
            let jobs: Vec<_> = (0..(self.depth * 2 - self.bodies.len()).min(3))
                .filter_map(|_| queue.pop_front())
                .collect();
            let service = &self.budgeted;
            let cancel = &self.cancellation;
            let age = self.intent.freshness.cache_age();
            let mut running = stream::iter(jobs.into_iter().map(|candidate| async move {
                let result = fetch_one(service, &candidate, age, round, cancel).await;
                (candidate, result)
            }))
            .buffered(3);
            while let Some((candidate, result)) = running.next().await {
                match result {
                    Ok(body) => self.bodies.push(body),
                    Err(WebFetchError::Cancelled | WebFetchError::BudgetExhausted) => return,
                    Err(error) => self.report.notes.push(format!(
                        "fetch failed for {}: {error}",
                        candidate.source.url
                    )),
                }
            }
        }
    }
}
async fn fetch_one(
    service: &WebSearchService,
    candidate: &Candidate,
    cache_age: i64,
    round: u8,
    cancel: &CancellationToken,
) -> Result<Body, WebFetchError> {
    if service.blocked(&candidate.source.url) {
        return Err(WebFetchError::BlockedTarget);
    }
    if let Some(page) = service
        .inner
        .documents
        .get(&candidate.source.url, cache_age)
    {
        return Ok(Body {
            updated_at: None,
            candidate: candidate.clone(),
            content: page.content,
            fetched_at: page.fetched_at,
            tier: "cache".into(),
            cache: true,
            truncated: false,
            round,
            selected: None,
            folded_into: None,
        });
    }
    let url = hanafubuki_agent_foundation::url_policy::UrlPolicy::PAGE
        .parse(&candidate.source.url)
        .map_err(|_| WebFetchError::BlockedTarget)?;
    let mut request = WebFetchRequest {
        url,
        maximum_content_bytes: MAX_WEB_FETCH_CONTENT_BYTES,
        from_offset: 0,
        content_generation: None,
    };
    let mut content = String::new();
    let (backend, truncated, updated_at) = loop {
        let page = service.fetch_page(request.clone(), cancel.clone()).await?;
        content.push_str(&page.content);
        let Some(next) = page.next_offset else {
            break (page.backend, page.document_truncated, page.updated_at);
        };
        request.from_offset = next;
        request.content_generation = Some(page.content_generation);
    };
    if content.chars().count() < 300 {
        return Err(WebFetchError::EmptyContent);
    }
    let fetched_at = chrono::Utc::now().timestamp();
    service.inner.documents.put(&CachedPage {
        url: candidate.source.url.clone(),
        title: candidate.source.title.clone(),
        content: content.clone(),
        fetched_at,
    });
    let tier = match backend {
        WebFetchBackend::DirectHtml | WebFetchBackend::DirectText => "direct",
        WebFetchBackend::JinaReader => "jina",
        WebFetchBackend::Exa => "exa",
        WebFetchBackend::Firecrawl => "firecrawl",
        WebFetchBackend::Crawl4ai => "crawl4ai",
        WebFetchBackend::Browser => "browser",
    };
    Ok(Body {
        updated_at,
        candidate: candidate.clone(),
        content,
        fetched_at,
        tier: tier.into(),
        cache: false,
        truncated,
        round,
        selected: None,
        folded_into: None,
    })
}
