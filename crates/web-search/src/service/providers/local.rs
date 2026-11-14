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

//! Local CLI result projection adapted from dsh-free-search agy.js and X search.
use super::wire::{Candidate, SearchResponse};
use crate::{
    LocalWebRequest, SourceResponse, WebSearchError, WebSearchProvider as E, WebSearchRequest,
    WebSearchResponse, WebSearchService,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde::Deserialize;

impl WebSearchService {
    pub(crate) async fn search_local(
        &self,
        request: &WebSearchRequest,
        grok: bool,
        cancel: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        let runner = self
            .local_runner
            .as_ref()
            .ok_or(WebSearchError::InvalidConfiguration(
                "local search helper requires host authorization",
            ))?;
        if let Some(budget) = &self.search_budget {
            budget.charge()?;
        }
        let command = if grok {
            LocalWebRequest::Grok {
                query: request.query.clone(),
                maximum: request.maximum_results,
            }
        } else {
            LocalWebRequest::Agy {
                query: request.query.clone(),
                maximum: request.maximum_results,
                model: self
                    .inner
                    .config
                    .search
                    .engines
                    .get(&E::Agy)
                    .and_then(|c| c.model.clone())
                    .or_else(|| std::env::var("AGY_MODEL").ok())
                    .unwrap_or_else(|| "gemini-3.8-flash-low".into()),
            }
        };
        let stdout = runner.run(command, cancel.clone()).await?;
        let mut results = parse(&stdout)?;
        if grok {
            let posts: Vec<_> = results
                .iter()
                .filter(|r| {
                    url::Url::parse(&r.url).ok().is_some_and(|u| {
                        matches!(
                            u.host_str(),
                            Some(
                                "x.com"
                                    | "www.x.com"
                                    | "mobile.x.com"
                                    | "twitter.com"
                                    | "www.twitter.com"
                            )
                        ) && u.path().contains("/status/")
                    })
                })
                .cloned()
                .collect();
            if !posts.is_empty() {
                results = posts;
            }
            for item in &mut results {
                item.source = "x".into();
                item.engines = vec!["x".into()];
            }
        }
        results.retain(|r| !self.blocked(&r.url));
        let mut response = super::super::normalize::empty_response(request, Vec::new());
        response.providers = vec![E::Agy];
        response.results = super::super::normalize_results(results, request.maximum_results);
        Ok(response)
    }
    pub(crate) async fn search_x(
        &self,
        query: &str,
        maximum: usize,
        cancel: &CancellationToken,
    ) -> Result<SourceResponse, WebSearchError> {
        let request = WebSearchRequest {
            query: query.into(),
            maximum_results: maximum,
            provider: E::Agy,
            engines: Vec::new(),
            category: Default::default(),
            time_range: None,
            region: None,
        };
        Ok(SourceResponse::new(
            self.search_local(&request, true, cancel).await?.results,
            maximum,
        ))
    }
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Envelope {
    status: String,
    #[serde(alias = "structured_output")]
    structured_output: Option<SearchResponse>,
    response: String,
    text: String,
    items: Vec<Candidate>,
    results: Vec<Candidate>,
}
fn parse(text: &str) -> Result<Vec<crate::WebSearchResult>, WebSearchError> {
    let spans = objects(text);
    let envelope = serde_json::from_str::<Envelope>(text)
        .ok()
        .or_else(|| {
            spans
                .iter()
                .rev()
                .find_map(|s| serde_json::from_str::<Envelope>(s).ok())
        })
        .ok_or(WebSearchError::InvalidResponse)?;
    if !envelope.status.is_empty() && !envelope.status.eq_ignore_ascii_case("success") {
        return Err(WebSearchError::RequestFailed);
    }
    if let Some(response) = envelope.structured_output {
        return response.results(E::Agy);
    }
    let found: Vec<_> = envelope
        .items
        .into_iter()
        .chain(envelope.results)
        .filter_map(|c| match c {
            Candidate::Source(s) => s.into_result(E::Agy),
            _ => None,
        })
        .collect();
    if !found.is_empty() {
        return Ok(found);
    }
    for nested in [&envelope.response, &envelope.text] {
        for span in objects(nested).into_iter().rev() {
            if let Ok(response) = serde_json::from_str::<SearchResponse>(span) {
                if let Ok(items) = response.results(E::Agy) {
                    if !items.is_empty() {
                        return Ok(items);
                    }
                }
            }
        }
    }
    Err(WebSearchError::NoResults)
}
fn objects(text: &str) -> Vec<&str> {
    let (mut depth, mut start, mut quoted, mut escape) = (0usize, 0, false, false);
    let mut spans = Vec::new();
    for (index, c) in text.char_indices() {
        if quoted {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                quoted = false;
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            '{' => {
                if depth == 0 {
                    start = index;
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    spans.push(&text[start..index + 1]);
                }
            }
            _ => {}
        }
    }
    spans
}

#[cfg(test)]
mod tests;
