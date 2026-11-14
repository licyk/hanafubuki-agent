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

// Derived from dsh-free-search 6ab5168 (MIT), single and multi search routing.

use super::{http::wait_cancelled, normalize::empty_response, normalize_results, WebSearchService};
use crate::{
    FailureClass, WebSearchError, WebSearchFailure, WebSearchFailureCode, WebSearchProvider as E,
    WebSearchProviderPreference, WebSearchRequest, WebSearchResponse,
};
use futures_util::{stream, StreamExt};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use std::time::Instant;

impl WebSearchService {
    pub(super) async fn execute(
        &self,
        request: &WebSearchRequest,
        cancellation: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        let options = &self.inner.config.search;
        let preferred = match self.inner.config.provider_preference {
            WebSearchProviderPreference::Duckduckgo => E::Duckduckgo,
            WebSearchProviderPreference::Searxng => E::Searxng,
            WebSearchProviderPreference::Auto if request.provider == E::Auto => options.provider,
            WebSearchProviderPreference::Auto => request.provider,
        };
        if preferred != E::Auto && preferred != E::Multi && !options.enabled(preferred) {
            return Err(WebSearchError::InvalidConfiguration(
                "requested search engine is disabled",
            ));
        }
        let mut chain = options.route(&request.query, request.time_range.is_some());
        if preferred == E::Auto && self.inner.searxng_url.is_some() {
            chain.retain(|e| *e != E::Searxng);
            if options.enabled(E::Searxng) {
                chain.insert(0, E::Searxng);
            }
        } else if !matches!(preferred, E::Auto | E::Multi) {
            chain.retain(|e| *e != preferred);
            chain.insert(0, preferred);
        }
        let mut response = empty_response(request, Vec::new());
        if self.inner.config.provider_preference != WebSearchProviderPreference::Auto {
            chain.truncate(1);
        }
        let mut successes = 0;
        if preferred == E::Multi {
            let mut pool = if request.engines.is_empty() {
                options.language_pool(&request.query)
            } else {
                let mut pool = Vec::new();
                for engine in &request.engines {
                    if options.enabled(*engine) && !pool.contains(engine) {
                        pool.push(*engine);
                    }
                }
                pool
            };
            if options.multi_max_engines > 0 {
                pool.truncate(options.multi_max_engines);
            }
            let mut running = stream::iter(pool.iter().copied().map(|engine| async move {
                (
                    engine,
                    self.fetch_provider(engine, request, cancellation).await,
                )
            }))
            .buffered(self.inner.config.maximum_concurrent_requests);
            while let Some((engine, result)) = running.next().await {
                if self.merge_engine(&mut response, engine, result)? {
                    successes += 1;
                }
            }
            chain.retain(|e| !pool.contains(e));
        }
        let target = if preferred == E::Multi {
            options.multi_min_sources.max(1)
        } else {
            1
        };
        for engine in chain {
            if successes >= target {
                break;
            }
            let result = self.fetch_provider(engine, request, cancellation).await;
            if self.merge_engine(&mut response, engine, result)? {
                successes += 1;
            }
        }
        response.results = normalize_results(
            super::ranking::merge_rank(response.results, request.time_range.is_some()),
            request.maximum_results,
        );
        response.answers = super::normalize::bounded_strings(
            response.answers,
            crate::model::MAX_WEB_SEARCH_ANSWERS,
            crate::model::MAX_WEB_SEARCH_ANSWER_BYTES,
        );
        response.suggestions = super::normalize::bounded_strings(
            response.suggestions,
            crate::model::MAX_WEB_SEARCH_SUGGESTIONS,
            crate::model::MAX_WEB_SEARCH_TITLE_BYTES,
        );
        response.uncertainty = Some(super::quality::assess(
            &response,
            request.time_range.is_some(),
            options.language == "zh",
        ));
        Ok(response)
    }

    fn merge_engine(
        &self,
        response: &mut WebSearchResponse,
        engine: E,
        result: Result<WebSearchResponse, WebSearchError>,
    ) -> Result<bool, WebSearchError> {
        response.providers.push(engine);
        match result {
            Ok(mut found) => {
                found.results.retain(|entry| !self.blocked(&entry.url));
                let nonempty = !found.results.is_empty();
                if !nonempty {
                    response.partial_failures.push(WebSearchFailure {
                        provider: engine,
                        code: WebSearchFailureCode::NoResults,
                    });
                }
                response.results.extend(found.results);
                response.answers.extend(found.answers);
                response.suggestions.extend(found.suggestions);
                Ok(nonempty)
            }
            Err(WebSearchError::Cancelled) => Err(WebSearchError::Cancelled),
            Err(error) => {
                if !matches!(error, WebSearchError::CoolingDown)
                    && !self
                        .inner
                        .config
                        .search
                        .fallback_on
                        .contains(&error.class())
                {
                    return Err(error);
                }
                response.partial_failures.push(WebSearchFailure {
                    provider: engine,
                    code: error.failure_code(),
                });
                Ok(false)
            }
        }
    }

    pub(super) fn blocked(&self, address: &str) -> bool {
        let Ok(url) = reqwest::Url::parse(address) else {
            return true;
        };
        let Some(host) = url.host_str() else {
            return true;
        };
        self.inner
            .config
            .search
            .blocked_domains
            .iter()
            .any(|domain| {
                let domain = domain
                    .trim()
                    .trim_start_matches("*.")
                    .trim_start_matches('.')
                    .to_ascii_lowercase();
                !domain.is_empty() && (host == domain || host.ends_with(&format!(".{domain}")))
            })
    }

    pub(super) async fn fetch_provider(
        &self,
        engine: E,
        request: &WebSearchRequest,
        cancellation: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        if !self.inner.config.search.enabled(engine) {
            return Err(WebSearchError::InvalidConfiguration(
                "search engine is disabled",
            ));
        }
        let runtime = &self.inner.engines;
        let gate = runtime
            .gates
            .get(&engine)
            .ok_or(WebSearchError::InvalidConfiguration(
                "not a concrete search engine",
            ))?;
        let _permit = tokio::select! {
            permit = gate.acquire() => permit.map_err(|_| WebSearchError::Cancelled)?,
            _ = wait_cancelled(cancellation) => return Err(WebSearchError::Cancelled),
        };
        let attempts = runtime.attempts(engine)?;
        let mut last = WebSearchError::InvalidResponse;
        for (index, key) in attempts {
            for retry in 0..2 {
                let started = Instant::now();
                let result = self.call_engine(engine, request, &key, cancellation).await;
                runtime.record(
                    engine,
                    index,
                    started.elapsed(),
                    &result
                        .as_ref()
                        .map(|r| r.results.len())
                        .map_err(Clone::clone),
                );
                match result {
                    Ok(found) => {
                        if let Some(budget) = &self.search_budget {
                            budget.partial.lock().push((
                                budget.round.load(std::sync::atomic::Ordering::Relaxed) as u8,
                                found.clone(),
                            ));
                        }
                        return Ok(found);
                    }
                    Err(WebSearchError::Cancelled) => return Err(WebSearchError::Cancelled),
                    Err(error) => {
                        let class = error.class();
                        let rotate = matches!(class, FailureClass::Auth | FailureClass::Quota)
                            || matches!(
                                error,
                                WebSearchError::HttpStatus(429)
                                    | WebSearchError::ProviderBackoff {
                                        class: FailureClass::Transient,
                                        ..
                                    }
                            );
                        last = error;
                        if rotate {
                            break;
                        }
                        if class == FailureClass::Transient && retry == 0 {
                            continue;
                        }
                        return Err(last);
                    }
                }
            }
        }
        Err(last)
    }

    async fn call_engine(
        &self,
        engine: E,
        request: &WebSearchRequest,
        key: &str,
        cancellation: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        match engine {
            E::Agy => self.search_local(request, false, cancellation).await,
            E::Exa | E::Keenable | E::Parallel if key.is_empty() => {
                self.search_mcp(engine, request, key, cancellation).await
            }
            E::Openai | E::Gemini | E::Claude | E::DeepseekOfficial | E::Perplexity => {
                self.search_model(engine, request, key, cancellation).await
            }
            E::ZhihuGlobal | E::ZhihuSite | E::Aliyun => {
                self.search_mcp(engine, request, key, cancellation).await
            }
            E::DdgLite | E::Bing | E::BaiduHtml | E::So360 | E::Sogou | E::Wechat | E::Toutiao => {
                self.search_html(engine, request, cancellation).await
            }
            E::Kimi | E::DeepseekTools | E::Baidu | E::Doubao | E::Metaso | E::Yuanbao => {
                self.search_regional(engine, request, key, cancellation)
                    .await
            }
            E::Duckduckgo => self.search_duckduckgo(request, cancellation).await,
            E::Yandex => self.search_yandex(request, key, cancellation).await,
            E::Searxng | E::Degoog => {
                self.search_instances(engine, request, key, cancellation)
                    .await
            }
            _ => self.search_rest(engine, request, key, cancellation).await,
        }
    }
}
