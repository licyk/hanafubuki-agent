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

use super::{source, WebSearchError, WebSearchResult, WebSearchService};
use chrono::Datelike;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone)]
pub(crate) struct ImfCatalog {
    indicators: BTreeMap<String, Label>,
    countries: BTreeMap<String, Label>,
}
impl WebSearchService {
    pub(super) async fn imf(
        &self,
        query: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<WebSearchResult>, WebSearchError> {
        let cached = self.inner.imf_catalog.lock().clone();
        let catalog = match cached {
            Some(catalog) => catalog,
            None => {
                let (ind, ctry): (Indicators, Countries) = tokio::try_join!(
                    self.domain_json(
                        "https://www.imf.org/external/datamapper/api/v1/indicators",
                        &[],
                        cancel
                    ),
                    self.domain_json(
                        "https://www.imf.org/external/datamapper/api/v1/countries",
                        &[],
                        cancel
                    )
                )?;
                let catalog = ImfCatalog {
                    indicators: ind.indicators,
                    countries: ctry.countries,
                };
                *self.inner.imf_catalog.lock() = Some(catalog.clone());
                catalog
            }
        };
        let lower = query.to_lowercase();
        let countries: Vec<_> = catalog
            .countries
            .iter()
            .filter(|(_, v)| !v.label.is_empty() && lower.contains(&v.label.to_lowercase()))
            .collect();
        let words: Vec<_> = lower
            .split_whitespace()
            .filter(|w| {
                w.len() > 2
                    && !countries
                        .iter()
                        .any(|(_, c)| c.label.to_lowercase().contains(w))
            })
            .collect();
        let mut scored: Vec<_> = catalog
            .indicators
            .iter()
            .filter_map(|(code, v)| {
                let label = v.label.to_lowercase();
                let score = words.iter().filter(|w| label.contains(**w)).count();
                (score > 0).then_some((code, v, score))
            })
            .collect();
        scored.sort_by_key(|a| std::cmp::Reverse(a.2));
        if scored.is_empty() {
            return Err(WebSearchError::NoResults);
        }
        let targets: Vec<&str> = if countries.is_empty() {
            vec!["USA", "CHN", "WLD"]
        } else {
            countries.iter().map(|(code, _)| code.as_str()).collect()
        };
        let mut result = Vec::new();
        for (code, indicator, _) in scored.into_iter().take(2) {
            let mut series: BTreeMap<String, Series> = self
                .domain_json(
                    &format!("https://www.imf.org/external/datamapper/api/v1/{code}"),
                    &[],
                    cancel,
                )
                .await?;
            let Some(series) = series.remove(code) else {
                continue;
            };
            for country in targets.iter().take(2) {
                let Some(years) = series.countries.get(*country) else {
                    continue;
                };
                let tail = years
                    .iter()
                    .filter(|(year, _)| {
                        year.parse::<i32>()
                            .is_ok_and(|y| y >= chrono::Utc::now().year() - 8)
                    })
                    .map(|(y, v)| format!("{y}:{v}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                result.push(source(
                    "imf",
                    format!("https://www.imf.org/external/datamapper/{code}/@country/{country}"),
                    format!(
                        "{} · {}",
                        catalog
                            .countries
                            .get(*country)
                            .map(|c| c.label.as_str())
                            .unwrap_or(country),
                        indicator.label
                    ),
                    tail,
                    None,
                ));
            }
        }
        Ok(result)
    }
}
#[derive(Clone, Deserialize)]
struct Label {
    label: String,
}
#[derive(Deserialize)]
struct Indicators {
    #[serde(default)]
    indicators: BTreeMap<String, Label>,
}
#[derive(Deserialize)]
struct Countries {
    #[serde(default)]
    countries: BTreeMap<String, Label>,
}
#[derive(Deserialize)]
struct Series {
    #[serde(default)]
    countries: BTreeMap<String, BTreeMap<String, f64>>,
}
