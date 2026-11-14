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

use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dom_query::Document;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_foundation::user_agent::BROWSER_USER_AGENT;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, USER_AGENT};
use reqwest::{Client, Url};
use serde::Deserialize;
use tokio::sync::{Mutex as AsyncMutex, Semaphore};

use crate::client::apply_proxy;
use crate::fetch::WebPageFetcher;
use crate::model::{
    WebFetchError, WebFetchRequest, WebFetchResponse, WebSearchConfig, WebSearchError,
    WebSearchProvider, WebSearchProviderPreference, WebSearchRequest, WebSearchResponse,
    WebSearchResult, MAX_WEB_SEARCH_ANSWERS, MAX_WEB_SEARCH_ANSWER_BYTES,
    MAX_WEB_SEARCH_SUGGESTIONS, MAX_WEB_SEARCH_TITLE_BYTES,
};

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Clone, Debug, PartialEq, Eq)]
struct CacheKey {
    engines: Vec<crate::WebSearchProvider>,
    query: String,
    maximum_results: usize,
    provider: WebSearchProvider,
    category: crate::WebSearchCategory,
    time_range: Option<crate::WebSearchTimeRange>,
    region: Option<String>,
}

struct CacheEntry {
    key: CacheKey,
    inserted_at: Instant,
    response: WebSearchResponse,
}

struct WebSearchInner {
    mcp_session: String,
    documents: research::DocumentCache,
    engines: crate::engines::EngineRuntime,
    config: WebSearchConfig,
    client: Client,
    duckduckgo_url: Url,
    searxng_url: Option<Url>,
    concurrency: Arc<Semaphore>,
    next_request_at: AsyncMutex<Instant>,
    cache: Mutex<VecDeque<CacheEntry>>,
    kev_cache: Mutex<Option<auxiliary::KevCache>>,
    imf_catalog: Mutex<Option<auxiliary::ImfCatalog>>,
    page_fetcher: WebPageFetcher,
}

#[derive(Clone)]
pub struct WebSearchService {
    local_runner: Option<Arc<dyn crate::LocalWebRunner>>,
    inner: Arc<WebSearchInner>,
    search_budget: Option<Arc<research::SearchBudget>>,
}

impl WebSearchService {
    pub fn with_local_runner(mut self, runner: Option<Arc<dyn crate::LocalWebRunner>>) -> Self {
        self.local_runner = runner;
        self
    }
    pub fn local_helpers_enabled(&self) -> bool {
        self.inner.config.search.local_helpers_enabled && self.local_runner.is_some()
    }
    pub fn cache_search(&self, query: &str, maximum: usize) -> Vec<crate::CacheMatch> {
        self.inner
            .documents
            .search(query, 20)
            .into_iter()
            .filter(|page| !self.blocked(&page.url))
            .take(maximum.clamp(1, 20))
            .collect()
    }
    pub fn prompt_language(&self) -> &str {
        &self.inner.config.search.prompt_language
    }
    pub fn multi_result_limit(&self) -> usize {
        self.inner.config.search.multi_max_results.clamp(1, 50)
    }
    pub fn diagnostics(&self) -> Vec<crate::EngineDiagnostic> {
        self.inner.engines.diagnostics(&self.inner.config.search)
    }

    pub fn new(config: WebSearchConfig) -> Result<Self, WebSearchError> {
        if config.maximum_concurrent_requests == 0 || config.maximum_concurrent_requests > 16 {
            return Err(WebSearchError::InvalidConfiguration(
                "maximum concurrent requests must be between 1 and 16",
            ));
        }
        if config.maximum_cache_entries == 0 || config.maximum_cache_entries > 1024 {
            return Err(WebSearchError::InvalidConfiguration(
                "maximum cache entries must be between 1 and 1024",
            ));
        }
        if config.request_timeout.is_zero() || config.request_timeout > Duration::from_secs(60) {
            return Err(WebSearchError::InvalidConfiguration(
                "request timeout must be between 1 millisecond and 60 seconds",
            ));
        }
        let (duckduckgo_url, searxng_url) = search_endpoints(&config)?;

        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(BROWSER_USER_AGENT));
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("text/html,application/json;q=0.9,*/*;q=0.5"),
        );
        headers.insert(
            ACCEPT_LANGUAGE,
            HeaderValue::from_str(&config.search.accept_language())
                .unwrap_or_else(|_| HeaderValue::from_static("en-US,en;q=0.9")),
        );
        let builder = Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(5).min(config.request_timeout))
            .timeout(config.request_timeout)
            .redirect(reqwest::redirect::Policy::none());
        let builder = apply_proxy(builder, &config.proxy)
            .map_err(|_| WebSearchError::InvalidConfiguration("proxy URL is invalid"))?;
        let client = builder
            .build()
            .map_err(|_| WebSearchError::InvalidConfiguration("HTTP client could not be built"))?;
        let page_fetcher = WebPageFetcher::new(&config).map_err(|error| {
            WebSearchError::InvalidConfiguration(match error {
                WebFetchError::InvalidConfiguration(message) => message,
                _ => "web page fetcher configuration is invalid",
            })
        })?;
        let maximum_concurrent_requests = config.maximum_concurrent_requests;
        Ok(Self {
            local_runner: None,
            search_budget: None,
            inner: Arc::new(WebSearchInner {
                mcp_session: uuid::Uuid::new_v4().to_string(),
                documents: research::DocumentCache::new(config.state_directory.as_deref()),
                engines: crate::engines::EngineRuntime::new(
                    &config.search,
                    config.state_directory.as_deref(),
                    &config.proxy,
                ),
                config,
                client,
                duckduckgo_url,
                searxng_url,
                concurrency: Arc::new(Semaphore::new(maximum_concurrent_requests)),
                next_request_at: AsyncMutex::new(Instant::now()),
                cache: Mutex::new(VecDeque::new()),
                kev_cache: Mutex::new(None),
                imf_catalog: Mutex::new(None),
                page_fetcher,
            }),
        })
    }

    pub async fn search(
        &self,
        mut request: WebSearchRequest,
        cancellation: CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        if cancellation.is_cancelled() {
            return Err(WebSearchError::Cancelled);
        }
        if request.region.is_none() {
            request.region = self.inner.config.search.region.clone();
        }
        let key = CacheKey {
            engines: request.engines.clone(),
            query: request.query.clone(),
            maximum_results: request.maximum_results,
            provider: request.provider,
            category: request.category,
            time_range: request.time_range,
            region: request.region.clone(),
        };
        if let Some(mut cached) = self.cached(&key) {
            cached.cache_hit = true;
            log::debug!(
                "web search cache hit provider={:?} category={:?}",
                request.provider,
                request.category
            );
            return Ok(cached);
        }

        log::debug!(
            "web search executing provider={:?} category={:?} max_results={}",
            request.provider,
            request.category,
            request.maximum_results
        );
        let result = self.execute(&request, &cancellation).await;
        match &result {
            Ok(_) => log::debug!("web search completed provider={:?}", request.provider),
            Err(error) => log::warn!("web search failed provider={:?}: {error}", request.provider),
        }
        let response = result?;
        self.store_cache(key, response.clone());
        Ok(response)
    }

    pub async fn fetch_page(
        &self,
        request: WebFetchRequest,
        cancellation: CancellationToken,
    ) -> Result<WebFetchResponse, WebFetchError> {
        let semaphore = self.inner.concurrency.clone();
        let permit = tokio::select! {
            permit = semaphore.acquire_owned() => permit.map_err(|_| WebFetchError::Cancelled)?,
            _ = wait_cancelled(&cancellation) => return Err(WebFetchError::Cancelled),
        };
        // The address is a model-chosen tool argument, so it stays out of the
        // log at every level below `trace`.
        log::debug!("web fetch executing");
        let result = self
            .inner
            .page_fetcher
            .fetch(
                request,
                cancellation,
                self.search_budget.as_deref(),
                self.local_runner.as_deref(),
            )
            .await;
        drop(permit);
        match &result {
            Ok(_) => log::debug!("web fetch completed"),
            Err(error) => log::warn!("web fetch failed: {error}"),
        }
        result
    }
}

impl WebSearchConfig {
    /// Why this configuration's search endpoints cannot be used, when they
    /// cannot. [`WebSearchService::new`] refuses such a configuration; a caller
    /// that can go on without search (page fetches need no search endpoint)
    /// reads the reason here and builds the service without them.
    pub fn search_endpoint_error(&self) -> Option<WebSearchError> {
        search_endpoints(self).err()
    }
}

/// The DuckDuckGo and SearXNG endpoints.
fn search_endpoints(config: &WebSearchConfig) -> Result<(Url, Option<Url>), WebSearchError> {
    let duckduckgo_url = parse_search_endpoint(&config.duckduckgo_search_url)?;
    let searxng_url = config
        .searxng_base_url
        .as_deref()
        .map(parse_search_endpoint)
        .transpose()?;
    if config.provider_preference == WebSearchProviderPreference::Searxng && searxng_url.is_none() {
        return Err(WebSearchError::InvalidConfiguration(
            "SearXNG is selected but no endpoint is configured",
        ));
    }
    Ok((duckduckgo_url, searxng_url))
}

mod auxiliary;
mod cache;
mod diagnostics;
pub use auxiliary::{DomainEngine, Platform, SourceResponse, VideoSource};
pub use diagnostics::EngineCheck;
mod http;
mod normalize;
mod providers;
mod routing;

use http::{parse_search_endpoint, wait_cancelled};
use normalize::*;

mod quality;
mod ranking;
pub use quality::{EvidenceLevel, SearchUncertainty};
mod weather;
pub use weather::{CurrentWeather, DailyWeather, WeatherReport};

pub(crate) mod research;
pub use research::{
    CacheMatch, CachedPage, DocumentKind, EvidenceQuality, ResearchDocument, ResearchOptions,
    ResearchReport, TextOrigin,
};

mod snippet;
