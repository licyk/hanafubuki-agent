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

// Derived from dsh-free-search 6ab5168 (MIT), SearXNG and Degoog instance failover.

use super::wire;
use crate::service::{normalize_results, WebSearchService};
use crate::{WebSearchError, WebSearchProvider as E, WebSearchRequest, WebSearchResponse};
use hanafubuki_agent_foundation::cancellation::CancellationToken;

const PUBLIC_SEARXNG: &[&str] = &[
    "https://opnxng.com",
    "https://priv.au",
    "https://searx.be",
    "https://searx.tiekoetter.com",
    "https://search.inetol.net",
    "https://paulgo.io",
];

impl WebSearchService {
    pub(in crate::service) async fn search_instances(
        &self,
        engine: E,
        r: &WebSearchRequest,
        key: &str,
        cancellation: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        let options = self
            .inner
            .config
            .search
            .engines
            .get(&engine)
            .cloned()
            .unwrap_or_default();
        let mut instances = options.instances;
        if let Some(endpoint) = options.endpoint {
            instances.insert(0, endpoint);
        }
        if engine == E::Searxng {
            if let Some(base) = &self.inner.searxng_url {
                instances.insert(0, base.to_string());
            }
            if instances.is_empty() {
                instances = PUBLIC_SEARXNG.iter().map(|url| (*url).into()).collect();
            }
        }
        let mut last = WebSearchError::InvalidConfiguration("no search instances configured");
        let mut empty = None;
        for base in instances {
            let mut url = super::super::http::parse_search_endpoint(&base)?;
            let result = if engine == E::Searxng {
                self.search_searxng(&url, r, key, cancellation).await
            } else {
                let path = url.path().trim_end_matches('/');
                url.set_path(&format!(
                    "{path}{}",
                    if path.ends_with("/api") {
                        "/search"
                    } else {
                        "/api/search"
                    }
                ));
                url.query_pairs_mut()
                    .append_pair("q", &r.query)
                    .append_pair("type", "web")
                    .append_pair("lang", &self.inner.config.search.language);
                if let Some(time) = r.time_range {
                    url.query_pairs_mut().append_pair("time", time.as_searxng());
                }
                let mut builder = self.inner.client.get(url);
                if !key.is_empty() {
                    builder = builder.bearer_auth(key);
                }
                self.send_bounded(builder, cancellation)
                    .await
                    .and_then(|bytes| wire::decode(&bytes, engine))
                    .map(|results| WebSearchResponse {
                        query: r.query.clone(),
                        providers: vec![engine],
                        results: normalize_results(results, r.maximum_results),
                        answers: Vec::new(),
                        suggestions: Vec::new(),
                        partial_failures: Vec::new(),
                        cache_hit: false,
                        uncertainty: None,
                    })
            };
            match result {
                Ok(result) if !result.results.is_empty() => return Ok(result),
                Ok(result) => empty = Some(result),
                Err(WebSearchError::Cancelled) => return Err(WebSearchError::Cancelled),
                Err(error) => last = error,
            }
        }
        empty.ok_or(last)
    }
}
