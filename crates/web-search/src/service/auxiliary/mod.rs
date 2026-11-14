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

//! Native platform and domain APIs ported from dsh-free-search (MIT).

mod domain;
mod platform;
pub use domain::DomainEngine;
pub(crate) use domain::ImfCatalog;
pub(crate) use domain::KevCache;
mod video;
pub(super) mod wire;
mod youtube;
pub use video::VideoSource;

use crate::{WebSearchError, WebSearchResult};
use serde::{Deserialize, Serialize};

pub use platform::Platform;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceResponse {
    pub sources: Vec<WebSearchResult>,
    pub untrusted: bool,
    pub notes: Vec<String>,
}

impl SourceResponse {
    pub(crate) fn new(sources: Vec<WebSearchResult>, maximum: usize) -> Self {
        Self {
            sources: super::normalize_results(sources, maximum),
            untrusted: true,
            notes: Vec::new(),
        }
    }
}

fn source(
    engine: &str,
    url: impl Into<String>,
    title: impl Into<String>,
    snippet: impl Into<String>,
    date: Option<String>,
) -> WebSearchResult {
    let url = url.into();
    let title = title.into();
    WebSearchResult {
        title: if title.is_empty() { url.clone() } else { title },
        url,
        snippet: snippet.into(),
        source: engine.into(),
        engines: vec![engine.into()],
        published_at: date,
    }
}

fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, WebSearchError> {
    serde_json::from_slice(bytes).map_err(|_| WebSearchError::InvalidResponse)
}

fn plain(text: &str) -> String {
    dom_query::Document::from(text)
        .text()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
