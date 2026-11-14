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

// Derived from dsh-free-search 6ab5168 (MIT), native video page adapters.
use super::{decode, plain, source, wire::Items, SourceResponse};
use crate::{WebSearchError, WebSearchResult, WebSearchService};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoSource {
    #[default]
    Auto,
    Bing,
    Duckduckgo,
}

impl WebSearchService {
    pub async fn search_video(
        &self,
        query: &str,
        maximum: usize,
        requested: VideoSource,
        cancellation: CancellationToken,
    ) -> Result<SourceResponse, WebSearchError> {
        let maximum = maximum.clamp(1, 10);
        let order: &[VideoSource] = match requested {
            VideoSource::Auto => &[VideoSource::Bing, VideoSource::Duckduckgo],
            VideoSource::Bing => &[VideoSource::Bing],
            VideoSource::Duckduckgo => &[VideoSource::Duckduckgo],
        };
        let mut sources = Vec::new();
        let mut failures = Vec::new();
        for engine in order {
            match self.video_provider(*engine, query, &cancellation).await {
                Ok(found) => sources.extend(found),
                Err(WebSearchError::Cancelled) => return Err(WebSearchError::Cancelled),
                Err(error) => failures.push(format!("{engine:?}: {error}")),
            }
            if sources.len() >= maximum {
                break;
            }
        }
        if sources.is_empty() {
            return Err(WebSearchError::NoResults);
        }
        sources.retain(|r| !self.blocked(&r.url));
        let mut response = SourceResponse::new(sources, maximum);
        response.notes = failures;
        Ok(response)
    }

    async fn video_provider(
        &self,
        provider: VideoSource,
        query: &str,
        cancellation: &CancellationToken,
    ) -> Result<Vec<WebSearchResult>, WebSearchError> {
        match provider {
            VideoSource::Bing | VideoSource::Auto => {
                let bytes = self
                    .send_bounded(
                        self.inner
                            .client
                            .get("https://www.bing.com/videos/asyncv2")
                            .query(&[
                                ("q", query),
                                ("count", "10"),
                                ("first", "1"),
                                ("mmasync", "1"),
                                ("mkt", self.inner.config.search.market()),
                            ]),
                        cancellation,
                    )
                    .await?;
                let html = String::from_utf8_lossy(&bytes);
                let lower = html.to_lowercase();
                if ["captcha", "unusual traffic", "robot check"]
                    .iter()
                    .any(|needle| lower.contains(needle))
                {
                    return Err(WebSearchError::ProviderFailure(
                        crate::FailureClass::BotWall,
                    ));
                }
                Ok(parse_bing(&html))
            }
            VideoSource::Duckduckgo => {
                let landing = self
                    .send_bounded(
                        self.inner.client.get("https://duckduckgo.com/").query(&[
                            ("q", query),
                            ("iax", "videos"),
                            ("ia", "videos"),
                        ]),
                        cancellation,
                    )
                    .await?;
                let landing = String::from_utf8_lossy(&landing);
                let regex = Regex::new(r#"vqd=["']?([\d-]+)"#)
                    .map_err(|_| WebSearchError::InvalidResponse)?;
                let token = regex
                    .captures(&landing)
                    .and_then(|c| c.get(1))
                    .ok_or(WebSearchError::InvalidResponse)?;
                let bytes = self
                    .send_bounded(
                        self.inner
                            .client
                            .get("https://duckduckgo.com/v.js")
                            .query(&[
                                ("l", "us-en"),
                                ("o", "json"),
                                ("q", query),
                                ("vqd", token.as_str()),
                                ("f", ",,,"),
                                ("p", "1"),
                            ]),
                        cancellation,
                    )
                    .await?;
                parse_ddg(&String::from_utf8_lossy(&bytes))
            }
        }
    }

    pub(super) async fn youtube(
        &self,
        query: &str,
        maximum: usize,
        cancellation: &CancellationToken,
    ) -> Result<SourceResponse, WebSearchError> {
        let bytes = self
            .send_bounded(
                self.inner
                    .client
                    .get("https://www.youtube.com/results")
                    .query(&[("search_query", query)])
                    .header("accept-language", "en-US,en;q=0.9"),
                cancellation,
            )
            .await?;
        Ok(SourceResponse::new(
            parse_youtube(&String::from_utf8_lossy(&bytes))?,
            maximum,
        ))
    }

    pub(super) async fn vimeo(
        &self,
        query: &str,
        maximum: usize,
        cancellation: &CancellationToken,
    ) -> Result<SourceResponse, WebSearchError> {
        match self.vimeo_native(query, maximum, cancellation).await {
            Ok(found) if !found.sources.is_empty() => return Ok(found),
            Err(WebSearchError::Cancelled) => return Err(WebSearchError::Cancelled),
            _ => {}
        }
        let request = crate::WebSearchRequest {
            query: format!("site:vimeo.com {query}"),
            maximum_results: maximum,
            engines: Vec::new(),
            provider: crate::WebSearchProvider::Bing,
            category: Default::default(),
            time_range: None,
            region: None,
        };
        for engine in [
            crate::WebSearchProvider::Bing,
            crate::WebSearchProvider::Duckduckgo,
        ] {
            match self.fetch_provider(engine, &request, cancellation).await {
                Ok(found) => {
                    let sources = found
                        .results
                        .into_iter()
                        .filter(|r| {
                            reqwest::Url::parse(&r.url).ok().is_some_and(|u| {
                                u.host_str()
                                    .is_some_and(|h| h == "vimeo.com" || h.ends_with(".vimeo.com"))
                            })
                        })
                        .collect::<Vec<_>>();
                    if !sources.is_empty() {
                        return Ok(SourceResponse::new(sources, maximum));
                    }
                }
                Err(WebSearchError::Cancelled) => return Err(WebSearchError::Cancelled),
                _ => {}
            }
        }
        Err(WebSearchError::NoResults)
    }

    async fn vimeo_native(
        &self,
        query: &str,
        maximum: usize,
        cancellation: &CancellationToken,
    ) -> Result<SourceResponse, WebSearchError> {
        let bytes = self
            .send_bounded(
                self.inner
                    .client
                    .get("https://vimeo.com/search")
                    .query(&[("q", query)]),
                cancellation,
            )
            .await?;
        let html = String::from_utf8_lossy(&bytes);
        let uri = Regex::new(r#""uri"\s*:\s*"/videos/(\d+)""#)
            .map_err(|_| WebSearchError::InvalidResponse)?;
        let name = Regex::new(r#""name"\s*:\s*("(?:[^"\\]|\\.)*")"#)
            .map_err(|_| WebSearchError::InvalidResponse)?;
        let sources = uri
            .captures_iter(&html)
            .filter_map(|c| {
                let id = c.get(1)?;
                let at = c.get(0)?.start();
                let start = html[..at]
                    .char_indices()
                    .rev()
                    .nth(600)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                let end = html[at..]
                    .char_indices()
                    .nth(200)
                    .map(|(i, _)| at + i)
                    .unwrap_or(html.len());
                let title = name
                    .captures(&html[start..end])
                    .and_then(|c| c.get(1))
                    .and_then(|s| serde_json::from_str::<String>(s.as_str()).ok())
                    .unwrap_or_default();
                Some(source(
                    "vimeo",
                    format!("https://vimeo.com/{}", id.as_str()),
                    plain(&title),
                    "",
                    None,
                ))
            })
            .collect();
        Ok(SourceResponse::new(sources, maximum))
    }
}

fn parse_bing(html: &str) -> Vec<WebSearchResult> {
    dom_query::Document::from(html)
        .select("[vrhm]")
        .iter()
        .filter_map(|node| {
            let raw = node.attr("vrhm")?;
            let v: BingVideo = serde_json::from_str(&raw).ok()?;
            Some(source(
                "bing-videos",
                v.murl,
                plain(&v.vt),
                [v.du, v.vd]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · "),
                None,
            ))
        })
        .collect()
}

fn parse_ddg(text: &str) -> Result<Vec<WebSearchResult>, WebSearchError> {
    let start = text.find('{').ok_or(WebSearchError::InvalidResponse)?;
    let end = text.rfind('}').ok_or(WebSearchError::InvalidResponse)?;
    if start >= end {
        return Err(WebSearchError::InvalidResponse);
    }
    let response: DdgResponse = decode(&text.as_bytes()[start..=end])?;
    Ok(response
        .results
        .0
        .into_iter()
        .filter_map(|v| {
            Some(source(
                "duckduckgo-videos",
                v.content.filter(|s| !s.is_empty()).or(v.url)?,
                plain(&v.title),
                [v.uploader, v.duration, v.published]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · "),
                None,
            ))
        })
        .collect())
}

fn parse_youtube(html: &str) -> Result<Vec<WebSearchResult>, WebSearchError> {
    let at = html
        .find("ytInitialData")
        .ok_or(WebSearchError::InvalidResponse)?;
    let brace = html[at..]
        .find('{')
        .ok_or(WebSearchError::InvalidResponse)?
        + at;
    let mut decoder = serde_json::Deserializer::from_str(&html[brace..]);
    let tree = super::youtube::VideoTree::deserialize(&mut decoder)
        .map_err(|_| WebSearchError::InvalidResponse)?;
    Ok(tree
        .0
        .into_iter()
        .map(|v| {
            source(
                "youtube",
                format!("https://www.youtube.com/watch?v={}", v.video_id),
                plain(&v.title.text()),
                [
                    v.owner_text.text(),
                    v.long_byline_text.text(),
                    v.length_text.text(),
                    v.view_count_text.text(),
                    v.published_time_text.text(),
                ]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · "),
                None,
            )
        })
        .collect())
}

#[derive(Deserialize)]
struct BingVideo {
    murl: String,
    #[serde(default)]
    vt: String,
    du: Option<String>,
    vd: Option<String>,
}
#[derive(Deserialize)]
struct DdgResponse {
    #[serde(default)]
    results: Items<DdgVideo>,
}
#[derive(Deserialize)]
struct DdgVideo {
    content: Option<String>,
    url: Option<String>,
    #[serde(default)]
    title: String,
    uploader: Option<String>,
    duration: Option<String>,
    published: Option<String>,
}

#[cfg(test)]
mod tests;
