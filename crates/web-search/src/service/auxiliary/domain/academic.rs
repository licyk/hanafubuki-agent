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
    date, path, source, DomainEngine as E, Items, WebSearchError, WebSearchResult, WebSearchService,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde::Deserialize;
use std::collections::BTreeMap;

impl WebSearchService {
    pub(super) async fn domain_academic(
        &self,
        engine: E,
        query: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<WebSearchResult>, WebSearchError> {
        Ok(match engine {
            E::Crossref => self.crossref(query, cancel).await?,
            E::Openalex => {
                let d: Openalex = self
                    .domain_json(
                        "https://api.openalex.org/works",
                        &[("search", query), ("per-page", "3")],
                        cancel,
                    )
                    .await?;
                d.results
                    .0
                    .into_iter()
                    .map(|p| {
                        source(
                            "openalex",
                            p.doi.unwrap_or(p.id),
                            p.display_name,
                            format!(
                                "{} · cited_by {}",
                                if p.abstract_inverted_index.is_some() {
                                    "abstract available"
                                } else {
                                    ""
                                },
                                p.cited_by_count
                            ),
                            p.publication_year.map(|y| y.to_string()),
                        )
                    })
                    .collect()
            }
            E::S2 => {
                let d: Semantic = self
                    .domain_json(
                        "https://api.semanticscholar.org/graph/v1/paper/search",
                        &[
                            ("query", query),
                            ("limit", "3"),
                            ("fields", "title,year,url,abstract"),
                        ],
                        cancel,
                    )
                    .await?;
                d.data
                    .0
                    .into_iter()
                    .map(|p| {
                        source(
                            "s2",
                            p.url,
                            p.title,
                            p.abstract_text.unwrap_or_default(),
                            p.year.map(|y| y.to_string()),
                        )
                    })
                    .collect()
            }
            E::Arxiv => {
                let bytes = self
                    .send_bounded(
                        self.inner
                            .client
                            .get("https://export.arxiv.org/api/query")
                            .query(&[
                                ("search_query", format!("all:{query}")),
                                ("max_results", "3".into()),
                            ]),
                        cancel,
                    )
                    .await?;
                atom(&String::from_utf8_lossy(&bytes), "arxiv")
            }
            E::Pubmed => self.pubmed(query, cancel).await?,
            E::Europepmc => {
                let d: Europe = self
                    .domain_json(
                        "https://www.ebi.ac.uk/europepmc/webservices/rest/search",
                        &[("query", query), ("format", "json"), ("pageSize", "3")],
                        cancel,
                    )
                    .await?;
                d.result_list
                    .result
                    .0
                    .into_iter()
                    .map(|p| {
                        source(
                            "europepmc",
                            p.doi
                                .map(|doi| format!("https://doi.org/{doi}"))
                                .unwrap_or_else(|| {
                                    format!("https://europepmc.org/article/{}/{}", p.source, p.id)
                                }),
                            p.title,
                            format!(
                                "{} · {} · {}",
                                p.author_string.unwrap_or_default(),
                                p.journal_title.unwrap_or(p.source),
                                p.pub_year.unwrap_or_default()
                            ),
                            date(p.first_publication_date),
                        )
                    })
                    .collect()
            }
            E::Dblp => {
                let d: Dblp = self
                    .domain_json(
                        "https://dblp.org/search/publ/api",
                        &[("q", query), ("format", "json"), ("h", "3")],
                        cancel,
                    )
                    .await?;
                d.result
                    .hits
                    .hit
                    .0
                    .into_iter()
                    .map(|h| {
                        let p = h.info;
                        source(
                            "dblp",
                            p.url.unwrap_or_else(|| "https://dblp.org".into()),
                            p.title,
                            format!(
                                "{} {}",
                                p.venue.unwrap_or_default(),
                                p.year.as_deref().unwrap_or("")
                            ),
                            p.year,
                        )
                    })
                    .collect()
            }
            E::Zenodo => {
                let d: Zenodo = self
                    .domain_json(
                        "https://zenodo.org/api/records",
                        &[("q", query), ("size", "3")],
                        cancel,
                    )
                    .await?;
                d.hits
                    .hits
                    .0
                    .into_iter()
                    .map(|p| {
                        source(
                            "zenodo",
                            format!("https://zenodo.org/records/{}", p.id),
                            p.metadata.title,
                            plain(&p.metadata.description.unwrap_or_default()),
                            date(p.metadata.publication_date),
                        )
                    })
                    .collect()
            }
            E::Doaj => {
                let d: Doaj = self
                    .domain_json(
                        &path("https://doaj.org", &["api", "search", "articles", query])?,
                        &[("pageSize", "3")],
                        cancel,
                    )
                    .await?;
                d.results
                    .0
                    .into_iter()
                    .map(|r| {
                        let p = r.bibjson;
                        source(
                            "doaj",
                            p.link
                                .0
                                .into_iter()
                                .next()
                                .map(|l| l.url)
                                .unwrap_or_else(|| "https://doaj.org".into()),
                            p.title,
                            format!(
                                "{} · {}",
                                p.journal.map(|j| j.title).unwrap_or_default(),
                                p.year.as_deref().unwrap_or("")
                            ),
                            p.year,
                        )
                    })
                    .collect()
            }
            _ => return Err(WebSearchError::InvalidQuery("not an academic engine")),
        })
    }
    async fn pubmed(
        &self,
        query: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<WebSearchResult>, WebSearchError> {
        Ok({
            let search: PubmedSearch = self
                .domain_json(
                    "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi",
                    &[
                        ("db", "pubmed"),
                        ("retmode", "json"),
                        ("retmax", "3"),
                        ("term", query),
                    ],
                    cancel,
                )
                .await?;
            let ids = search.esearchresult.idlist;
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let summary: PubmedSummary = self
                .domain_json(
                    "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esummary.fcgi",
                    &[
                        ("db", "pubmed"),
                        ("retmode", "json"),
                        ("id", &ids.join(",")),
                    ],
                    cancel,
                )
                .await?;
            ids.into_iter()
                .filter_map(|id| {
                    let Some(SummaryItem::Paper(p)) = summary.result.get(&id) else {
                        return None;
                    };
                    Some(source(
                        "pubmed",
                        format!("https://pubmed.ncbi.nlm.nih.gov/{id}/"),
                        p.title.clone(),
                        format!("{} · {}", p.source, p.pubdate),
                        None,
                    ))
                })
                .collect()
        })
    }

    async fn crossref(
        &self,
        query: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<WebSearchResult>, WebSearchError> {
        Ok({
            let d: Crossref = self
                .domain_json(
                    "https://api.crossref.org/works",
                    &[
                        ("query", query),
                        ("rows", "3"),
                        ("select", "DOI,title,issued"),
                    ],
                    cancel,
                )
                .await?;
            d.message
                .items
                .0
                .into_iter()
                .map(|p| {
                    source(
                        "crossref",
                        format!("https://doi.org/{}", p.doi),
                        p.title.into_iter().next().unwrap_or_else(|| p.doi.clone()),
                        format!("DOI {}", p.doi),
                        p.issued
                            .and_then(|d| d.date_parts.into_iter().next())
                            .and_then(|d| d.into_iter().next())
                            .map(|y| y.to_string()),
                    )
                })
                .collect()
        })
    }
}

pub(super) fn atom(xml: &str, engine: &str) -> Vec<WebSearchResult> {
    let doc = dom_query::Document::from(xml);
    doc.select("entry")
        .iter()
        .take(3)
        .filter_map(|entry| {
            let pick = |tag: &str| {
                entry
                    .select(tag)
                    .text()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            let url = if engine == "arxiv" {
                pick("id")
            } else {
                let links: Vec<String> = entry
                    .select("link")
                    .iter()
                    .filter_map(|l| l.attr("href").map(|s| s.to_string()))
                    .collect();
                links
                    .iter()
                    .find(|s| !s.contains("/id/"))
                    .or(links.first())?
                    .replacen("http:", "https:", 1)
            };
            Some(source(
                engine,
                url,
                pick("title"),
                pick("summary"),
                date(Some(pick(if engine == "arxiv" {
                    "published"
                } else {
                    "updated"
                }))),
            ))
        })
        .collect()
}

#[derive(Deserialize)]
struct Crossref {
    message: CrossrefMessage,
}
#[derive(Deserialize)]
struct CrossrefMessage {
    #[serde(default)]
    items: Items<CrossrefPaper>,
}
#[derive(Deserialize)]
struct CrossrefPaper {
    #[serde(rename = "DOI")]
    doi: String,
    #[serde(default)]
    title: Vec<String>,
    issued: Option<Issued>,
}
#[derive(Deserialize)]
struct Issued {
    #[serde(rename = "date-parts", default)]
    date_parts: Vec<Vec<u64>>,
}
#[derive(Deserialize)]
struct Openalex {
    #[serde(default)]
    results: Items<OpenalexPaper>,
}
#[derive(Deserialize)]
struct OpenalexPaper {
    id: String,
    doi: Option<String>,
    display_name: String,
    abstract_inverted_index: Option<serde::de::IgnoredAny>,
    #[serde(default)]
    cited_by_count: u64,
    publication_year: Option<u64>,
}
#[derive(Deserialize)]
struct Semantic {
    #[serde(default)]
    data: Items<SemanticPaper>,
}
#[derive(Deserialize)]
struct SemanticPaper {
    url: String,
    title: String,
    #[serde(rename = "abstract")]
    abstract_text: Option<String>,
    year: Option<u64>,
}
#[derive(Deserialize)]
struct PubmedSearch {
    esearchresult: IdList,
}
#[derive(Deserialize)]
struct IdList {
    #[serde(default)]
    idlist: Vec<String>,
}
#[derive(Deserialize)]
struct PubmedSummary {
    #[serde(default)]
    result: BTreeMap<String, SummaryItem>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum SummaryItem {
    Paper(PubmedPaper),
    Other(serde::de::IgnoredAny),
}
#[derive(Deserialize)]
struct PubmedPaper {
    title: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    pubdate: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Europe {
    result_list: EuropeResults,
}
#[derive(Deserialize)]
struct EuropeResults {
    #[serde(default)]
    result: Items<EuropePaper>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EuropePaper {
    id: String,
    source: String,
    doi: Option<String>,
    title: String,
    author_string: Option<String>,
    journal_title: Option<String>,
    pub_year: Option<String>,
    first_publication_date: Option<String>,
}
#[derive(Deserialize)]
struct Dblp {
    result: DblpResult,
}
#[derive(Deserialize)]
struct DblpResult {
    hits: DblpHits,
}
#[derive(Deserialize)]
struct DblpHits {
    #[serde(default)]
    hit: Items<DblpHit>,
}
#[derive(Deserialize)]
struct DblpHit {
    info: DblpInfo,
}
#[derive(Deserialize)]
struct DblpInfo {
    title: String,
    url: Option<String>,
    venue: Option<String>,
    year: Option<String>,
}
#[derive(Deserialize)]
struct Zenodo {
    hits: ZenodoHits,
}
#[derive(Deserialize)]
struct ZenodoHits {
    #[serde(default)]
    hits: Items<ZenodoRecord>,
}
#[derive(Deserialize)]
struct ZenodoRecord {
    id: u64,
    metadata: ZenodoMetadata,
}
#[derive(Deserialize)]
struct ZenodoMetadata {
    title: String,
    description: Option<String>,
    publication_date: Option<String>,
}
#[derive(Deserialize)]
struct Doaj {
    #[serde(default)]
    results: Items<DoajResult>,
}
#[derive(Deserialize)]
struct DoajResult {
    bibjson: Bibliography,
}
#[derive(Deserialize)]
struct Bibliography {
    title: String,
    #[serde(default)]
    link: Items<Link>,
    journal: Option<Journal>,
    year: Option<String>,
}
#[derive(Deserialize)]
struct Link {
    url: String,
}
#[derive(Deserialize)]
struct Journal {
    title: String,
}

#[cfg(test)]
mod tests;
