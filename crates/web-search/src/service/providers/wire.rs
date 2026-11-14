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

// Derived from dsh-free-search 6ab5168 (MIT), lib/engines result projection.

use crate::{WebSearchError, WebSearchProvider, WebSearchResult};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Candidate {
    Source(Box<Source>),
    Other(serde::de::IgnoredAny),
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct Source {
    pub url: String,
    pub link: String,
    pub title: String,
    pub snippet: String,
    pub summary: String,
    pub chunks: Vec<Chunk>,
    pub content: String,
    pub description: String,
    pub highlights: Vec<String>,
    pub snippets: Vec<String>,
    pub excerpts: Vec<String>,
    #[serde(rename = "publishedAt")]
    pub published: Option<String>,
    pub pagemap: Option<PageMap>,
    #[serde(rename = "publishedDate")]
    pub published_date: Option<String>,
    pub published_at: Option<String>,
    pub publish_date: Option<String>,
    pub date: Option<String>,
}

impl Source {
    pub(super) fn into_result(self, engine: WebSearchProvider) -> Option<WebSearchResult> {
        let url = if self.url.is_empty() {
            self.link
        } else {
            self.url
        };
        if url.is_empty() {
            return None;
        }
        let title = if self.title.is_empty() {
            url.clone()
        } else {
            self.title
        };
        let snippet = [self.summary, self.snippet, self.content, self.description]
            .into_iter()
            .find(|text| !text.is_empty())
            .unwrap_or_else(|| {
                self.highlights
                    .into_iter()
                    .chain(self.snippets)
                    .chain(self.excerpts)
                    .chain(self.chunks.into_iter().map(|c| match c {
                        Chunk::Text(t) | Chunk::Object { text: t } => t,
                    }))
                    .collect::<Vec<_>>()
                    .join(" ")
            });
        Some(WebSearchResult {
            title,
            url,
            snippet,
            source: engine.id().into(),
            engines: vec![engine.id().into()],
            published_at: self
                .published_date
                .or(self.published)
                .or(self.published_at)
                .or(self.publish_date)
                .or(self.date)
                .or_else(|| {
                    self.pagemap
                        .and_then(|m| m.metatags.into_iter().next())
                        .and_then(|m| m.published.or(m.updated))
                }),
        })
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Results {
    List(Vec<Candidate>),
    Web { web: Vec<Candidate> },
}
impl Default for Results {
    fn default() -> Self {
        Self::List(Vec::new())
    }
}
impl Results {
    fn items(self) -> Vec<Candidate> {
        match self {
            Self::List(v) | Self::Web { web: v } => v,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct Nested {
    pub results: Vec<Candidate>,
    pub web: Vec<Candidate>,
}
#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Data {
    List(Vec<Candidate>),
    Nested(Nested),
}
impl Default for Data {
    fn default() -> Self {
        Self::Nested(Nested::default())
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct SearchResponse {
    pub results: Results,
    pub data: Data,
    pub hits: Vec<Candidate>,
    pub items: Vec<Candidate>,
    pub organic: Vec<Candidate>,
    pub references: Vec<Candidate>,
    pub pages: Vec<Candidate>,
    pub search_results: Vec<Candidate>,
    pub webpages: Vec<Candidate>,
    pub documents: Vec<Candidate>,
    pub scholars: Vec<Candidate>,
    pub code: Option<i64>,
    pub status: Option<i64>,
}

pub(super) fn decode(
    body: &[u8],
    engine: WebSearchProvider,
) -> Result<Vec<WebSearchResult>, WebSearchError> {
    let data: SearchResponse =
        serde_json::from_slice(body).map_err(|_| WebSearchError::InvalidResponse)?;
    data.results(engine)
}
impl SearchResponse {
    pub(super) fn results(
        self,
        engine: WebSearchProvider,
    ) -> Result<Vec<WebSearchResult>, WebSearchError> {
        let data = self;
        if data.code.is_some_and(|code| code != 0 && code != 200)
            || data
                .status
                .is_some_and(|status| status != 0 && status != 200)
        {
            return Err(WebSearchError::InvalidResponse);
        }
        let nested = match data.data {
            Data::List(v) => v,
            Data::Nested(v) => v.results.into_iter().chain(v.web).collect(),
        };
        Ok(data
            .results
            .items()
            .into_iter()
            .chain(nested)
            .chain(data.hits)
            .chain(data.items)
            .chain(data.organic)
            .chain(data.references)
            .chain(data.pages)
            .chain(data.search_results)
            .chain(data.webpages)
            .chain(data.documents)
            .chain(data.scholars)
            .filter_map(|candidate| match candidate {
                Candidate::Source(source) => source.into_result(engine),
                Candidate::Other(_) => None,
            })
            .collect())
    }
}
#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Chunk {
    Text(String),
    Object { text: String },
}

#[derive(Deserialize)]
pub(super) struct PageMap {
    #[serde(default)]
    metatags: Vec<Meta>,
}
#[derive(Deserialize)]
struct Meta {
    #[serde(rename = "article:published_time")]
    published: Option<String>,
    #[serde(rename = "og:updated_time")]
    updated: Option<String>,
}
