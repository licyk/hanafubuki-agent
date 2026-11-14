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

use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const MAX_WEB_SEARCH_QUERY_BYTES: usize = 512;
pub const MAX_WEB_SEARCH_RESULTS: usize = 10;
pub const MAX_WEB_FETCH_CONTENT_BYTES: usize = 100_000;
pub const DEFAULT_WEB_FETCH_CONTENT_BYTES: usize = 50_000;
pub const MAX_WEB_FETCH_DOCUMENT_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_WEB_SEARCH_BODY_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_WEB_SEARCH_TITLE_BYTES: usize = 512;
pub(crate) const MAX_WEB_SEARCH_SNIPPET_BYTES: usize = 8 * 1024;
pub(crate) const MAX_WEB_SEARCH_URL_BYTES: usize =
    hanafubuki_agent_foundation::url_policy::MAX_URL_BYTES;
pub(crate) const MAX_WEB_SEARCH_ANSWER_BYTES: usize = 1024;
pub(crate) const MAX_WEB_SEARCH_ANSWERS: usize = 4;
pub(crate) const MAX_WEB_SEARCH_SUGGESTIONS: usize = 8;
pub const MAX_WEB_SEARCH_REGION_BYTES: usize = 16;
pub(crate) const MAX_WEB_FETCH_DIRECT_BODY_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_WEB_FETCH_READER_BODY_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAX_WEB_FETCH_REDIRECTS: usize = 5;

pub use crate::engines::WebSearchProvider;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WebSearchProviderPreference {
    #[default]
    Auto,
    Duckduckgo,
    Searxng,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchCategory {
    #[default]
    General,
    News,
    It,
    Science,
}

impl WebSearchCategory {
    pub(crate) const fn as_searxng(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::News => "news",
            Self::It => "it",
            Self::Science => "science",
        }
    }
}

pub use crate::time_range::WebSearchTimeRange;

/// A search as the caller bounded it (`web_search` checks its arguments): a
/// trimmed query of 1 to [`MAX_WEB_SEARCH_QUERY_BYTES`] without control
/// characters, 1 to [`MAX_WEB_SEARCH_RESULTS`] results, and a region of 1 to
/// [`MAX_WEB_SEARCH_REGION_BYTES`] ASCII letters, digits or hyphens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebSearchRequest {
    /// Explicit pool for multi search; subject to the host's allow/deny policy.
    pub engines: Vec<WebSearchProvider>,
    pub query: String,
    pub maximum_results: usize,
    pub provider: WebSearchProvider,
    pub category: WebSearchCategory,
    pub time_range: Option<WebSearchTimeRange>,
    pub region: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebSearchProxy {
    System,
    Disabled,
    Explicit(String),
}

#[derive(Clone, Debug)]
pub struct WebSearchConfig {
    pub state_directory: Option<std::path::PathBuf>,
    pub search: crate::engines::SearchOptions,
    pub provider_preference: WebSearchProviderPreference,
    pub searxng_base_url: Option<String>,
    pub proxy: WebSearchProxy,
    pub request_timeout: Duration,
    pub minimum_request_interval: Duration,
    pub maximum_concurrent_requests: usize,
    pub cache_ttl: Duration,
    pub maximum_cache_entries: usize,
    pub page_fetch_timeout: Duration,
    pub jina_reader_timeout: Duration,
    pub page_fetch_total_timeout: Duration,
    pub jina_reader_enabled: bool,
    /// Through a proxy URL, a page's name this machine cannot resolve is
    /// left to the proxy instead of failing the fetch: the Agent cannot
    /// check where it points. Names that resolve here are still checked.
    pub resolve_through_proxy: bool,
    pub(crate) duckduckgo_search_url: String,
    pub(crate) jina_reader_base_url: String,
    pub(crate) test_allowed_private_origins: Vec<String>,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            state_directory: None,
            search: crate::engines::SearchOptions::default(),
            provider_preference: WebSearchProviderPreference::Auto,
            searxng_base_url: None,
            proxy: WebSearchProxy::System,
            request_timeout: Duration::from_secs(12),
            minimum_request_interval: Duration::from_millis(250),
            maximum_concurrent_requests: 4,
            cache_ttl: Duration::from_secs(5 * 60),
            maximum_cache_entries: 128,
            page_fetch_timeout: Duration::from_secs(20),
            jina_reader_timeout: Duration::from_secs(20),
            page_fetch_total_timeout: Duration::from_secs(45),
            jina_reader_enabled: false,
            resolve_through_proxy: false,
            duckduckgo_search_url: "https://html.duckduckgo.com/html/".to_string(),
            jina_reader_base_url: "https://r.jina.ai/".to_string(),
            test_allowed_private_origins: Vec::new(),
        }
    }
}

impl WebSearchConfig {
    #[cfg(feature = "test-support")]
    pub fn with_test_duckduckgo_url(mut self, value: impl Into<String>) -> Self {
        self.duckduckgo_search_url = value.into();
        self
    }

    #[cfg(feature = "test-support")]
    pub fn with_test_jina_reader_url(mut self, value: impl Into<String>) -> Self {
        self.jina_reader_base_url = value.into();
        self
    }

    #[cfg(feature = "test-support")]
    pub fn with_test_allowed_private_origin(mut self, value: impl Into<String>) -> Self {
        self.test_allowed_private_origins.push(value.into());
        self
    }
}

/// A page fetch as the caller bounded it (`fetch_web_page` checks its
/// arguments).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebFetchRequest {
    /// An HTTP(S) page without credentials; its fragment is ignored.
    pub url: url::Url,
    /// 4 to [`MAX_WEB_FETCH_CONTENT_BYTES`].
    pub maximum_content_bytes: usize,
    /// At most [`MAX_WEB_FETCH_DOCUMENT_BYTES`].
    pub from_offset: usize,
    /// A lowercase SHA-256 digest, present whenever `from_offset` is not 0.
    pub content_generation: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebFetchBackend {
    Exa,
    Firecrawl,
    Crawl4ai,
    Browser,
    DirectHtml,
    DirectText,
    JinaReader,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebFetchResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    pub final_url: String,
    pub title: Option<String>,
    pub content: String,
    pub content_type: String,
    pub backend: WebFetchBackend,
    pub fetched_bytes: usize,
    pub content_bytes: usize,
    pub content_offset: usize,
    pub next_offset: Option<usize>,
    pub total_content_bytes: usize,
    pub content_generation: String,
    pub document_truncated: bool,
    pub truncated: bool,
    pub redirect_count: usize,
    pub untrusted_external_content: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebFetchError {
    BudgetExhausted,
    InvalidRequest(&'static str),
    InvalidConfiguration(&'static str),
    BlockedTarget,
    DnsResolutionFailed,
    RedirectRejected,
    TooManyRedirects,
    UnsupportedContentType,
    UnsupportedContentEncoding,
    EmptyContent,
    Cancelled,
    Timeout,
    RequestFailed,
    HttpStatus(u16),
    ResponseTooLarge,
    InvalidContentOffset,
    StaleContentGeneration,
}

impl WebFetchError {
    pub(crate) const fn may_use_reader_fallback(&self) -> bool {
        matches!(
            self,
            Self::RequestFailed
                | Self::Timeout
                | Self::ResponseTooLarge
                | Self::HttpStatus(403 | 429 | 500..=599)
                | Self::EmptyContent
                | Self::UnsupportedContentType
        )
    }
}

impl std::fmt::Display for WebFetchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BudgetExhausted => formatter.write_str("web fetch attempt budget exhausted"),
            Self::InvalidRequest(message) => {
                write!(formatter, "invalid web fetch request: {message}")
            }
            Self::InvalidConfiguration(message) => {
                write!(formatter, "invalid web fetch configuration: {message}")
            }
            Self::BlockedTarget => {
                formatter.write_str("web page target is not a public web address")
            }
            Self::DnsResolutionFailed => {
                formatter.write_str("web page host could not be resolved safely")
            }
            Self::RedirectRejected => formatter.write_str("web page redirect was rejected"),
            Self::TooManyRedirects => formatter.write_str("web page exceeded the redirect limit"),
            Self::UnsupportedContentType => {
                formatter.write_str("web page content type is not supported")
            }
            Self::UnsupportedContentEncoding => {
                formatter.write_str("web page content encoding is not supported")
            }
            Self::EmptyContent => formatter.write_str("web page did not contain readable content"),
            Self::Cancelled => formatter.write_str("web page fetch cancelled"),
            Self::Timeout => formatter.write_str("web page fetch timed out"),
            Self::RequestFailed => formatter.write_str("web page request failed"),
            Self::HttpStatus(status) => write!(formatter, "web page returned HTTP status {status}"),
            Self::ResponseTooLarge => formatter.write_str("web page exceeded its response limit"),
            Self::InvalidContentOffset => formatter.write_str("web page content offset is invalid"),
            Self::StaleContentGeneration => {
                formatter.write_str("web page content changed; restart from offset zero")
            }
        }
    }
}

impl std::error::Error for WebFetchError {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub source: String,
    pub engines: Vec<String>,
    pub published_at: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchFailureCode {
    Quota,
    Auth,
    BotWall,
    CoolingDown,
    ProviderUnavailable,
    Timeout,
    RequestFailed,
    HttpStatus,
    ResponseTooLarge,
    InvalidResponse,
    NoResults,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchFailure {
    pub provider: WebSearchProvider,
    pub code: WebSearchFailureCode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchResponse {
    #[serde(default)]
    pub uncertainty: Option<crate::SearchUncertainty>,
    pub query: String,
    pub providers: Vec<WebSearchProvider>,
    pub results: Vec<WebSearchResult>,
    pub answers: Vec<String>,
    pub suggestions: Vec<String>,
    pub partial_failures: Vec<WebSearchFailure>,
    pub cache_hit: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebSearchError {
    ProviderBackoff {
        class: crate::engines::FailureClass,
        delay: std::time::Duration,
    },
    BudgetExhausted,
    NoResults,
    InvalidQuery(&'static str),
    ProviderFailure(crate::engines::FailureClass),
    CoolingDown,
    InvalidConfiguration(&'static str),
    Cancelled,
    Timeout,
    RequestFailed,
    HttpStatus(u16),
    ResponseTooLarge,
    InvalidResponse,
}

impl WebSearchError {
    pub(crate) const fn failure_code(&self) -> WebSearchFailureCode {
        match self {
            Self::ProviderBackoff { .. } => WebSearchFailureCode::CoolingDown,
            Self::BudgetExhausted => WebSearchFailureCode::ProviderUnavailable,
            Self::NoResults => WebSearchFailureCode::NoResults,
            Self::InvalidQuery(_) => WebSearchFailureCode::InvalidResponse,
            Self::ProviderFailure(crate::engines::FailureClass::Quota) => {
                WebSearchFailureCode::Quota
            }
            Self::ProviderFailure(crate::engines::FailureClass::Auth) => WebSearchFailureCode::Auth,
            Self::ProviderFailure(crate::engines::FailureClass::BotWall) => {
                WebSearchFailureCode::BotWall
            }
            Self::ProviderFailure(_) => WebSearchFailureCode::RequestFailed,
            Self::CoolingDown => WebSearchFailureCode::CoolingDown,
            Self::InvalidConfiguration(_) => WebSearchFailureCode::ProviderUnavailable,
            Self::Timeout => WebSearchFailureCode::Timeout,
            Self::RequestFailed => WebSearchFailureCode::RequestFailed,
            Self::HttpStatus(_) => WebSearchFailureCode::HttpStatus,
            Self::ResponseTooLarge => WebSearchFailureCode::ResponseTooLarge,
            Self::InvalidResponse => WebSearchFailureCode::InvalidResponse,
            Self::Cancelled => WebSearchFailureCode::RequestFailed,
        }
    }
}

impl std::fmt::Display for WebSearchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProviderBackoff { class, .. } => write!(
                formatter,
                "web search provider requires cooldown: {class:?}"
            ),
            Self::BudgetExhausted => formatter.write_str("web search request budget exhausted"),
            Self::NoResults => formatter.write_str("web search returned no results"),
            Self::InvalidQuery(message) => write!(formatter, "invalid web query: {message}"),
            Self::ProviderFailure(class) => {
                write!(formatter, "web search provider failed: {class:?}")
            }
            Self::CoolingDown => {
                formatter.write_str("web search engine or credentials are cooling down")
            }
            Self::InvalidConfiguration(message) => {
                write!(formatter, "invalid web search configuration: {message}")
            }
            Self::Cancelled => formatter.write_str("web search cancelled"),
            Self::Timeout => formatter.write_str("web search timed out"),
            Self::RequestFailed => formatter.write_str("web search request failed"),
            Self::HttpStatus(status) => {
                write!(
                    formatter,
                    "web search provider returned HTTP status {status}"
                )
            }
            Self::ResponseTooLarge => formatter.write_str("web search response exceeded its limit"),
            Self::InvalidResponse => {
                formatter.write_str("web search provider returned an invalid response")
            }
        }
    }
}

impl std::error::Error for WebSearchError {}
