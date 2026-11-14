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

use super::{date, last, path, source, Items, WebSearchError, WebSearchResult, WebSearchService};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde::Deserialize;
use std::collections::BTreeMap;

impl WebSearchService {
    pub(super) async fn domain_registry(
        &self,
        query: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<WebSearchResult>, WebSearchError> {
        let name = last(query)?;
        let lower = query.to_lowercase();
        let engine = [
            ("maven", &["maven", "gradle", "java", "kotlin"][..]),
            ("rubygems", &["ruby", "gem"]),
            ("go", &["go", "golang"]),
            ("brew", &["brew", "homebrew"]),
            ("docker", &["docker", "image", "容器"]),
            ("packagist", &["packagist", "composer", "php"]),
            ("nuget", &["nuget", "dotnet", ".net", "c#"]),
        ]
        .into_iter()
        .find(|(_, words)| {
            words
                .iter()
                .any(|w| lower.split_whitespace().any(|s| s == *w))
        })
        .map(|(id, _)| id)
        .ok_or(WebSearchError::InvalidQuery(
            "name an ecosystem and package: maven/rubygems/go/brew/docker/packagist/nuget",
        ))?;
        let results = match engine {
            "maven" => {
                let d: Maven = self
                    .domain_json(
                        "https://search.maven.org/solrsearch/select",
                        &[("q", name), ("rows", "3"), ("wt", "json")],
                        cancel,
                    )
                    .await?;
                d.response
                    .docs
                    .0
                    .into_iter()
                    .take(1)
                    .map(|p| {
                        source(
                            "registries",
                            format!("https://central.sonatype.com/artifact/{}/{}", p.g, p.a),
                            format!("{}:{} {} on Maven Central", p.g, p.a, p.latest_version),
                            format!("{} versions published", p.version_count),
                            p.timestamp
                                .and_then(chrono::DateTime::from_timestamp_millis)
                                .map(|d| d.format("%Y-%m-%d").to_string()),
                        )
                    })
                    .collect()
            }
            "rubygems" => {
                let p: Ruby = self
                    .domain_json(
                        &path(
                            "https://rubygems.org",
                            &["api", "v1", "gems", &format!("{name}.json")],
                        )?,
                        &[],
                        cancel,
                    )
                    .await?;
                vec![source(
                    "registries",
                    path("https://rubygems.org", &["gems", name])?,
                    format!("{} {} on RubyGems", p.name, p.version),
                    p.info.unwrap_or_default(),
                    date(p.version_created_at),
                )]
            }
            "go" => self.go_registry(query, cancel).await?,
            "brew" => {
                let p: Brew = self
                    .domain_json(
                        &path(
                            "https://formulae.brew.sh",
                            &["api", "formula", &format!("{name}.json")],
                        )?,
                        &[],
                        cancel,
                    )
                    .await?;
                vec![source(
                    "registries",
                    path("https://formulae.brew.sh", &["formula", name])?,
                    format!("{} {} on Homebrew", p.name, p.versions.stable),
                    format!(
                        "{} · license {}",
                        p.desc.unwrap_or_default(),
                        p.license.unwrap_or_else(|| "?".into())
                    ),
                    None,
                )]
            }
            "docker" => {
                let (namespace, repository) = name.split_once('/').unwrap_or(("library", name));
                let p: Docker = self
                    .domain_json(
                        &path(
                            "https://hub.docker.com",
                            &["v2", "repositories", namespace, repository],
                        )?,
                        &[],
                        cancel,
                    )
                    .await?;
                vec![source(
                    "registries",
                    path("https://hub.docker.com", &["r", namespace, repository])?,
                    format!(
                        "{} on Docker Hub",
                        p.name.unwrap_or_else(|| repository.into())
                    ),
                    format!(
                        "{} · {} pulls",
                        p.description.unwrap_or_default(),
                        p.pull_count.unwrap_or(0)
                    ),
                    date(p.last_updated),
                )]
            }
            "packagist" => {
                let name = name.to_lowercase();
                let (vendor, package) = name
                    .split_once('/')
                    .ok_or(WebSearchError::InvalidQuery("give vendor/package"))?;
                let mut d: Packagist = self
                    .domain_json(
                        &path(
                            "https://repo.packagist.org",
                            &["p2", vendor, &format!("{package}.json")],
                        )?,
                        &[],
                        cancel,
                    )
                    .await?;
                d.packages
                    .remove(&name)
                    .unwrap_or_default()
                    .0
                    .into_iter()
                    .take(1)
                    .map(|p| {
                        source(
                            "registries",
                            format!("https://packagist.org/packages/{name}"),
                            format!("{name} {} on Packagist", p.version),
                            p.description.unwrap_or_default(),
                            date(p.time),
                        )
                    })
                    .collect()
            }
            "nuget" => {
                let d: Nuget = self
                    .domain_json(
                        "https://azuresearch-usnc.nuget.org/query",
                        &[("q", name), ("take", "3")],
                        cancel,
                    )
                    .await?;
                d.data
                    .0
                    .into_iter()
                    .take(2)
                    .map(|p| {
                        source(
                            "registries",
                            format!("https://www.nuget.org/packages/{}/{}", p.id, p.version),
                            format!("{} {} on NuGet", p.id, p.version),
                            p.description.unwrap_or_default(),
                            None,
                        )
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        Ok(results)
    }
    async fn go_registry(
        &self,
        query: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<WebSearchResult>, WebSearchError> {
        Ok({
            let module = query
                .split_whitespace()
                .find(|s| {
                    [
                        "github.com/",
                        "gitlab.com/",
                        "golang.org/",
                        "google.golang.org/",
                        "k8s.io/",
                    ]
                    .iter()
                    .any(|prefix| s.starts_with(prefix))
                })
                .ok_or(WebSearchError::InvalidQuery("give a Go module path"))?;
            let escaped = module
                .trim_end_matches('/')
                .chars()
                .map(|c| {
                    if c.is_ascii_uppercase() {
                        format!("!{}", c.to_ascii_lowercase())
                    } else {
                        c.to_string()
                    }
                })
                .collect::<String>();
            let segments: Vec<_> = escaped.split('/').chain(["@latest"]).collect();
            let p: Go = self
                .domain_json(&path("https://proxy.golang.org", &segments)?, &[], cancel)
                .await?;
            vec![source(
                "registries",
                format!("https://pkg.go.dev/{module}@{}", p.version),
                format!("{module} {} (Go module)", p.version),
                "latest tag",
                date(p.time),
            )]
        })
    }
}
#[derive(Deserialize)]
struct Maven {
    response: MavenResponse,
}
#[derive(Deserialize)]
struct MavenResponse {
    #[serde(default)]
    docs: Items<MavenPackage>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MavenPackage {
    g: String,
    a: String,
    latest_version: String,
    version_count: u64,
    timestamp: Option<i64>,
}
#[derive(Deserialize)]
struct Ruby {
    name: String,
    version: String,
    info: Option<String>,
    version_created_at: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Go {
    version: String,
    time: Option<String>,
}
#[derive(Deserialize)]
struct Brew {
    name: String,
    versions: BrewVersions,
    desc: Option<String>,
    license: Option<String>,
}
#[derive(Deserialize)]
struct BrewVersions {
    stable: String,
}
#[derive(Deserialize)]
struct Docker {
    name: Option<String>,
    description: Option<String>,
    pull_count: Option<u64>,
    last_updated: Option<String>,
}
#[derive(Deserialize)]
struct Packagist {
    #[serde(default)]
    packages: BTreeMap<String, Items<PhpPackage>>,
}
#[derive(Deserialize)]
struct PhpPackage {
    version: String,
    description: Option<String>,
    time: Option<String>,
}
#[derive(Deserialize)]
struct Nuget {
    #[serde(default)]
    data: Items<NugetPackage>,
}
#[derive(Deserialize)]
struct NugetPackage {
    id: String,
    version: String,
    description: Option<String>,
}
