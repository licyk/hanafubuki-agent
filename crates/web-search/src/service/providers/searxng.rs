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

use super::super::*;

impl WebSearchService {
    pub(in crate::service) async fn search_searxng(
        &self,
        base: &Url,
        request: &WebSearchRequest,
        key: &str,
        cancellation: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        let mut url = base.clone();
        let base_path = base.path().trim_end_matches('/');
        url.set_path(&format!("{base_path}/search"));
        {
            let mut pairs = url.query_pairs_mut();
            pairs
                .append_pair("q", &request.query)
                .append_pair("format", "json")
                .append_pair("categories", request.category.as_searxng())
                .append_pair("pageno", "1");
            if let Some(time_range) = request.time_range {
                pairs.append_pair("time_range", time_range.as_searxng());
            }
        }
        let mut builder = self.inner.client.get(url);
        if !key.is_empty() {
            builder = builder.bearer_auth(key);
        }
        let body = self.send_bounded(builder, cancellation).await?;
        let decoded: SearxResponse =
            serde_json::from_slice(&body).map_err(|_| WebSearchError::InvalidResponse)?;
        let results = normalize_results(
            decoded.results.into_iter().map(|result| WebSearchResult {
                title: result.title,
                url: result.url,
                snippet: result.content.unwrap_or_default(),
                source: result
                    .engines
                    .as_ref()
                    .and_then(|engines| engines.first())
                    .cloned()
                    .or(result.engine)
                    .unwrap_or_else(|| "searxng".to_string()),
                engines: result
                    .engines
                    .unwrap_or_else(|| vec!["searxng".to_string()]),
                published_at: result.published_date,
            }),
            request.maximum_results,
        );
        Ok(WebSearchResponse {
            query: request.query.clone(),
            providers: vec![WebSearchProvider::Searxng],
            results,
            answers: bounded_strings(
                decoded.answers.unwrap_or_default(),
                MAX_WEB_SEARCH_ANSWERS,
                MAX_WEB_SEARCH_ANSWER_BYTES,
            ),
            suggestions: bounded_strings(
                decoded.suggestions.unwrap_or_default(),
                MAX_WEB_SEARCH_SUGGESTIONS,
                MAX_WEB_SEARCH_TITLE_BYTES,
            ),
            partial_failures: Vec::new(),
            cache_hit: false,
            uncertainty: None,
        })
    }
}

#[derive(Deserialize)]
struct SearxResponse {
    #[serde(default)]
    results: Vec<SearxResult>,
    answers: Option<Vec<String>>,
    suggestions: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct SearxResult {
    title: String,
    url: String,
    content: Option<String>,
    engine: Option<String>,
    engines: Option<Vec<String>>,
    #[serde(rename = "publishedDate", alias = "published_date")]
    published_date: Option<String>,
}
