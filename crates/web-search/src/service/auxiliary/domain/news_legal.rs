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

use super::super::plain;
use super::{
    date, source, DomainEngine as E, Items, WebSearchError, WebSearchResult, WebSearchService,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde::Deserialize;

impl WebSearchService {
    pub(super) async fn domain_news_legal(
        &self,
        engine: E,
        query: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<WebSearchResult>, WebSearchError> {
        Ok(match engine {
            E::Googlenews => {
                let bytes = self
                    .send_bounded(
                        self.inner
                            .client
                            .get("https://news.google.com/rss/search")
                            .query(&[
                                ("q", query),
                                ("hl", "zh-CN"),
                                ("gl", "CN"),
                                ("ceid", "CN:zh-Hans"),
                            ]),
                        cancel,
                    )
                    .await?;
                rss(&String::from_utf8_lossy(&bytes))
            }
            E::Gdelt => {
                let d: Gdelt = self
                    .domain_json(
                        "https://api.gdeltproject.org/api/v2/doc/doc",
                        &[
                            ("query", query),
                            ("mode", "artlist"),
                            ("maxrecords", "3"),
                            ("format", "json"),
                            ("sort", "hybridrel"),
                        ],
                        cancel,
                    )
                    .await?;
                d.articles
                    .0
                    .into_iter()
                    .map(|a| {
                        let stamp = a
                            .seendate
                            .and_then(|s| s.get(..8).map(String::from))
                            .and_then(|s| chrono::NaiveDate::parse_from_str(&s, "%Y%m%d").ok())
                            .map(|d| d.to_string());
                        source(
                            "gdelt",
                            a.url,
                            a.title.unwrap_or_default(),
                            format!(
                                "{} · {}",
                                a.domain.unwrap_or_default(),
                                a.sourcecountry.unwrap_or_default()
                            ),
                            stamp,
                        )
                    })
                    .collect()
            }
            E::Aihot => {
                if !(2..=200).contains(&query.trim().chars().count()) {
                    return Err(WebSearchError::InvalidQuery(
                        "AIHOT keywords must contain 2–200 characters",
                    ));
                }
                let d: Aihot = self
                    .domain_json(
                        "https://aihot.news/api/v1/items",
                        &[
                            ("mode", "selected"),
                            ("window", "7d"),
                            ("q", query.trim()),
                            ("limit", "5"),
                        ],
                        cancel,
                    )
                    .await?;
                d.items
                    .0
                    .into_iter()
                    .filter_map(|i| {
                        Some(source(
                            "aihot",
                            i.links.original.or(i.links.aihot)?,
                            i.title.or(i.original_title).unwrap_or_default(),
                            i.summary
                                .map(|s| format!("[provider-summary] {s}"))
                                .unwrap_or_default(),
                            date(i.published_at),
                        ))
                    })
                    .collect()
            }
            E::Federalregister => {
                let d: Federal = self
                    .domain_json(
                        "https://www.federalregister.gov/api/v1/documents.json",
                        &[
                            ("per_page", "3"),
                            ("order", "relevance"),
                            ("conditions[term]", query),
                            ("fields[]", "title"),
                            ("fields[]", "html_url"),
                            ("fields[]", "publication_date"),
                            ("fields[]", "type"),
                        ],
                        cancel,
                    )
                    .await?;
                d.results
                    .0
                    .into_iter()
                    .map(|r| {
                        source(
                            "federalregister",
                            r.html_url,
                            r.title,
                            r.kind.unwrap_or_default(),
                            date(r.publication_date),
                        )
                    })
                    .collect()
            }
            E::Courtlistener => self.courtlistener(query, cancel).await?,
            E::Openlegaldata => {
                let d: Legal = self
                    .domain_json(
                        "https://de.openlegaldata.io/api/cases/search/",
                        &[("text", query), ("format", "json"), ("limit", "3")],
                        cancel,
                    )
                    .await?;
                d.results
                    .0
                    .into_iter()
                    .filter_map(|r| {
                        let slug = r.slug.or(r.id.map(|i| i.to_string()))?;
                        Some(source(
                            "openlegaldata",
                            format!("https://de.openlegaldata.io/case/{slug}/"),
                            format!(
                                "{} · {}",
                                r.court.unwrap_or_else(|| "?".into()),
                                r.decision_type.unwrap_or(slug)
                            ),
                            plain(
                                &r.snippets
                                    .0
                                    .into_iter()
                                    .next()
                                    .map(|s| s.text)
                                    .unwrap_or_default(),
                            ),
                            date(r.date),
                        ))
                    })
                    .collect()
            }
            E::UkLegislation => {
                let bytes = self
                    .send_bounded(
                        self.inner
                            .client
                            .get("https://www.legislation.gov.uk/all/data.feed")
                            .query(&[("title", query)])
                            .header(
                                "accept",
                                "application/atom+xml, application/xml, text/xml, */*",
                            ),
                        cancel,
                    )
                    .await?;
                super::academic::atom(&String::from_utf8_lossy(&bytes), "uk_legislation")
            }
            _ => return Err(WebSearchError::InvalidQuery("not a news or legal engine")),
        })
    }
    async fn courtlistener(
        &self,
        query: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<WebSearchResult>, WebSearchError> {
        Ok({
            let d: Court = self
                .domain_json(
                    "https://www.courtlistener.com/api/rest/v4/search/",
                    &[("q", query), ("type", "o"), ("page_size", "3")],
                    cancel,
                )
                .await?;
            d.results
                .0
                .into_iter()
                .filter(|r| !r.absolute_url.is_empty())
                .map(|r| {
                    source(
                        "courtlistener",
                        format!("https://www.courtlistener.com{}", r.absolute_url),
                        r.case_name.unwrap_or_default(),
                        [
                            r.court,
                            r.docket_number.map(|d| format!("No. {d}")),
                            r.citation.into_iter().next(),
                        ]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" · "),
                        date(r.date_filed),
                    )
                })
                .collect()
        })
    }
}

fn rss(xml: &str) -> Vec<WebSearchResult> {
    // HTML parsing treats <link> as a void element; keep RSS element contents intact.
    let item = match regex::Regex::new(r"(?s)<item>(.*?)</item>") {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    item.captures_iter(xml)
        .take(3)
        .map(|c| {
            let pick = |tag: &str| -> String {
                let start = format!("<{tag}>");
                let end = format!("</{tag}>");
                c[1].split_once(&start)
                    .and_then(|(_, s)| s.split_once(&end))
                    .map(|(s, _)| plain(&s.replace("<![CDATA[", "").replace("]]>", "")))
                    .unwrap_or_default()
            };
            let stamp = chrono::DateTime::parse_from_rfc2822(&pick("pubDate"))
                .ok()
                .map(|d| d.format("%Y-%m-%d").to_string());
            source(
                "googlenews",
                pick("link"),
                pick("title"),
                pick("description"),
                stamp,
            )
        })
        .collect()
}
#[derive(Deserialize)]
struct Gdelt {
    #[serde(default)]
    articles: Items<News>,
}
#[derive(Deserialize)]
struct News {
    url: String,
    title: Option<String>,
    domain: Option<String>,
    sourcecountry: Option<String>,
    seendate: Option<String>,
}
#[derive(Deserialize)]
struct Aihot {
    #[serde(default)]
    items: Items<AihotItem>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AihotItem {
    links: NewsLinks,
    title: Option<String>,
    original_title: Option<String>,
    summary: Option<String>,
    published_at: Option<String>,
}
#[derive(Deserialize)]
struct NewsLinks {
    original: Option<String>,
    aihot: Option<String>,
}
#[derive(Deserialize)]
struct Federal {
    #[serde(default)]
    results: Items<FederalDocument>,
}
#[derive(Deserialize)]
struct FederalDocument {
    html_url: String,
    title: String,
    #[serde(rename = "type")]
    kind: Option<String>,
    publication_date: Option<String>,
}
#[derive(Deserialize)]
struct Court {
    #[serde(default)]
    results: Items<CourtCase>,
}
#[derive(Deserialize)]
struct CourtCase {
    absolute_url: String,
    #[serde(rename = "caseName")]
    case_name: Option<String>,
    court: Option<String>,
    #[serde(rename = "docketNumber")]
    docket_number: Option<String>,
    #[serde(default)]
    citation: Vec<String>,
    #[serde(rename = "dateFiled")]
    date_filed: Option<String>,
}
#[derive(Deserialize)]
struct Legal {
    #[serde(default)]
    results: Items<LegalCase>,
}
#[derive(Deserialize)]
struct LegalCase {
    id: Option<u64>,
    slug: Option<String>,
    court: Option<String>,
    decision_type: Option<String>,
    date: Option<String>,
    #[serde(default)]
    snippets: Items<Snippet>,
}
#[derive(Deserialize)]
struct Snippet {
    text: String,
}
