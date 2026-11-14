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

// Derived from dsh-free-search 6ab5168 (MIT), regional search APIs.

use super::wire;
use crate::service::auxiliary::wire::Items;
use crate::service::{normalize_results, WebSearchService};
use crate::{
    WebSearchError, WebSearchProvider as E, WebSearchRequest, WebSearchResponse, WebSearchResult,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct Kimi<'a> {
    text_query: &'a str,
    limit: usize,
    timeout_seconds: u8,
}
#[derive(Serialize)]
struct Deepseek {
    query: String,
    count: usize,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Metaso<'a> {
    q: &'a str,
    scope: &'a str,
    size: usize,
    include_summary: bool,
    include_raw_content: bool,
    concise_snippet: bool,
}
#[derive(Serialize)]
struct Baidu<'a> {
    messages: [Message<'a>; 1],
    search_source: &'static str,
    resource_type_filter: [Resource; 1],
    search_filter: SearchFilter,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}
#[derive(Serialize)]
struct Message<'a> {
    content: &'a str,
    role: &'static str,
}
#[derive(Serialize)]
struct Resource {
    #[serde(rename = "type")]
    kind: &'static str,
    top_k: usize,
}
#[derive(Serialize)]
struct SearchFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    range: Option<Range>,
}
#[derive(Serialize)]
struct Range {
    page_time: PageTime,
}
#[derive(Serialize)]
struct PageTime {
    gte: String,
    lt: String,
}
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct Doubao {
    query: String,
    search_type: &'static str,
    count: usize,
    need_summary: bool,
    filter: DoubaoFilter,
    #[serde(skip_serializing_if = "Option::is_none")]
    time_range: Option<&'static str>,
}
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct DoubaoFilter {
    need_url: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct Yuanbao<'a> {
    query: &'a str,
    mode: u8,
    #[serde(skip_serializing_if = "str::is_empty")]
    site: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_time: Option<i64>,
}

impl WebSearchService {
    pub(in crate::service) async fn search_regional(
        &self,
        engine: E,
        r: &WebSearchRequest,
        key: &str,
        cancellation: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        let default = match engine {
            E::Kimi => "https://api.moonshot.cn/v1/tools/search",
            E::DeepseekTools => "https://api.deepseek.com/v1/tools/search",
            E::Baidu => "https://qianfan.baidubce.com/v2/ai_search/web_search",
            E::Doubao => "https://open.feedcoopapi.com/search_api/web_search",
            E::Metaso => "https://metaso.cn/api/v1/search",
            E::Yuanbao => "https://api.wsa.cloud.tencent.com/SearchPro",
            _ => {
                return Err(WebSearchError::InvalidConfiguration(
                    "not a regional API search engine",
                ))
            }
        };
        let mut url = self.endpoint(engine, default)?;
        if engine == E::DeepseekTools && matches!(url.path(), "/" | "/v1" | "/v1/") {
            url.set_path("/v1/tools/search");
        }
        let builder = self.inner.client.post(url.clone()).bearer_auth(key);
        let options = self
            .inner
            .config
            .search
            .engines
            .get(&engine)
            .cloned()
            .unwrap_or_default();
        let baidu_body = || Baidu {
            messages: [Message {
                content: &r.query,
                role: "user",
            }],
            search_source: "baidu_search_v2",
            resource_type_filter: [Resource {
                kind: "web",
                top_k: r.maximum_results.min(50),
            }],
            search_filter: SearchFilter {
                range: r.time_range.map(|t| Range {
                    page_time: PageTime {
                        gte: t.start_date(),
                        lt: (chrono::Utc::now() + chrono::Duration::days(1))
                            .format("%Y-%m-%d")
                            .to_string(),
                    },
                }),
            },
            stream: None,
        };
        let builder = match engine {
            E::Kimi => builder.json(&Kimi {
                text_query: &r.query,
                limit: r.maximum_results.min(20),
                timeout_seconds: 30,
            }),
            E::DeepseekTools => builder.json(&Deepseek {
                query: r
                    .time_range
                    .map(|t| format!("{} after:{}", r.query, t.start_date()))
                    .unwrap_or_else(|| r.query.clone()),
                count: (r.maximum_results * 3).div_ceil(2).min(50),
            }),
            E::Metaso => builder.json(&Metaso {
                q: &r.query,
                scope: if ["webpage", "document", "scholar"]
                    .contains(&options.metaso_scope.as_str())
                {
                    &options.metaso_scope
                } else {
                    "webpage"
                },
                size: r.maximum_results.min(50),
                include_summary: true,
                include_raw_content: false,
                concise_snippet: true,
            }),
            E::Baidu => builder
                .header("X-Appbuilder-From", "openclaw")
                .json(&baidu_body()),
            E::Doubao => builder
                .header("X-Traffic-Tag", "hanafubuki-agent")
                .json(&Doubao {
                    query: r.query.chars().take(100).collect(),
                    search_type: "web",
                    count: r.maximum_results.min(50),
                    need_summary: true,
                    filter: DoubaoFilter { need_url: true },
                    time_range: r.time_range.map(|t| match t.as_searxng() {
                        "day" => "OneDay",
                        "week" => "OneWeek",
                        "month" => "OneMonth",
                        _ => "OneYear",
                    }),
                }),
            E::Yuanbao => builder.json(&Yuanbao {
                query: &r.query,
                mode: options.yuanbao_mode.min(2),
                site: &options.yuanbao_site,
                from_time: r.time_range.map(|t| t.start_timestamp()),
                to_time: r.time_range.map(|_| chrono::Utc::now().timestamp()),
            }),
            _ => {
                return Err(WebSearchError::InvalidConfiguration(
                    "not a regional API search engine",
                ))
            }
        };
        let primary = self.send_bounded(builder, cancellation).await;
        let primary = primary.and_then(|bytes| decode(engine, &bytes));
        let results = if engine == E::Baidu
            && matches!(
                primary,
                Err(WebSearchError::HttpStatus(403 | 429)
                    | WebSearchError::InvalidResponse
                    | WebSearchError::ProviderBackoff {
                        class: crate::FailureClass::Auth
                            | crate::FailureClass::Transient
                            | crate::FailureClass::Quota,
                        ..
                    })
            ) {
            url.set_path("/v2/ai_search/chat/completions");
            let mut body = baidu_body();
            body.stream = Some(false);
            let bytes = self
                .send_bounded(
                    self.inner
                        .client
                        .post(url)
                        .bearer_auth(key)
                        .header("X-Appbuilder-From", "openclaw")
                        .json(&body),
                    cancellation,
                )
                .await?;
            decode(engine, &bytes)?
        } else {
            primary?
        };
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

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DoubaoResponse {
    result: Option<DoubaoResult>,
    response_metadata: Option<Metadata>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Metadata {
    error: Option<serde::de::IgnoredAny>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DoubaoResult {
    web_results: Items<DoubaoItem>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
struct DoubaoItem {
    url: String,
    title: String,
    summary: String,
    content: String,
    snippet: String,
    publish_time: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct YuanbaoResponse {
    response: YuanbaoResult,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct YuanbaoResult {
    #[serde(default)]
    pages: Vec<YuanbaoPage>,
    error: Option<serde::de::IgnoredAny>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum YuanbaoPage {
    Json(String),
    Item(Page),
    Other(serde::de::IgnoredAny),
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Page {
    vr: bool,
    display: Option<Box<Page>>,
    url: String,
    title: String,
    passage: String,
    content: String,
    date: Option<String>,
}

fn decode(engine: E, bytes: &[u8]) -> Result<Vec<WebSearchResult>, WebSearchError> {
    if engine == E::Doubao {
        let data: DoubaoResponse =
            serde_json::from_slice(bytes).map_err(|_| WebSearchError::InvalidResponse)?;
        if data.response_metadata.is_some_and(|m| m.error.is_some()) {
            return Err(WebSearchError::InvalidResponse);
        }
        return Ok(data
            .result
            .map(|r| r.web_results.0)
            .unwrap_or_default()
            .into_iter()
            .map(|item| WebSearchResult {
                url: item.url,
                title: item.title,
                snippet: [item.summary, item.content, item.snippet]
                    .into_iter()
                    .find(|t| !t.is_empty())
                    .unwrap_or_default(),
                source: engine.id().into(),
                engines: vec![engine.id().into()],
                published_at: item.publish_time,
            })
            .collect());
    }
    if engine == E::Yuanbao {
        let data: YuanbaoResponse =
            serde_json::from_slice(bytes).map_err(|_| WebSearchError::InvalidResponse)?;
        if data.response.error.is_some() {
            return Err(WebSearchError::InvalidResponse);
        }
        return Ok(data
            .response
            .pages
            .into_iter()
            .filter_map(|page| {
                let item = match page {
                    YuanbaoPage::Json(s) => serde_json::from_str::<Page>(&s).ok()?,
                    YuanbaoPage::Item(item) => item,
                    YuanbaoPage::Other(_) => return None,
                };
                let item = if item.vr { *item.display? } else { item };
                Some(WebSearchResult {
                    title: item.title,
                    url: item.url,
                    snippet: if item.passage.is_empty() {
                        item.content
                    } else {
                        item.passage
                    },
                    source: engine.id().into(),
                    engines: vec![engine.id().into()],
                    published_at: item.date,
                })
            })
            .collect());
    }
    wire::decode(bytes, engine)
}
