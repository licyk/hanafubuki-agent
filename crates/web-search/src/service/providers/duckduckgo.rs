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

use super::super::*;

impl WebSearchService {
    pub(in crate::service) async fn search_duckduckgo(
        &self,
        request: &WebSearchRequest,
        cancellation: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        let safe = match self.inner.config.search.safe_search {
            crate::SafeSearch::Off => "-1",
            crate::SafeSearch::Moderate => "0",
            crate::SafeSearch::Strict => "1",
        };
        let mut form = vec![("q", request.query.as_str()), ("b", ""), ("adlt", safe)];
        if let Some(time_range) = request.time_range {
            form.push(("df", time_range.as_duckduckgo()));
        }
        if let Some(region) = request.region.as_deref() {
            form.push(("kl", region));
        }
        let primary = self
            .send_bounded(
                self.inner
                    .client
                    .post(self.inner.duckduckgo_url.clone())
                    .form(&form),
                cancellation,
            )
            .await;
        let mut results = primary
            .as_ref()
            .map(|body| parse_duckduckgo(body, request.maximum_results))
            .unwrap_or_default();
        if results.is_empty() {
            let mut url = self.inner.duckduckgo_url.clone();
            {
                let mut pairs = url.query_pairs_mut();
                pairs.append_pair("q", &request.query);
                pairs.append_pair("adlt", safe);
                if let Some(time_range) = request.time_range {
                    pairs.append_pair("df", time_range.as_duckduckgo());
                }
                if let Some(region) = request.region.as_deref() {
                    pairs.append_pair("kl", region);
                }
            }
            let body = self.get_bounded(url, cancellation).await?;
            results = parse_duckduckgo(&body, request.maximum_results);
        }
        Ok(WebSearchResponse {
            query: request.query.clone(),
            providers: vec![WebSearchProvider::Duckduckgo],
            results,
            answers: Vec::new(),
            suggestions: Vec::new(),
            partial_failures: Vec::new(),
            cache_hit: false,
            uncertainty: None,
        })
    }
}

fn parse_duckduckgo(body: &[u8], maximum_results: usize) -> Vec<WebSearchResult> {
    let html = String::from_utf8_lossy(body);
    let document = Document::from(html.as_ref());
    let parsed = document.select("div.result").iter().filter_map(|card| {
        let link = card.select("a.result__a").first();
        let title = link.text().trim().to_string();
        let href = link.attr("href")?.to_string();
        if title.is_empty() || href.is_empty() {
            return None;
        }
        let url = decode_duckduckgo_url(&href)?;
        let snippet = card
            .select(".result__snippet")
            .first()
            .text()
            .trim()
            .to_string();
        let source = card
            .select(".result__url")
            .first()
            .text()
            .trim()
            .to_string();
        Some(WebSearchResult {
            title,
            url,
            snippet,
            source,
            engines: vec!["duckduckgo".to_string()],
            published_at: None,
        })
    });
    normalize_results(parsed, maximum_results)
}

fn decode_duckduckgo_url(raw: &str) -> Option<String> {
    let absolute = if raw.starts_with("//") {
        format!("https:{raw}")
    } else {
        raw.to_string()
    };
    let parsed = Url::parse(&absolute).ok()?;
    if parsed
        .host_str()
        .is_some_and(|host| host.ends_with("duckduckgo.com"))
        && parsed.path().starts_with("/l/")
    {
        if let Some((_, target)) = parsed.query_pairs().find(|(key, _)| key == "uddg") {
            return Some(target.into_owned());
        }
    }
    Some(parsed.into())
}
