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

//! Keyed extraction tiers from dsh-free-search (MIT). Targets keep the direct-fetch policy.
use super::*;
use serde::{Deserialize, Serialize};

impl WebPageFetcher {
    pub(super) fn remote_tiers(&self) -> Vec<WebFetchBackend> {
        let order = match self.options.prefer {
            options::FetchPreference::Exa => [WebFetchBackend::Exa, WebFetchBackend::Firecrawl],
            options::FetchPreference::Firecrawl => {
                [WebFetchBackend::Firecrawl, WebFetchBackend::Exa]
            }
        };
        order
            .into_iter()
            .chain([WebFetchBackend::Crawl4ai, WebFetchBackend::JinaReader])
            .filter(|b| match b {
                WebFetchBackend::Exa => !self.exa_keys.is_empty(),
                WebFetchBackend::Firecrawl => !self.firecrawl_keys.is_empty(),
                WebFetchBackend::Crawl4ai => !self.crawl4ai_key.is_empty(),
                WebFetchBackend::JinaReader => self.jina_reader_enabled,
                _ => false,
            })
            .collect()
    }
    pub(super) async fn fetch_remote(
        &self,
        backend: WebFetchBackend,
        target: Url,
        cancel: &CancellationToken,
    ) -> Result<FetchedDocument, WebFetchError> {
        self.resolve_public_target(&target, cancel).await?;
        if backend == WebFetchBackend::JinaReader {
            return self.fetch_jina(target, cancel).await;
        }
        let (endpoint, keys) = match backend {
            WebFetchBackend::Exa => (&self.options.exa_endpoint, self.exa_keys.as_slice()),
            WebFetchBackend::Firecrawl => (
                &self.options.firecrawl_endpoint,
                self.firecrawl_keys.as_slice(),
            ),
            WebFetchBackend::Crawl4ai => (
                &self.options.crawl4ai_endpoint,
                std::slice::from_ref(&self.crawl4ai_key),
            ),
            _ => return Err(WebFetchError::InvalidConfiguration("not a remote reader")),
        };
        let mut last = WebFetchError::RequestFailed;
        for key in keys.iter().take(1) {
            self.wait_for_rate_limit(cancel).await?;
            let request = self.jina_client.post(endpoint);
            let request = match backend {
                WebFetchBackend::Exa => request.bearer_auth(key).json(&ExaRequest {
                    ids: [target.as_str()],
                    text: true,
                }),
                WebFetchBackend::Firecrawl => request.bearer_auth(key).json(&FirecrawlRequest {
                    url: target.as_str(),
                    formats: ["markdown"],
                }),
                WebFetchBackend::Crawl4ai => request.header("X-API-Key", key).json(&CrawlRequest {
                    url: target.as_str(),
                }),
                _ => return Err(WebFetchError::InvalidConfiguration("not a remote reader")),
            };
            let response = tokio::select! {result=request.send()=>result.map_err(map_request_error)?,_ = wait_cancelled(cancel)=>return Err(WebFetchError::Cancelled)};
            if !response.status().is_success() {
                last = WebFetchError::HttpStatus(response.status().as_u16());
                continue;
            }
            let body = read_bounded(response, MAX_WEB_FETCH_READER_BODY_BYTES, cancel).await?;
            let text = extract(backend, &target, &body)?;
            if text.trim().chars().count() < 300 {
                return Err(WebFetchError::EmptyContent);
            }
            let (content, document_truncated) =
                truncate_utf8(text.trim(), MAX_WEB_FETCH_DOCUMENT_BYTES);
            return Ok(FetchedDocument {
                updated_at: None,
                final_url: target.to_string(),
                title: None,
                content,
                content_type: "text/markdown".into(),
                backend,
                fetched_bytes: body.len(),
                generation: String::new(),
                document_truncated,
                redirect_count: 0,
            });
        }
        Err(last)
    }
}
fn extract(backend: WebFetchBackend, target: &Url, body: &[u8]) -> Result<String, WebFetchError> {
    match backend {
        WebFetchBackend::Exa => {
            let response: ExaResponse =
                serde_json::from_slice(body).map_err(|_| WebFetchError::EmptyContent)?;
            if response
                .statuses
                .iter()
                .find(|s| s.id.as_deref() == Some(target.as_str()))
                .or(response.statuses.first())
                .is_some_and(|s| s.status != "success")
            {
                return Err(WebFetchError::RequestFailed);
            }
            response
                .results
                .iter()
                .find(|r| {
                    r.id.as_deref() == Some(target.as_str())
                        || r.url.as_deref() == Some(target.as_str())
                })
                .or(response.results.first())
                .map(|r| r.text.clone())
                .ok_or(WebFetchError::EmptyContent)
        }
        WebFetchBackend::Firecrawl => serde_json::from_slice::<FirecrawlResponse>(body)
            .ok()
            .and_then(|r| r.data.markdown)
            .ok_or(WebFetchError::EmptyContent),
        WebFetchBackend::Crawl4ai => {
            let response: CrawlResponse =
                serde_json::from_slice(body).map_err(|_| WebFetchError::EmptyContent)?;
            response
                .markdown
                .or_else(|| {
                    response.html.and_then(|html| {
                        extract_html(html.as_bytes(), target, MAX_WEB_FETCH_DOCUMENT_BYTES)
                            .ok()
                            .map(|(_, text, _)| text)
                    })
                })
                .ok_or(WebFetchError::EmptyContent)
        }
        _ => Err(WebFetchError::InvalidConfiguration(
            "not an extraction reader",
        )),
    }
}
#[derive(Serialize)]
struct ExaRequest<'a> {
    ids: [&'a str; 1],
    text: bool,
}
#[derive(Serialize)]
struct FirecrawlRequest<'a> {
    url: &'a str,
    formats: [&'static str; 1],
}
#[derive(Serialize)]
struct CrawlRequest<'a> {
    url: &'a str,
}
#[derive(Deserialize)]
struct ExaResponse {
    #[serde(default)]
    results: Vec<ExaDocument>,
    #[serde(default)]
    statuses: Vec<ExaStatus>,
}
#[derive(Deserialize)]
struct ExaDocument {
    id: Option<String>,
    url: Option<String>,
    text: String,
}
#[derive(Deserialize)]
struct ExaStatus {
    id: Option<String>,
    status: String,
}
#[derive(Deserialize)]
struct FirecrawlResponse {
    data: CrawlResponse,
}
#[derive(Deserialize)]
struct CrawlResponse {
    markdown: Option<String>,
    html: Option<String>,
}

impl WebPageFetcher {
    pub(super) async fn document_with_fallback(
        &self,
        target: Url,
        cancel: &CancellationToken,
        budget: Option<&crate::service::research::SearchBudget>,
        runner: Option<&dyn crate::LocalWebRunner>,
    ) -> Result<FetchedDocument, WebFetchError> {
        if let Some(budget) = budget {
            budget.charge_fetch()?;
        }
        let direct = tokio::select! {result=tokio::time::timeout(self.direct_timeout,self.fetch_direct(target.clone(),cancel))=>result.unwrap_or(Err(WebFetchError::Timeout)).and_then(project_direct_document),_ = wait_cancelled(cancel)=>return Err(WebFetchError::Cancelled)};
        let tiers = self.remote_tiers();
        let mut short = None;
        let mut last = match direct {
            Ok(document)
                if document.content.chars().count() >= 300
                    || (tiers.is_empty() && runner.is_none()) =>
            {
                return Ok(document)
            }
            Ok(document) => {
                short = Some(document);
                WebFetchError::EmptyContent
            }
            Err(error) if error.may_use_reader_fallback() => error,
            Err(error) => return Err(error),
        };
        if let Some(runner) = runner
            .filter(|r| self.options.browser_enabled && r.available(crate::LocalWebKind::Browser))
        {
            if let Some(budget) = budget {
                budget.charge_fetch()?;
            }
            match self.fetch_browser(target.clone(), runner, cancel).await {
                Ok(document) => return Ok(document),
                Err(WebFetchError::Cancelled) => return Err(WebFetchError::Cancelled),
                Err(error) => last = error,
            }
        }
        for backend in tiers {
            if let Some(budget) = budget {
                if budget.charge_reader(backend).is_err() {
                    continue;
                }
                budget.charge_fetch()?;
            }
            let result = tokio::select! {result=tokio::time::timeout(self.reader_timeout,self.fetch_remote(backend,target.clone(),cancel))=>result.unwrap_or(Err(WebFetchError::Timeout)),_ = wait_cancelled(cancel)=>return Err(WebFetchError::Cancelled)};
            match result {
                Ok(document) => return Ok(document),
                Err(error @ (WebFetchError::Cancelled | WebFetchError::BlockedTarget)) => {
                    return Err(error)
                }
                Err(error) => last = error,
            }
        }
        short.ok_or(last)
    }
}
