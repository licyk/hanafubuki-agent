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

//! Bounded, Tauri-free web search and public web-page fetching.
//!
//! This crate owns HTTP, rate limiting, response parsing, deterministic URL
//! de-duplication, and safe result projection. It does not own Agent tools,
//! application settings, approvals, or Agent protocol. Optional local helper
//! execution is delegated through `LocalWebRunner`.

mod client;
mod engines;
mod fetch;
pub use fetch::{FetchOptions, FetchPreference};
mod model;
pub use service::{
    CacheMatch, CachedPage, DocumentKind, EvidenceQuality, ResearchDocument, ResearchOptions,
    ResearchReport, TextOrigin,
};
mod service;
mod time_range;

pub use model::{
    WebFetchBackend, WebFetchError, WebFetchRequest, WebFetchResponse, WebSearchCategory,
    WebSearchConfig, WebSearchError, WebSearchFailure, WebSearchFailureCode, WebSearchProvider,
    WebSearchProviderPreference, WebSearchProxy, WebSearchRequest, WebSearchResponse,
    WebSearchResult, WebSearchTimeRange, DEFAULT_WEB_FETCH_CONTENT_BYTES,
    MAX_WEB_FETCH_CONTENT_BYTES, MAX_WEB_FETCH_DOCUMENT_BYTES, MAX_WEB_SEARCH_QUERY_BYTES,
    MAX_WEB_SEARCH_REGION_BYTES, MAX_WEB_SEARCH_RESULTS,
};
pub use service::EngineCheck;
pub use service::WebSearchService;
pub use service::{DomainEngine, Platform, SourceResponse, VideoSource};
pub use service::{EvidenceLevel, SearchUncertainty};

pub use engines::{EngineConfig, FailureClass, SafeSearch, SearchOptions};
pub use engines::{EngineDiagnostic, EngineStatistics};
pub use service::{CurrentWeather, DailyWeather, WeatherReport};

mod local;
pub use local::{LocalWebFuture, LocalWebKind, LocalWebRequest, LocalWebRunner};
