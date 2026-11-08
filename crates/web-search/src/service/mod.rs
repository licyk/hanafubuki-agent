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
    config: WebSearchConfig,
    client: Client,
    duckduckgo_url: Url,
    searxng_url: Option<Url>,
    concurrency: Arc<Semaphore>,
    next_request_at: AsyncMutex<Instant>,
    cache: Mutex<VecDeque<CacheEntry>>,
    page_fetcher: WebPageFetcher,
}

#[derive(Clone)]
pub struct WebSearchService {
    inner: Arc<WebSearchInner>,
}

impl WebSearchService {
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
        headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.8"));
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
            inner: Arc::new(WebSearchInner {
                config,
                client,
                duckduckgo_url,
                searxng_url,
                concurrency: Arc::new(Semaphore::new(maximum_concurrent_requests)),
                next_request_at: AsyncMutex::new(Instant::now()),
                cache: Mutex::new(VecDeque::new()),
                page_fetcher,
            }),
        })
    }

    pub async fn search(
        &self,
        request: WebSearchRequest,
        cancellation: CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        if cancellation.is_cancelled() {
            return Err(WebSearchError::Cancelled);
        }
        let key = CacheKey {
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

        let semaphore = self.inner.concurrency.clone();
        let permit = tokio::select! {
            permit = semaphore.acquire_owned() => permit.map_err(|_| WebSearchError::Cancelled)?,
            _ = wait_cancelled(&cancellation) => return Err(WebSearchError::Cancelled),
        };
        log::debug!(
            "web search executing provider={:?} category={:?} max_results={}",
            request.provider,
            request.category,
            request.maximum_results
        );
        let result = self.execute(&request, &cancellation).await;
        drop(permit);
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
        let result = self.inner.page_fetcher.fetch(request, cancellation).await;
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

mod cache;
mod http;
mod normalize;
mod providers;
mod routing;

use http::{parse_search_endpoint, wait_cancelled};
use normalize::*;
