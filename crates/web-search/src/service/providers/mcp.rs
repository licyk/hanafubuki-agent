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

// Derived from dsh-free-search 6ab5168 (MIT), mcp_adapter.js and MCP engines.

use super::wire;
use crate::service::{normalize_results, WebSearchService};
use crate::{
    WebSearchError, WebSearchProvider as E, WebSearchRequest, WebSearchResponse, WebSearchResult,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct Call<'a> {
    jsonrpc: &'static str,
    id: u32,
    method: &'static str,
    params: Parameters<'a>,
}
#[derive(Serialize)]
struct Parameters<'a> {
    name: &'static str,
    arguments: Arguments<'a>,
}
#[derive(Default, Serialize)]
struct Arguments<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    query: Option<&'a str>,
    #[serde(rename = "numResults", skip_serializing_if = "Option::is_none")]
    num_results: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    published_after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    search_db: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    objective: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    search_queries: Option<[&'a str; 1]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<&'a str>,
}
#[derive(Deserialize)]
struct Envelope {
    result: Option<Output>,
    error: Option<serde::de::IgnoredAny>,
}
#[derive(Deserialize)]
struct Output {
    #[serde(default, rename = "isError")]
    is_error: bool,
    content: Vec<Block>,
}
#[derive(Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

impl WebSearchService {
    pub(in crate::service) async fn search_mcp(
        &self,
        engine: E,
        r: &WebSearchRequest,
        key: &str,
        cancellation: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        if engine == E::Aliyun && r.query.chars().count() < 2 {
            return Err(WebSearchError::InvalidQuery(
                "Aliyun requires at least two characters",
            ));
        }
        let short_query = r.query.chars().take(100).collect::<String>();
        let mut args = Arguments {
            query: Some(&r.query),
            ..Arguments::default()
        };
        let (endpoint, tool) = match engine {
            E::Aliyun => (
                "https://dashscope.aliyuncs.com/api/v1/mcps/EnhancedSearch/mcp",
                "search_pro",
            ),
            E::Exa => {
                args.num_results = Some(r.maximum_results);
                ("https://mcp.exa.ai/mcp", "web_search_exa")
            }
            E::Keenable => {
                args.published_after = r.time_range.map(|t| t.start_date());
                ("https://api.keenable.ai/mcp", "search_web_pages")
            }
            E::Parallel => {
                args.query = None;
                args.objective = Some(
                    r.time_range
                        .map(|t| {
                            format!(
                                "{} (prefer results published after {})",
                                r.query,
                                t.start_date()
                            )
                        })
                        .unwrap_or_else(|| r.query.clone()),
                );
                args.search_queries = Some([&r.query]);
                args.session_id = Some(&self.inner.mcp_session);
                ("https://search.parallel.ai/mcp", "web_search")
            }
            E::ZhihuGlobal | E::ZhihuSite => {
                args.query = Some(&short_query);
                args.count = Some(r.maximum_results.min(20));
                if engine == E::ZhihuGlobal {
                    args.search_db = Some("all");
                    args.filter = r
                        .time_range
                        .map(|t| format!("publish_time>={}", t.start_timestamp()));
                }
                (
                    "https://developer.zhihu.com/api/mcp/v1",
                    if engine == E::ZhihuGlobal {
                        "global_search"
                    } else {
                        "zhihu_search"
                    },
                )
            }
            _ => {
                return Err(WebSearchError::InvalidConfiguration(
                    "not an MCP search engine",
                ))
            }
        };
        let mut builder = self
            .inner
            .client
            .post(self.endpoint(engine, endpoint)?)
            .header("accept", "application/json, text/event-stream")
            .json(&Call {
                jsonrpc: "2.0",
                id: 1,
                method: "tools/call",
                params: Parameters {
                    name: tool,
                    arguments: args,
                },
            });
        if !key.is_empty() {
            builder = builder.bearer_auth(key);
        }
        let bytes = self.send_bounded(builder, cancellation).await?;
        let output = parse(&bytes)?;
        let mut results = Vec::new();
        for block in output.content.into_iter().filter(|b| b.kind == "text") {
            let parsed = if matches!(engine, E::Exa | E::Keenable) {
                parse_sources(&block.text, engine)
            } else if matches!(engine, E::ZhihuGlobal | E::ZhihuSite) {
                parse_zhihu(&block.text, engine)?
            } else {
                wire::decode(block.text.as_bytes(), engine)?
            };
            results.extend(parsed);
        }
        Ok(WebSearchResponse {
            query: r.query.clone(),
            providers: vec![engine],
            results: normalize_results(results, r.maximum_results),
            answers: Vec::new(),
            suggestions: Vec::new(),
            partial_failures: Vec::new(),
            cache_hit: false,
            uncertainty: None,
        })
    }
}

fn parse(bytes: &[u8]) -> Result<Output, WebSearchError> {
    let text = String::from_utf8_lossy(bytes).replace("\r\n", "\n");
    let direct = serde_json::from_str::<Envelope>(&text).ok();
    let envelope = direct
        .or_else(|| {
            text.split("\n\n")
                .filter_map(|event| {
                    let data = event
                        .lines()
                        .filter_map(|line| line.strip_prefix("data:"))
                        .map(str::trim_start)
                        .collect::<Vec<_>>()
                        .join("\n");
                    serde_json::from_str::<Envelope>(&data).ok()
                })
                .find(|e| e.result.is_some() || e.error.is_some())
        })
        .or_else(|| {
            text.lines()
                .filter_map(|line| {
                    serde_json::from_str::<Envelope>(line.strip_prefix("data:")?.trim()).ok()
                })
                .find(|e| e.result.is_some() || e.error.is_some())
        })
        .ok_or(WebSearchError::InvalidResponse)?;
    if envelope.error.is_some() {
        return Err(WebSearchError::InvalidResponse);
    }
    let output = envelope.result.ok_or(WebSearchError::InvalidResponse)?;
    if output.is_error {
        return Err(WebSearchError::InvalidResponse);
    }
    Ok(output)
}

fn parse_sources(text: &str, engine: E) -> Vec<WebSearchResult> {
    text.split("Title:")
        .filter_map(|block| {
            let mut lines = block.lines();
            let title = lines.next()?.trim().to_string();
            let mut url = None;
            let mut published_at = None;
            let mut excerpt = false;
            let mut snippets = Vec::new();
            for line in lines {
                if let Some(value) = line.strip_prefix("URL:") {
                    url = Some(value.trim().to_string());
                } else if let Some(value) = line
                    .strip_prefix("Published:")
                    .or_else(|| line.strip_prefix("Acquired:"))
                {
                    if value.trim().starts_with(|c: char| c.is_ascii_digit()) {
                        published_at = Some(value.trim().into());
                    }
                } else if matches!(line.trim(), "Highlights:" | "Snippets:") {
                    excerpt = true;
                } else if excerpt
                    && !line.trim().is_empty()
                    && !line.trim().starts_with("...")
                    && snippets.len() < 3
                {
                    snippets.push(line.trim());
                }
            }
            Some(WebSearchResult {
                title,
                url: url?,
                snippet: snippets.join(" "),
                source: engine.id().into(),
                engines: vec![engine.id().into()],
                published_at,
            })
        })
        .collect()
}

#[derive(Deserialize)]
struct Zhihu {
    code: Option<i64>,
    data: ZhihuData,
}
#[derive(Deserialize)]
struct ZhihuData {
    items: Vec<ZhihuCandidate>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum ZhihuCandidate {
    Item(ZhihuItem),
    Other(serde::de::IgnoredAny),
}
#[derive(Deserialize)]
struct ZhihuItem {
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    summary: String,
    edit_time: Option<i64>,
}
fn parse_zhihu(text: &str, engine: E) -> Result<Vec<WebSearchResult>, WebSearchError> {
    let value: Zhihu = serde_json::from_str(text).map_err(|_| WebSearchError::InvalidResponse)?;
    if value.code.is_some_and(|code| code != 0) {
        return Err(WebSearchError::InvalidResponse);
    }
    Ok(value
        .data
        .items
        .into_iter()
        .filter_map(|item| {
            let ZhihuCandidate::Item(item) = item else {
                return None;
            };
            Some(WebSearchResult {
                title: item.title,
                url: item.url,
                snippet: item.summary,
                source: engine.id().into(),
                engines: vec![engine.id().into()],
                published_at: item
                    .edit_time
                    .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
                    .map(|t| t.to_rfc3339()),
            })
        })
        .collect())
}
