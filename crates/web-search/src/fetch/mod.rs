use parking_lot::Mutex;
use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_foundation::user_agent::BROWSER_USER_AGENT;
use reqwest::header::{
    HeaderMap, HeaderValue, ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE, CONTENT_ENCODING,
    CONTENT_TYPE, LOCATION, USER_AGENT,
};
use reqwest::{Client, Response, Url};
use sha2::{Digest, Sha256};
use tokio::net::lookup_host;
use tokio::sync::Mutex as AsyncMutex;

use crate::client::apply_proxy;
use crate::model::{
    WebFetchBackend, WebFetchError, WebFetchRequest, WebFetchResponse, WebSearchConfig,
    WebSearchProxy, MAX_WEB_FETCH_DIRECT_BODY_BYTES, MAX_WEB_FETCH_DOCUMENT_BYTES,
    MAX_WEB_FETCH_READER_BODY_BYTES, MAX_WEB_FETCH_REDIRECTS, MAX_WEB_SEARCH_TITLE_BYTES,
    MAX_WEB_SEARCH_URL_BYTES,
};

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(20);
const MAX_HTML_NODES: usize = 20_000;
const MAX_HTML_DEPTH: usize = 64;
const MAX_DOCUMENT_CACHE_ENTRIES: usize = 8;
const MAX_DOCUMENT_CACHE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct WebPageFetcher {
    direct_timeout: Duration,
    reader_timeout: Duration,
    total_timeout: Duration,
    request_interval: Duration,
    jina_reader_enabled: bool,
    jina_reader_base: Url,
    jina_client: Client,
    proxy: WebSearchProxy,
    next_request_at: Arc<AsyncMutex<Instant>>,
    test_allowed_private_origins: Vec<String>,
    document_cache_ttl: Duration,
    document_cache: Arc<Mutex<VecDeque<CachedDocument>>>,
}

struct RawPage {
    final_url: Url,
    content_type: String,
    body: Vec<u8>,
    redirect_count: usize,
}

#[derive(Clone)]
struct FetchedDocument {
    final_url: String,
    title: Option<String>,
    content: String,
    content_type: String,
    backend: WebFetchBackend,
    fetched_bytes: usize,
    generation: String,
    document_truncated: bool,
    redirect_count: usize,
}

struct CachedDocument {
    requested_url: String,
    inserted_at: Instant,
    document: Arc<FetchedDocument>,
}

impl WebPageFetcher {
    pub(crate) fn new(config: &WebSearchConfig) -> Result<Self, WebFetchError> {
        if config.page_fetch_timeout.is_zero()
            || config.page_fetch_timeout > Duration::from_secs(60)
        {
            return Err(WebFetchError::InvalidConfiguration(
                "page timeout must be between 1 millisecond and 60 seconds",
            ));
        }
        if config.jina_reader_timeout.is_zero()
            || config.jina_reader_timeout > Duration::from_secs(60)
        {
            return Err(WebFetchError::InvalidConfiguration(
                "Jina Reader timeout must be between 1 millisecond and 60 seconds",
            ));
        }
        let minimum_total_timeout = if config.jina_reader_enabled {
            config
                .page_fetch_timeout
                .checked_add(config.jina_reader_timeout)
                .ok_or(WebFetchError::InvalidConfiguration(
                    "page total timeout is invalid",
                ))?
        } else {
            config.page_fetch_timeout
        };
        if config.page_fetch_total_timeout < minimum_total_timeout
            || config.page_fetch_total_timeout > Duration::from_secs(120)
        {
            return Err(WebFetchError::InvalidConfiguration(
                "page total timeout must cover each enabled fetch phase and be at most 120 seconds",
            ));
        }
        let jina_reader_base = validate_fixed_endpoint(&config.jina_reader_base_url)?;
        let jina_client = reader_client(config.jina_reader_timeout, &config.proxy)?;
        let test_allowed_private_origins = config
            .test_allowed_private_origins
            .iter()
            .map(|value| {
                let url = validate_page_url(value)?;
                Ok(origin(&url))
            })
            .collect::<Result<Vec<_>, WebFetchError>>()?;
        Ok(Self {
            direct_timeout: config.page_fetch_timeout,
            reader_timeout: config.jina_reader_timeout,
            total_timeout: config.page_fetch_total_timeout,
            request_interval: config.minimum_request_interval,
            jina_reader_enabled: config.jina_reader_enabled,
            jina_reader_base,
            jina_client,
            proxy: config.proxy.clone(),
            next_request_at: Arc::new(AsyncMutex::new(Instant::now())),
            test_allowed_private_origins,
            document_cache_ttl: config.cache_ttl,
            document_cache: Arc::new(Mutex::new(VecDeque::new())),
        })
    }

    pub(crate) async fn fetch(
        &self,
        request: WebFetchRequest,
        cancellation: CancellationToken,
    ) -> Result<WebFetchResponse, WebFetchError> {
        let mut target = request.url.clone();
        target.set_fragment(None);
        let requested_url = target.to_string();
        if cancellation.is_cancelled() {
            return Err(WebFetchError::Cancelled);
        }
        if let Some(generation) = request.content_generation.as_deref() {
            if let Some(document) = self.cached_document(&requested_url, generation) {
                return page_document(&document, &request);
            }
        }
        let expected_generation = request.content_generation.clone();
        let operation = async {
            let direct = tokio::select! {
                result = tokio::time::timeout(
                    self.direct_timeout,
                    self.fetch_direct(target.clone(), &cancellation),
                ) => result.unwrap_or(Err(WebFetchError::Timeout)),
                _ = wait_cancelled(&cancellation) => return Err(WebFetchError::Cancelled),
            };
            let document = match direct {
                Ok(page) => project_direct_document(page),
                Err(error) if self.jina_reader_enabled && error.may_use_reader_fallback() => {
                    tokio::select! {
                        result = tokio::time::timeout(
                            self.reader_timeout,
                            self.fetch_jina(target, &cancellation),
                        ) => result.map_err(|_| WebFetchError::Timeout)?,
                        _ = wait_cancelled(&cancellation) => {
                            return Err(WebFetchError::Cancelled);
                        }
                    }
                }
                Err(error) => return Err(error),
            }?;
            Ok(finalize_document(document, &requested_url))
        };
        let document = tokio::select! {
            result = tokio::time::timeout(self.total_timeout, operation) => {
                result.map_err(|_| WebFetchError::Timeout)?
            }
            _ = wait_cancelled(&cancellation) => Err(WebFetchError::Cancelled),
        }?;
        if expected_generation
            .as_deref()
            .is_some_and(|expected| expected != document.generation)
        {
            return Err(WebFetchError::StaleContentGeneration);
        }
        let document = Arc::new(document);
        self.store_document(requested_url, document.clone());
        page_document(&document, &request)
    }
}

mod cache;
mod direct;
mod document;
mod extract;
mod http;
mod markdown;
mod reader;
mod target_policy;

use document::{finalize_document, page_document, project_direct_document};
use extract::extract_html;
use http::*;
use markdown::truncate_utf8;
use target_policy::*;
