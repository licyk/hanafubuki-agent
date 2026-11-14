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

// Derived from dsh-free-search 6ab5168 (MIT), HTML engines and serp_engines.js.

use crate::service::{normalize_results, WebSearchService};
use crate::{
    FailureClass, SafeSearch, WebSearchError, WebSearchProvider as E, WebSearchRequest,
    WebSearchResponse, WebSearchResult,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use dom_query::Document;
use hanafubuki_agent_foundation::cancellation::CancellationToken;

impl WebSearchService {
    pub(in crate::service) async fn search_html(
        &self,
        engine: E,
        r: &WebSearchRequest,
        cancellation: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        let (base, query) = match engine {
            E::DdgLite => ("https://lite.duckduckgo.com/lite/", "q"),
            E::Bing => ("https://www.bing.com/search", "q"),
            E::BaiduHtml => ("https://www.baidu.com/s", "wd"),
            E::So360 => ("https://www.so.com/s", "q"),
            E::Sogou => ("https://www.sogou.com/web", "query"),
            E::Wechat => ("https://wx.sogou.com/weixin", "query"),
            E::Toutiao => ("https://so.toutiao.com/search", "keyword"),
            _ => {
                return Err(WebSearchError::InvalidConfiguration(
                    "not an HTML search engine",
                ))
            }
        };
        let mut url = self.endpoint(engine, base)?;
        url.query_pairs_mut().append_pair(query, &r.query);
        let options = &self.inner.config.search;
        if engine == E::Wechat {
            url.query_pairs_mut().append_pair("type", "2");
        }
        if engine == E::BaiduHtml {
            url.query_pairs_mut().append_pair("rn", "20");
        }
        if engine == E::Bing {
            // An implicit zh-CN market can produce an empty SERP abroad.
            if !options.bing_market.is_empty() {
                url.query_pairs_mut()
                    .append_pair("mkt", &options.bing_market);
            }
            url.query_pairs_mut().append_pair(
                "adlt",
                match options.safe_search {
                    SafeSearch::Off => "off",
                    SafeSearch::Moderate => "moderate",
                    SafeSearch::Strict => "strict",
                },
            );
        }
        if engine == E::DdgLite {
            url.query_pairs_mut().append_pair(
                "adlt",
                match options.safe_search {
                    SafeSearch::Off => "-1",
                    SafeSearch::Moderate => "0",
                    SafeSearch::Strict => "1",
                },
            );
            if let Some(time) = r.time_range {
                url.query_pairs_mut()
                    .append_pair("df", time.as_duckduckgo());
            }
        }
        if matches!(engine, E::So360 | E::Sogou) {
            if let Some(time) = r.time_range {
                let days = match time.days() {
                    0..=1 => 1,
                    2..=7 => 7,
                    8..=30 => 30,
                    _ => 365,
                };
                url.query_pairs_mut()
                    .append_pair("qft", &format!("+filterui:age-lt{}", days * 1440));
            }
        }
        let body = self
            .send_bounded(
                self.inner
                    .client
                    .get(url)
                    .header("accept-language", options.accept_language()),
                cancellation,
            )
            .await?;
        let html = String::from_utf8_lossy(&body);
        let lower = html.to_lowercase();
        if [
            "captcha",
            "unusual traffic",
            "robot check",
            "访问异常",
            "安全验证",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
        {
            return Err(WebSearchError::ProviderFailure(FailureClass::BotWall));
        }
        let mut results = parse(&html, engine);
        if engine != E::Wechat {
            self.resolve_signed_links(&mut results, cancellation)
                .await?;
        }
        if engine == E::Bing && !relevant(&r.query, &results) {
            results.clear();
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

fn parse(html: &str, engine: E) -> Vec<WebSearchResult> {
    let document = Document::from(html);
    if engine == E::DdgLite {
        let snippets = document
            .select(".result-snippet")
            .iter()
            .map(|s| s.text().to_string())
            .collect::<Vec<_>>();
        return document
            .select("a.result-link")
            .iter()
            .enumerate()
            .filter_map(|(i, link)| {
                Some(source(
                    engine,
                    link.text().to_string(),
                    decode_url(&link.attr("href")?, engine)?,
                    snippets.get(i).cloned().unwrap_or_default(),
                ))
            })
            .collect();
    }
    if engine == E::Toutiao {
        let found = toutiao(html);
        if !found.is_empty() {
            return found;
        }
    }
    let (card, anchor, snippet) = match engine {
        E::Bing => ("li.b_algo", "h2 a", "p"),
        E::BaiduHtml => ("div.result", "h3 a", ".content-right, .c-abstract"),
        E::So360 => ("li.res-list", "h3 a", "p"),
        E::Sogou => ("div.vrwrap", "h3 a", "p"),
        E::Wechat => ("li[id^=sogou_vr_11002601_box]", "h3 a", "p"),
        _ => ("article", "a", "p"),
    };
    let found = document
        .select(card)
        .iter()
        .filter_map(|card| {
            let link = card.select(anchor).first();
            let href = if engine == E::BaiduHtml {
                card.attr("mu").or_else(|| link.attr("href"))
            } else {
                link.attr("href")
            }?;
            Some(source(
                engine,
                link.text().to_string(),
                decode_url(&href, engine)?,
                card.select(snippet).text().to_string(),
            ))
        })
        .collect::<Vec<_>>();
    if !found.is_empty() {
        return found;
    }
    document
        .select("a[href]")
        .iter()
        .filter_map(|link| {
            let url = reqwest::Url::parse(&link.attr("href")?).ok()?;
            let host = url.host_str()?;
            if engine == E::Wechat && host != "mp.weixin.qq.com" {
                return None;
            }
            if [
                "bing.com",
                "baidu.com",
                "bdstatic.com",
                "so.com",
                "360.cn",
                "qhimg.com",
                "sogou.com",
                "sogoucdn.com",
                "toutiao.com",
                "zjurl.cn",
                "bytedance.com",
                "snssdk.com",
                "ixigua.com",
                "microsoft.com",
                "msn.com",
                "msn.cn",
                "google.com",
                "gstatic.com",
                "googleusercontent.com",
                "mozilla.org",
                "apple.com",
                "adobe.com",
            ]
            .iter()
            .any(|ignored| host == *ignored || host.ends_with(&format!(".{ignored}")))
            {
                return None;
            }
            let title = link.text().trim().to_string();
            if title.is_empty()
                || [
                    "登录", "注册", "首页", "更多", "下载", "帮助", "反馈", "设置",
                ]
                .contains(&title.as_str())
            {
                return None;
            }
            Some(source(engine, title, url.to_string(), String::new()))
        })
        .take(25)
        .collect()
}

fn source(engine: E, title: String, url: String, snippet: String) -> WebSearchResult {
    WebSearchResult {
        title,
        url,
        snippet,
        source: engine.id().into(),
        engines: vec![engine.id().into()],
        published_at: None,
    }
}

fn decode_url(href: &str, engine: E) -> Option<String> {
    let base = if engine == E::Wechat {
        "https://weixin.sogou.com"
    } else if engine == E::Sogou {
        "https://www.sogou.com"
    } else if engine == E::So360 {
        "https://www.so.com"
    } else if engine == E::Bing {
        "https://www.bing.com"
    } else if engine == E::BaiduHtml {
        "https://www.baidu.com"
    } else {
        "https://duckduckgo.com"
    };
    let url = reqwest::Url::parse(base).ok()?.join(href).ok()?;
    if engine == E::DdgLite {
        if let Some((_, target)) = url.query_pairs().find(|(key, _)| key == "uddg") {
            return Some(target.into_owned());
        }
    }
    if engine == E::Bing && url.path().starts_with("/ck/a") {
        if let Some((_, target)) = url.query_pairs().find(|(key, _)| key == "u") {
            if let Ok(bytes) = URL_SAFE_NO_PAD.decode(
                target
                    .strip_prefix("a1")
                    .unwrap_or(&target)
                    .trim_end_matches('='),
            ) {
                if let Ok(text) = String::from_utf8(bytes) {
                    if text.starts_with("https://") || text.starts_with("http://") {
                        return Some(text);
                    }
                }
            }
        }
    }
    Some(url.to_string())
}

fn relevant(query: &str, sources: &[WebSearchResult]) -> bool {
    let mut tokens = query
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(str::to_string)
        .collect::<Vec<_>>();
    let chars = query.chars().collect::<Vec<_>>();
    tokens.extend(
        chars
            .windows(2)
            .filter(|pair| pair.iter().all(|c| ('\u{4e00}'..='\u{9fff}').contains(c)))
            .map(|pair| pair.iter().collect::<String>()),
    );
    tokens.sort();
    tokens.dedup();
    if tokens.is_empty() {
        return true;
    }
    if tokens
        .iter()
        .any(|t| !t.bytes().all(|c| c.is_ascii_digit()))
    {
        tokens.retain(|t| !t.bytes().all(|c| c.is_ascii_digit()));
    }
    let mut matches = std::collections::HashSet::new();
    let mut hits = 0;
    for source in sources {
        let text = format!("{} {} {}", source.title, source.snippet, source.url).to_lowercase();
        let found = tokens
            .iter()
            .filter(|t| text.contains(t.as_str()))
            .collect::<Vec<_>>();
        if !found.is_empty() {
            hits += 1;
            matches.extend(found);
        }
    }
    hits >= sources.len().div_ceil(3).max(1)
        && (matches.len() >= tokens.len().min(2) || matches.len() * 2 >= tokens.len())
}

fn toutiao(html: &str) -> Vec<WebSearchResult> {
    let patterns = [
        r#""title":"((?:[^"\\]|\\.)*)"[\s\S]{0,4000}?"article_url":"https?://toutiao\.com/group/(\d+)"#,
        r#""article_url":"https?://toutiao\.com/group/(\d+)[^"]*"[\s\S]{0,2000}?"title":"((?:[^"\\]|\\.)*)""#,
    ];
    let mut out = Vec::new();
    for (index, pattern) in patterns.iter().enumerate() {
        let Ok(pattern) = regex::Regex::new(pattern) else {
            continue;
        };
        for captures in pattern.captures_iter(html).take(25) {
            let (title, id) = if index == 0 {
                (&captures[1], &captures[2])
            } else {
                (&captures[2], &captures[1])
            };
            let title = serde_json::from_str::<String>(&format!("\"{title}\""))
                .unwrap_or_else(|_| title.into());
            out.push(source(
                E::Toutiao,
                Document::from(title).text().to_string(),
                format!("https://www.toutiao.com/group/{id}/"),
                String::new(),
            ));
        }
    }
    out
}
