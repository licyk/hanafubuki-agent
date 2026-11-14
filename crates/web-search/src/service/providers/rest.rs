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

// Derived from dsh-free-search 6ab5168 (MIT), lib/engines REST adapters.

use super::{api_request as body, wire};
use crate::service::{normalize_results, WebSearchService};
use crate::{
    SafeSearch, WebSearchError, WebSearchProvider as E, WebSearchRequest, WebSearchResponse,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use reqwest::RequestBuilder;

impl WebSearchService {
    pub(in crate::service) async fn search_rest(
        &self,
        engine: E,
        request: &WebSearchRequest,
        key: &str,
        cancellation: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        if engine == E::Google {
            return self.search_google(request, key, cancellation).await;
        }
        let (endpoint, verb) = match engine {
            E::Anysearch => ("https://api.anysearch.com/v1/search", "POST"),
            E::Exa => ("https://api.exa.ai/search", "POST"),
            E::Tavily => ("https://api.tavily.com/search", "POST"),
            E::Firecrawl => ("https://api.firecrawl.dev/v2/search", "POST"),
            E::Keenable => ("https://api.keenable.ai/v1/search", "POST"),
            E::Parallel => ("https://api.parallel.ai/v1/search", "POST"),
            E::Serpbase => ("https://api.serpbase.dev/google/search", "POST"),
            E::Serply => ("https://api.serply.io/v1/search", "GET"),
            E::Outcrawl => ("https://api.outcrawl.ai/v1/search", "POST"),
            E::Google => ("https://www.googleapis.com/customsearch/v1", "GET"),
            E::You if key.is_empty() => ("https://api.you.com/v1/agents/search", "GET"),
            E::You => ("https://ydc-index.io/v1/search", "GET"),
            E::Tinyfish => ("https://api.search.tinyfish.ai", "GET"),
            E::Jina => ("https://s.jina.ai/", "GET"),
            _ => {
                return Err(WebSearchError::InvalidConfiguration(
                    "not a REST search engine",
                ))
            }
        };
        let endpoint = self.endpoint(engine, endpoint)?;
        let builder = if verb == "POST" {
            self.inner.client.post(endpoint)
        } else {
            self.inner.client.get(endpoint)
        };
        let builder = builder.header("accept", "application/json");
        let builder = if !key.is_empty() && engine != E::Google {
            if matches!(
                engine,
                E::Keenable | E::Parallel | E::Serpbase | E::Serply | E::You | E::Tinyfish
            ) {
                builder.header("x-api-key", key)
            } else {
                builder.bearer_auth(key)
            }
        } else if engine == E::Tavily {
            builder.header("x-tavily-access-mode", "keyless")
        } else {
            builder
        };
        let builder = self.rest_body(engine, request, key, builder)?;
        let bytes = self
            .send_bounded(builder, cancellation)
            .await
            .map_err(|error| {
                if engine == E::Firecrawl
                    && key.is_empty()
                    && matches!(error, WebSearchError::HttpStatus(403))
                {
                    WebSearchError::ProviderBackoff {
                        class: crate::FailureClass::Auth,
                        delay: std::time::Duration::from_secs(6 * 3600),
                    }
                } else {
                    error
                }
            })?;
        let results = normalize_results(wire::decode(&bytes, engine)?, request.maximum_results);
        Ok(WebSearchResponse {
            query: request.query.clone(),
            providers: vec![engine],
            results,
            answers: Vec::new(),
            suggestions: Vec::new(),
            partial_failures: Vec::new(),
            cache_hit: false,
            uncertainty: None,
        })
    }

    fn rest_body(
        &self,
        engine: E,
        r: &WebSearchRequest,
        key: &str,
        b: RequestBuilder,
    ) -> Result<RequestBuilder, WebSearchError> {
        let q = r.query.as_str();
        let n = r.maximum_results;
        let time = r.time_range.map(|t| t.as_searxng());
        let date = r.time_range.map(|t| t.start_date());
        let options = &self.inner.config.search;
        let configured = options.engines.get(&engine).cloned().unwrap_or_default();
        let lang = options.language.as_str();
        let gl = if lang == "zh" { "cn" } else { "us" };
        Ok(match engine {
            E::Anysearch => b.json(&body::Query {
                query: q,
                max_results: n,
            }),
            E::Exa => b.json(&body::Exa {
                query: q,
                search_type: "auto",
                num_results: n,
                contents: body::ExaContents {
                    highlights: body::ExaHighlights {
                        highlights_per_url: 1,
                    },
                },
                start_published_date: date,
            }),
            E::Tavily => b.json(&body::Tavily {
                query: q,
                max_results: n.min(20),
                search_depth: "basic",
                time_range: time,
            }),
            E::Firecrawl => b.json(&body::Firecrawl {
                query: q,
                limit: n.min(10),
                tbs: r.time_range.map(|t| match t {
                    crate::WebSearchTimeRange::After(date) => {
                        format!("cdr:1,cd_min:{}", date.format("%-m/%-d/%Y"))
                    }
                    _ => format!("qdr:{}", t.as_duckduckgo()),
                }),
            }),
            E::Keenable => b.json(&body::Keenable {
                query: q,
                mode: "realtime",
                published_after: date,
            }),
            E::Parallel => b.json(&body::Parallel {
                objective: q,
                search_queries: [q],
                mode: "fast",
                advanced_settings: body::ParallelSettings {
                    max_results: n.min(20),
                    source_policy: date.map(|after_date| body::SourcePolicy { after_date }),
                },
            }),
            E::Outcrawl => b.json(&body::LimitedQuery {
                query: q,
                limit: n.min(20),
            }),
            E::Serpbase => b.json(&body::Serpbase {
                q,
                hl: lang,
                gl,
                page: 1,
            }),
            E::Serply => b.query(&[
                ("q", q.to_string()),
                ("num", n.min(10).to_string()),
                ("hl", lang.into()),
                ("gl", gl.into()),
            ]),
            E::Google => {
                if configured.google_cse_id.is_empty() {
                    return Err(WebSearchError::InvalidConfiguration(
                        "Google needs googleCseId",
                    ));
                }
                let mut pairs = vec![
                    ("q", q.to_string()),
                    ("key", key.into()),
                    ("cx", configured.google_cse_id),
                    ("num", n.min(10).to_string()),
                    ("hl", lang.into()),
                ];
                if options.safe_search != SafeSearch::Off {
                    pairs.push(("safe", "active".into()));
                }
                if let Some(range) = r.time_range {
                    pairs.push(("dateRestrict", format!("d{}", range.days())));
                }
                b.query(&pairs)
            }
            E::You => b.query(&[("query", q.to_string()), ("count", n.min(20).to_string())]),
            E::Tinyfish => {
                let mut pairs = vec![("query", q.to_string())];
                if let Some(after) = date {
                    pairs.push(("after_date", after));
                }
                b.query(&pairs)
            }
            E::Jina => b.query(&[("q", q)]),
            _ => {
                return Err(WebSearchError::InvalidConfiguration(
                    "not a REST search engine",
                ))
            }
        })
    }

    pub(in crate::service) fn endpoint(
        &self,
        engine: E,
        default: &str,
    ) -> Result<reqwest::Url, WebSearchError> {
        let configured = self
            .inner
            .config
            .search
            .engines
            .get(&engine)
            .and_then(|c| c.endpoint.as_deref())
            .unwrap_or(default);
        super::super::http::parse_search_endpoint(configured)
    }
}

impl WebSearchService {
    async fn search_google(
        &self,
        request: &WebSearchRequest,
        key: &str,
        cancellation: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        let limit = request.maximum_results.clamp(1, 30);
        let mut results = Vec::new();
        for start in (1..=limit).step_by(10) {
            let mut page = request.clone();
            page.maximum_results = (limit - results.len()).min(10);
            let builder = self
                .inner
                .client
                .get(self.endpoint(E::Google, "https://www.googleapis.com/customsearch/v1")?)
                .query(&[("start", start)]);
            let bytes = self
                .send_bounded(
                    self.rest_body(E::Google, &page, key, builder)?,
                    cancellation,
                )
                .await?;
            let found = wire::decode(&bytes, E::Google)?;
            let count = found.len();
            results.extend(found);
            if count < page.maximum_results {
                break;
            }
        }
        let mut response = super::super::normalize::empty_response(request, Vec::new());
        response.providers = vec![E::Google];
        response.results = normalize_results(results, limit);
        Ok(response)
    }
}
