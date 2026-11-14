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

// Derived from dsh-free-search 6ab5168 (MIT), lib/engines/yandex.js.

use crate::service::{normalize_results, WebSearchService};
use crate::{
    SafeSearch, WebSearchError, WebSearchProvider as E, WebSearchRequest, WebSearchResponse,
    WebSearchResult,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use dom_query::Document;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Search<'a> {
    query: Query<'a>,
    group_spec: Group,
    max_passages: &'static str,
    folder_id: &'a str,
    response_format: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    period: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    region: Option<&'a str>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Query<'a> {
    search_type: String,
    query_text: &'a str,
    family_mode: &'static str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Group {
    group_mode: &'static str,
    groups_on_page: String,
    docs_in_group: &'static str,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    raw_data: String,
}

impl WebSearchService {
    pub(in crate::service) async fn search_yandex(
        &self,
        r: &WebSearchRequest,
        key: &str,
        cancellation: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        let config = self
            .inner
            .config
            .search
            .engines
            .get(&E::Yandex)
            .cloned()
            .unwrap_or_default();
        if config.yandex_folder_id.is_empty() {
            return Err(WebSearchError::InvalidConfiguration(
                "Yandex needs yandexFolderId",
            ));
        }
        let language = if ["ru", "tr", "com", "kk", "be", "uz"]
            .contains(&config.yandex_search_type.as_str())
        {
            config.yandex_search_type.as_str()
        } else if ["ru", "tr", "kk", "be", "uz"]
            .contains(&self.inner.config.search.language.as_str())
        {
            self.inner.config.search.language.as_str()
        } else {
            "com"
        };
        let body = Search {
            query: Query {
                search_type: format!("SEARCH_TYPE_{}", language.to_uppercase()),
                query_text: &r.query,
                family_mode: match self.inner.config.search.safe_search {
                    SafeSearch::Off => "FAMILY_MODE_NONE",
                    SafeSearch::Moderate => "FAMILY_MODE_MODERATE",
                    SafeSearch::Strict => "FAMILY_MODE_STRICT",
                },
            },
            group_spec: Group {
                group_mode: "GROUP_MODE_FLAT",
                groups_on_page: r.maximum_results.min(50).to_string(),
                docs_in_group: "1",
            },
            max_passages: "3",
            folder_id: &config.yandex_folder_id,
            response_format: "FORMAT_XML",
            region: r.region.as_deref(),
            period: r.time_range.and_then(|t| match t.days() {
                1 => Some("PERIOD_DAY"),
                2..=14 => Some("PERIOD_2_WEEKS"),
                15..=31 => Some("PERIOD_MONTH"),
                _ => None,
            }),
        };
        let bytes = self
            .send_bounded(
                self.inner
                    .client
                    .post(self.endpoint(
                        E::Yandex,
                        "https://searchapi.api.cloud.yandex.net/v2/web/search",
                    )?)
                    .header("authorization", format!("Api-Key {key}"))
                    .json(&body),
                cancellation,
            )
            .await?;
        let response: Response =
            serde_json::from_slice(&bytes).map_err(|_| WebSearchError::InvalidResponse)?;
        let bytes = STANDARD
            .decode(response.raw_data)
            .map_err(|_| WebSearchError::InvalidResponse)?;
        let text = String::from_utf8(bytes).map_err(|_| WebSearchError::InvalidResponse)?;
        let document = Document::from(text);
        let results = document.select("doc").iter().map(|doc| {
            let date = doc.select("modtime").text().to_string();
            let published_at =
                chrono::NaiveDate::parse_from_str(date.get(..8).unwrap_or(""), "%Y%m%d")
                    .ok()
                    .map(|d| d.to_string());
            WebSearchResult {
                title: doc.select("title").text().to_string(),
                url: doc.select("url").text().to_string(),
                snippet: doc
                    .select("passage")
                    .iter()
                    .take(3)
                    .map(|p| p.text().to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
                source: E::Yandex.id().into(),
                engines: vec![E::Yandex.id().into()],
                published_at,
            }
        });
        Ok(WebSearchResponse {
            query: r.query.clone(),
            providers: vec![E::Yandex],
            results: normalize_results(results, r.maximum_results),
            answers: Vec::new(),
            suggestions: Vec::new(),
            partial_failures: Vec::new(),
            cache_hit: false,
            uncertainty: None,
        })
    }
}
