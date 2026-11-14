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

//! Typed adapters for the public domain APIs in dsh-free-search 6ab5168 (MIT).
mod academic;
mod facts;
mod finance;
mod imf;
mod news_legal;
mod packages;
mod registries;
mod security;
pub(crate) use imf::ImfCatalog;
pub(crate) use security::KevCache;

use super::{decode, source, wire::Items, SourceResponse};
use crate::{WebSearchError, WebSearchResult, WebSearchService};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainEngine {
    Pypi,
    Crates,
    GithubReleases,
    Endoflife,
    Registries,
    Huggingface,
    Crossref,
    Openalex,
    S2,
    Arxiv,
    Pubmed,
    Europepmc,
    Dblp,
    Zenodo,
    Doaj,
    Nvd,
    Osv,
    Cisakev,
    Yahoofinance,
    Coingecko,
    Frankfurter,
    SecEdgar,
    Imf,
    Cfets,
    Cninfo,
    Worldbank,
    Wikidata,
    Holidays,
    Worldclock,
    Openmeteo,
    Rdap,
    Gleif,
    Googlenews,
    Gdelt,
    Aihot,
    Federalregister,
    Courtlistener,
    Openlegaldata,
    UkLegislation,
    So360,
    Sogou,
}
impl DomainEngine {
    pub const ALL: [Self; 41] = [
        Self::Pypi,
        Self::Crates,
        Self::GithubReleases,
        Self::Endoflife,
        Self::Registries,
        Self::Huggingface,
        Self::Crossref,
        Self::Openalex,
        Self::S2,
        Self::Arxiv,
        Self::Pubmed,
        Self::Europepmc,
        Self::Dblp,
        Self::Zenodo,
        Self::Doaj,
        Self::Nvd,
        Self::Osv,
        Self::Cisakev,
        Self::Yahoofinance,
        Self::Coingecko,
        Self::Frankfurter,
        Self::SecEdgar,
        Self::Imf,
        Self::Cfets,
        Self::Cninfo,
        Self::Worldbank,
        Self::Wikidata,
        Self::Holidays,
        Self::Worldclock,
        Self::Openmeteo,
        Self::Rdap,
        Self::Gleif,
        Self::Googlenews,
        Self::Gdelt,
        Self::Aihot,
        Self::Federalregister,
        Self::Courtlistener,
        Self::Openlegaldata,
        Self::UkLegislation,
        Self::So360,
        Self::Sogou,
    ];
}
impl WebSearchService {
    pub async fn search_domain(
        &self,
        engine: DomainEngine,
        query: &str,
        cancellation: CancellationToken,
    ) -> Result<SourceResponse, WebSearchError> {
        let sources = match engine {
            DomainEngine::Pypi
            | DomainEngine::Crates
            | DomainEngine::GithubReleases
            | DomainEngine::Endoflife
            | DomainEngine::Registries
            | DomainEngine::Huggingface => {
                self.domain_package(engine, query, &cancellation).await?
            }
            DomainEngine::Crossref
            | DomainEngine::Openalex
            | DomainEngine::S2
            | DomainEngine::Arxiv
            | DomainEngine::Pubmed
            | DomainEngine::Europepmc
            | DomainEngine::Dblp
            | DomainEngine::Zenodo
            | DomainEngine::Doaj => self.domain_academic(engine, query, &cancellation).await?,
            DomainEngine::Nvd | DomainEngine::Osv | DomainEngine::Cisakev => {
                self.domain_security(engine, query, &cancellation).await?
            }
            DomainEngine::Googlenews
            | DomainEngine::Gdelt
            | DomainEngine::Aihot
            | DomainEngine::Federalregister
            | DomainEngine::Courtlistener
            | DomainEngine::Openlegaldata
            | DomainEngine::UkLegislation => {
                self.domain_news_legal(engine, query, &cancellation).await?
            }
            DomainEngine::Yahoofinance
            | DomainEngine::Coingecko
            | DomainEngine::Frankfurter
            | DomainEngine::SecEdgar
            | DomainEngine::Imf
            | DomainEngine::Cfets
            | DomainEngine::Cninfo => self.domain_finance(engine, query, &cancellation).await?,
            DomainEngine::Worldbank
            | DomainEngine::Wikidata
            | DomainEngine::Holidays
            | DomainEngine::Worldclock
            | DomainEngine::Openmeteo
            | DomainEngine::Rdap
            | DomainEngine::Gleif => self.domain_facts(engine, query, &cancellation).await?,
            DomainEngine::So360 | DomainEngine::Sogou => {
                let request = crate::WebSearchRequest {
                    engines: Vec::new(),
                    query: query.into(),
                    maximum_results: 5,
                    provider: if engine == DomainEngine::So360 {
                        crate::WebSearchProvider::So360
                    } else {
                        crate::WebSearchProvider::Sogou
                    },
                    category: Default::default(),
                    time_range: None,
                    region: None,
                };
                self.fetch_provider(request.provider, &request, &cancellation)
                    .await?
                    .results
            }
        };
        Ok(SourceResponse::new(
            sources
                .into_iter()
                .filter(|r| !self.blocked(&r.url))
                .collect(),
            5,
        ))
    }
    async fn domain_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        params: &[(&str, &str)],
        cancellation: &CancellationToken,
    ) -> Result<T, WebSearchError> {
        decode(
            &self
                .send_bounded(self.inner.client.get(url).query(params), cancellation)
                .await?,
        )
    }
}
fn path(base: &str, segments: &[&str]) -> Result<String, WebSearchError> {
    let mut url = Url::parse(base)
        .map_err(|_| WebSearchError::InvalidConfiguration("invalid domain API URL"))?;
    url.path_segments_mut()
        .map_err(|_| WebSearchError::InvalidConfiguration("invalid domain API URL"))?
        .extend(segments);
    Ok(url.into())
}
fn last(query: &str) -> Result<&str, WebSearchError> {
    query
        .split_whitespace()
        .last()
        .ok_or(WebSearchError::InvalidQuery(
            "a package or product name is required",
        ))
}
fn date(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.chars().take(10).collect())
        .filter(|s: &String| !s.is_empty())
}
