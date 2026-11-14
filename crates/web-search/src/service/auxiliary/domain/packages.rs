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
    date, last, path, source, DomainEngine as E, Items, WebSearchError, WebSearchResult,
    WebSearchService,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde::Deserialize;

impl WebSearchService {
    pub(super) async fn domain_package(
        &self,
        engine: E,
        query: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<WebSearchResult>, WebSearchError> {
        let name = last(query)?;
        match engine {
            E::Pypi => {
                let d: Pypi = self
                    .domain_json(
                        &path("https://pypi.org", &["pypi", name, "json"])?,
                        &[],
                        cancel,
                    )
                    .await?;
                Ok(vec![source(
                    "pypi",
                    path("https://pypi.org", &["project", name])?,
                    format!("{name} {} on PyPI", d.info.version),
                    format!(
                        "{} · python: {}",
                        d.info.summary.unwrap_or_default(),
                        d.info.requires_python.unwrap_or_else(|| "?".into())
                    ),
                    None,
                )])
            }
            E::Crates => {
                let d: Crates = self
                    .domain_json(
                        &path("https://crates.io", &["api", "v1", "crates", name])?,
                        &[],
                        cancel,
                    )
                    .await?;
                let c = d.krate;
                Ok(vec![source(
                    "crates",
                    path("https://crates.io", &["crates", name])?,
                    format!(
                        "{} {} on crates.io",
                        c.name,
                        c.max_stable_version.unwrap_or(c.max_version)
                    ),
                    format!(
                        "{} · downloads {}",
                        c.description.unwrap_or_default(),
                        c.downloads
                    ),
                    date(c.updated_at),
                )])
            }
            E::GithubReleases => {
                let pattern = regex::Regex::new(r"([\w.-]+)/([\w.-]+)")
                    .map_err(|_| WebSearchError::InvalidResponse)?;
                let captures = pattern.captures(query).ok_or(WebSearchError::InvalidQuery(
                    "query must contain owner/repo",
                ))?;
                let (owner, repo) = (&captures[1], &captures[2]);
                let d: Release = self
                    .domain_json(
                        &path(
                            "https://api.github.com",
                            &["repos", owner, repo, "releases", "latest"],
                        )?,
                        &[],
                        cancel,
                    )
                    .await?;
                Ok(vec![source(
                    "github_releases",
                    d.html_url,
                    format!("{owner}/{repo} {}", d.tag_name),
                    d.body.unwrap_or_default(),
                    date(d.published_at),
                )])
            }
            E::Endoflife => {
                let name = name.to_lowercase();
                let cycles: Items<Cycle> = self
                    .domain_json(
                        &path("https://endoflife.date", &["api", &format!("{name}.json")])?,
                        &[],
                        cancel,
                    )
                    .await?;
                let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
                Ok(cycles
                    .0
                    .into_iter()
                    .filter(|c| match &c.eol {
                        Eol::Flag(v) => !v,
                        Eol::Date(d) => d > &today,
                    })
                    .take(3)
                    .map(|c| {
                        source(
                            "endoflife",
                            format!("https://endoflife.date/{name}"),
                            format!("{name} {}", c.cycle),
                            format!(
                                "released {} · EOL {}",
                                c.release_date.as_deref().unwrap_or("?"),
                                match c.eol {
                                    Eol::Date(d) => d,
                                    Eol::Flag(_) => "not yet".into(),
                                }
                            ),
                            c.release_date,
                        )
                    })
                    .collect())
            }
            E::Huggingface => {
                let d: Items<Model> = self
                    .domain_json(
                        "https://huggingface.co/api/models",
                        &[("search", query), ("limit", "3")],
                        cancel,
                    )
                    .await?;
                Ok(d.0
                    .into_iter()
                    .map(|m| {
                        source(
                            "huggingface",
                            format!("https://huggingface.co/{}", m.model_id),
                            m.model_id,
                            format!(
                                "{} · downloads {} · likes {}",
                                m.pipeline_tag.unwrap_or_else(|| "model".into()),
                                m.downloads.unwrap_or(0),
                                m.likes.unwrap_or(0)
                            ),
                            date(m.last_modified),
                        )
                    })
                    .collect())
            }
            E::Registries => self.domain_registry(query, cancel).await,
            _ => Err(WebSearchError::InvalidQuery("not a package engine")),
        }
    }
}

#[derive(Deserialize)]
struct Pypi {
    info: PypiInfo,
}
#[derive(Deserialize)]
struct PypiInfo {
    version: String,
    summary: Option<String>,
    requires_python: Option<String>,
}
#[derive(Deserialize)]
struct Crates {
    #[serde(rename = "crate")]
    krate: CrateInfo,
}
#[derive(Deserialize)]
struct CrateInfo {
    name: String,
    max_version: String,
    max_stable_version: Option<String>,
    description: Option<String>,
    downloads: u64,
    updated_at: Option<String>,
}
#[derive(Deserialize)]
struct Release {
    html_url: String,
    tag_name: String,
    body: Option<String>,
    published_at: Option<String>,
}
#[derive(Deserialize)]
struct Cycle {
    cycle: String,
    release_date: Option<String>,
    eol: Eol,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Eol {
    Flag(bool),
    Date(String),
}
#[derive(Deserialize)]
struct Model {
    #[serde(rename = "modelId")]
    model_id: String,
    pipeline_tag: Option<String>,
    downloads: Option<u64>,
    likes: Option<u64>,
    #[serde(rename = "lastModified")]
    last_modified: Option<String>,
}
