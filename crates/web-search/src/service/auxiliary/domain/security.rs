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

use super::{
    date, source, DomainEngine as E, Items, WebSearchError, WebSearchResult, WebSearchService,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub(crate) struct KevCache {
    pub at: Instant,
    catalog: Kev,
}

impl WebSearchService {
    pub(super) async fn domain_security(
        &self,
        engine: E,
        query: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<WebSearchResult>, WebSearchError> {
        Ok(match engine {
            E::Nvd => {
                let d: Nvd = self
                    .domain_json(
                        "https://services.nvd.nist.gov/rest/json/cves/2.0",
                        &[("keywordSearch", query), ("resultsPerPage", "3")],
                        cancel,
                    )
                    .await?;
                d.vulnerabilities
                    .0
                    .into_iter()
                    .map(|r| {
                        let v = r.cve;
                        let description = v
                            .descriptions
                            .0
                            .iter()
                            .find(|d| d.lang == "en")
                            .or(v.descriptions.0.first())
                            .map(|d| d.value.as_str())
                            .unwrap_or(&v.id);
                        source(
                            "nvd",
                            format!("https://nvd.nist.gov/vuln/detail/{}", v.id),
                            format!("{}: {description}", v.id),
                            v.metrics
                                .and_then(|m| m.cvss_metric_v31.0.into_iter().next())
                                .map(|m| format!("CVSS {}", m.cvss_data.base_score))
                                .unwrap_or_default(),
                            date(v.published),
                        )
                    })
                    .collect()
            }
            E::Osv => {
                let (ecosystem, name) = query.trim().split_once(char::is_whitespace).ok_or(
                    WebSearchError::InvalidQuery(
                        "start with an ecosystem, followed by a package name",
                    ),
                )?;
                let ecosystem = match ecosystem.to_lowercase().as_str() {
                    "npm" => "npm",
                    "pypi" | "py" => "PyPI",
                    "go" => "Go",
                    "rust" | "crates" => "crates.io",
                    "maven" | "java" => "Maven",
                    "ruby" | "gem" | "rubygems" => "RubyGems",
                    "nuget" | "dotnet" => "NuGet",
                    "php" | "composer" => "Packagist",
                    _ => return Err(WebSearchError::InvalidQuery("unknown OSV ecosystem")),
                };
                let body = OsvRequest {
                    package: OsvPackage {
                        ecosystem,
                        name: name.trim(),
                    },
                };
                let d: Osv = super::decode(
                    &self
                        .send_bounded(
                            self.inner
                                .client
                                .post("https://api.osv.dev/v1/query")
                                .json(&body),
                            cancel,
                        )
                        .await?,
                )?;
                d.vulns
                    .0
                    .into_iter()
                    .take(3)
                    .map(|v| {
                        source(
                            "osv",
                            format!("https://osv.dev/vulnerability/{}", v.id),
                            format!(
                                "{}{}",
                                v.id,
                                if v.aliases.is_empty() {
                                    String::new()
                                } else {
                                    format!(
                                        " ({})",
                                        v.aliases
                                            .into_iter()
                                            .take(2)
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    )
                                }
                            ),
                            v.summary.unwrap_or_default(),
                            date(v.modified),
                        )
                    })
                    .collect()
            }
            E::Cisakev => {
                let cached = self
                    .inner
                    .kev_cache
                    .lock()
                    .as_ref()
                    .filter(|c| c.at.elapsed() < Duration::from_secs(86400))
                    .map(|c| c.catalog.clone());
                let catalog = match cached {
                    Some(catalog) => catalog,
                    None => {
                        let bytes = self.send_bounded_limit(self.inner.client.get("https://www.cisa.gov/sites/default/files/feeds/known_exploited_vulnerabilities.json"),cancel,8*1024*1024).await?;
                        let catalog: Kev = super::decode(&bytes)?;
                        *self.inner.kev_cache.lock() = Some(KevCache {
                            at: Instant::now(),
                            catalog: catalog.clone(),
                        });
                        catalog
                    }
                };
                let needle = query.to_lowercase();
                catalog
                    .vulnerabilities
                    .0
                    .into_iter()
                    .filter(|v| {
                        format!(
                            "{} {} {} {}",
                            v.cve_id, v.vendor_project, v.product, v.vulnerability_name
                        )
                        .to_lowercase()
                        .contains(&needle)
                    })
                    .take(3)
                    .map(|v| {
                        source(
                            "cisakev",
                            "https://www.cisa.gov/known-exploited-vulnerabilities-catalog",
                            format!("{}: {}", v.cve_id, v.vulnerability_name),
                            format!(
                                "{} {}{} · due {}",
                                v.vendor_project,
                                v.product,
                                if v.known_ransomware_campaign_use.as_deref() == Some("Known") {
                                    " · known ransomware use"
                                } else {
                                    ""
                                },
                                v.due_date
                            ),
                            Some(v.date_added),
                        )
                    })
                    .collect()
            }
            _ => return Err(WebSearchError::InvalidQuery("not a security engine")),
        })
    }
}
#[derive(Deserialize)]
struct Nvd {
    #[serde(default)]
    vulnerabilities: Items<NvdItem>,
}
#[derive(Deserialize)]
struct NvdItem {
    cve: Cve,
}
#[derive(Deserialize)]
struct Cve {
    id: String,
    #[serde(default)]
    descriptions: Items<Description>,
    published: Option<String>,
    metrics: Option<Metrics>,
}
#[derive(Deserialize)]
struct Description {
    lang: String,
    value: String,
}
#[derive(Deserialize)]
struct Metrics {
    #[serde(rename = "cvssMetricV31", default)]
    cvss_metric_v31: Items<Cvss>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cvss {
    cvss_data: Score,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Score {
    base_score: f64,
}
#[derive(Serialize)]
struct OsvRequest<'a> {
    package: OsvPackage<'a>,
}
#[derive(Serialize)]
struct OsvPackage<'a> {
    ecosystem: &'a str,
    name: &'a str,
}
#[derive(Deserialize)]
struct Osv {
    #[serde(default)]
    vulns: Items<Vulnerability>,
}
#[derive(Deserialize)]
struct Vulnerability {
    id: String,
    #[serde(default)]
    aliases: Vec<String>,
    summary: Option<String>,
    modified: Option<String>,
}
#[derive(Clone, Deserialize)]
struct Kev {
    #[serde(default)]
    vulnerabilities: Items<KevItem>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KevItem {
    #[serde(rename = "cveID")]
    cve_id: String,
    vendor_project: String,
    product: String,
    vulnerability_name: String,
    known_ransomware_campaign_use: Option<String>,
    due_date: String,
    date_added: String,
}
