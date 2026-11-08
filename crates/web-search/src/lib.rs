//! Bounded, Tauri-free web search and public web-page fetching.
//!
//! This crate owns HTTP, rate limiting, response parsing, deterministic URL
//! de-duplication, and safe result projection. It does not own Agent tools,
//! application settings, approvals, Agent protocol, or browser automation.

mod client;
mod fetch;
mod model;
mod service;

pub use model::{
    WebFetchBackend, WebFetchError, WebFetchRequest, WebFetchResponse, WebSearchCategory,
    WebSearchConfig, WebSearchError, WebSearchFailure, WebSearchFailureCode, WebSearchProvider,
    WebSearchProviderPreference, WebSearchProxy, WebSearchRequest, WebSearchResponse,
    WebSearchResult, WebSearchTimeRange, DEFAULT_WEB_FETCH_CONTENT_BYTES,
    MAX_WEB_FETCH_CONTENT_BYTES, MAX_WEB_FETCH_DOCUMENT_BYTES, MAX_WEB_SEARCH_QUERY_BYTES,
    MAX_WEB_SEARCH_REGION_BYTES, MAX_WEB_SEARCH_RESULTS,
};
pub use service::WebSearchService;
