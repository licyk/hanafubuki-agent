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

// Derived from dsh-free-search 6ab5168 (MIT), platform_search.
use super::{decode, plain, source, wire::Items, SourceResponse};
use crate::{WebSearchError, WebSearchService};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Github,
    V2ex,
    Bilibili,
    Reddit,
    Hackernews,
    Stackoverflow,
    Wikipedia,
    Npm,
    Juejin,
    Douban,
    Qiita,
    Youtube,
    Vimeo,
    X,
}
impl Platform {
    pub const ALL: [Self; 14] = [
        Self::Github,
        Self::V2ex,
        Self::Bilibili,
        Self::Reddit,
        Self::Hackernews,
        Self::Stackoverflow,
        Self::Wikipedia,
        Self::Npm,
        Self::Juejin,
        Self::Douban,
        Self::Qiita,
        Self::Youtube,
        Self::Vimeo,
        Self::X,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::V2ex => "v2ex",
            Self::Bilibili => "bilibili",
            Self::Reddit => "reddit",
            Self::Hackernews => "hackernews",
            Self::Stackoverflow => "stackoverflow",
            Self::Wikipedia => "wikipedia",
            Self::Npm => "npm",
            Self::Juejin => "juejin",
            Self::Douban => "douban",
            Self::Qiita => "qiita",
            Self::Youtube => "youtube",
            Self::Vimeo => "vimeo",
            Self::X => "x",
        }
    }
}

impl WebSearchService {
    pub async fn search_platform(
        &self,
        platform: Platform,
        query: &str,
        maximum: usize,
        cancellation: CancellationToken,
    ) -> Result<SourceResponse, WebSearchError> {
        let mut response = self
            .platform_response(platform, query, maximum, cancellation)
            .await?;
        response.sources.retain(|r| !self.blocked(&r.url));
        Ok(response)
    }
    async fn platform_response(
        &self,
        platform: Platform,
        query: &str,
        maximum: usize,
        cancellation: CancellationToken,
    ) -> Result<SourceResponse, WebSearchError> {
        if !self.inner.config.search.platforms.contains(&platform) {
            return Err(WebSearchError::InvalidConfiguration("platform is disabled"));
        }
        let maximum = maximum.clamp(1, 50);
        if platform == Platform::X {
            return self.search_x(query, maximum, &cancellation).await;
        }
        if platform == Platform::Youtube {
            return self.youtube(query, maximum, &cancellation).await;
        }
        if platform == Platform::Vimeo {
            return self.vimeo(query, maximum, &cancellation).await;
        }
        let (url, params) = endpoint(platform, query, maximum, &self.inner.config.search.language)?;
        let mut request = self.inner.client.get(url).query(&params);
        if platform == Platform::Bilibili {
            request = request.header("referer", "https://www.bilibili.com");
        }
        let bytes = self.send_bounded(request, &cancellation).await?;
        parse(
            platform,
            query,
            maximum,
            &self.inner.config.search.language,
            &bytes,
        )
    }
}

fn endpoint(
    platform: Platform,
    query: &str,
    maximum: usize,
    language: &str,
) -> Result<(String, Vec<(String, String)>), WebSearchError> {
    let n = maximum.to_string();
    let (url, params) = match platform {
        Platform::Github => (
            "https://api.github.com/search/repositories".into(),
            vec![("q", query), ("per_page", &n)],
        ),
        Platform::V2ex => ("https://www.v2ex.com/api/topics/hot.json".into(), vec![]),
        Platform::Bilibili => (
            "https://api.bilibili.com/x/web-interface/search/all/v2".into(),
            vec![("keyword", query)],
        ),
        Platform::Reddit => (
            "https://old.reddit.com/search.json".into(),
            vec![("q", query), ("limit", &n), ("sort", "relevance")],
        ),
        Platform::Hackernews => (
            "https://hn.algolia.com/api/v1/search".into(),
            vec![("query", query), ("hitsPerPage", &n)],
        ),
        Platform::Stackoverflow => (
            "https://api.stackexchange.com/2.3/search/advanced".into(),
            vec![
                ("order", "desc"),
                ("sort", "relevance"),
                ("q", query),
                ("site", "stackoverflow"),
                ("pagesize", &n),
                ("filter", "!nNPvSNVZJS"),
            ],
        ),
        Platform::Wikipedia => (
            format!(
                "https://{}.wikipedia.org/w/api.php",
                wiki_language(language)
            ),
            vec![
                ("action", "query"),
                ("list", "search"),
                ("srsearch", query),
                ("format", "json"),
                ("srlimit", &n),
            ],
        ),
        Platform::Npm => (
            "https://registry.npmjs.com/-/v1/search".into(),
            vec![("text", query), ("size", &n)],
        ),
        Platform::Juejin => (
            "https://api.juejin.cn/search_api/v1/search".into(),
            vec![
                ("query", query),
                ("id_type", "0"),
                ("cursor", "0"),
                ("limit", &n),
            ],
        ),
        Platform::Douban => (
            "https://www.douban.com/search".into(),
            vec![("q", query), ("cat", "1001")],
        ),
        Platform::Qiita => (
            "https://qiita.com/api/v2/items".into(),
            vec![("query", query), ("per_page", &n)],
        ),
        _ => {
            return Err(WebSearchError::InvalidConfiguration(
                "platform requires its video or CLI adapter",
            ))
        }
    };
    Ok((
        url,
        params
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect(),
    ))
}

fn wiki_language(value: &str) -> &str {
    match value {
        "zh" | "en" | "ja" | "ko" | "de" | "fr" | "es" | "ru" => value,
        _ => "en",
    }
}

fn parse(
    platform: Platform,
    query: &str,
    maximum: usize,
    language: &str,
    bytes: &[u8],
) -> Result<SourceResponse, WebSearchError> {
    let engine = platform.id();
    let sources = match platform {
        Platform::Github => decode::<Github>(bytes)?
            .items
            .0
            .into_iter()
            .map(|r| {
                source(
                    engine,
                    r.html_url,
                    r.full_name.or(r.name).unwrap_or_default(),
                    format!(
                        "{}{}",
                        r.description.unwrap_or_default(),
                        r.stargazers_count
                            .map(|n| format!(" ⭐{n}"))
                            .unwrap_or_default()
                    ),
                    None,
                )
            })
            .collect(),
        Platform::V2ex => decode::<Items<Topic>>(bytes)?
            .0
            .into_iter()
            .filter(|r| {
                r.title.to_lowercase().contains(&query.to_lowercase())
                    || r.content.to_lowercase().contains(&query.to_lowercase())
            })
            .map(|r| {
                source(
                    engine,
                    format!("https://www.v2ex.com/t/{}", r.id),
                    r.title,
                    r.content,
                    None,
                )
            })
            .collect(),
        Platform::Bilibili => {
            let d = decode::<Bilibili>(bytes)?;
            if d.code != 0 {
                return Err(WebSearchError::InvalidResponse);
            }
            d.data
                .result
                .0
                .into_iter()
                .flat_map(|s| s.data.0)
                .filter(|r| !r.arcurl.is_empty())
                .map(|r| source(engine, r.arcurl, plain(&r.title), r.desc, None))
                .collect()
        }
        Platform::Reddit => decode::<Reddit>(bytes)?
            .data
            .children
            .0
            .into_iter()
            .map(|r| source(engine, r.data.url, r.data.title, r.data.selftext, None))
            .collect(),
        Platform::Hackernews => decode::<Hackernews>(bytes)?
            .hits
            .0
            .into_iter()
            .map(|r| {
                source(
                    engine,
                    r.url.unwrap_or_else(|| {
                        format!("https://news.ycombinator.com/item?id={}", r.object_id)
                    }),
                    r.title.or(r.story_title).unwrap_or_default(),
                    format!(
                        "HN discussion · {} points · {} comments",
                        r.points.unwrap_or(0),
                        r.num_comments.unwrap_or(0)
                    ),
                    None,
                )
            })
            .collect(),
        Platform::Stackoverflow => {
            let d = decode::<Stackoverflow>(bytes)?;
            if d.error_message.is_some() {
                return Err(WebSearchError::InvalidResponse);
            }
            d.items
                .0
                .into_iter()
                .map(|r| {
                    source(
                        engine,
                        r.link,
                        plain(&r.title),
                        format!(
                            "{} · score {} · {} answers",
                            if r.is_answered {
                                "✓ answered"
                            } else {
                                "unanswered"
                            },
                            r.score,
                            r.answer_count
                        ),
                        None,
                    )
                })
                .collect()
        }
        Platform::Wikipedia => decode::<Wikipedia>(bytes)?
            .query
            .search
            .0
            .into_iter()
            .filter_map(|r| {
                let mut url = Url::parse(&format!(
                    "https://{}.wikipedia.org",
                    wiki_language(language)
                ))
                .ok()?;
                url.path_segments_mut()
                    .ok()?
                    .push("wiki")
                    .push(&r.title.replace(' ', "_"));
                Some(source(
                    engine,
                    url.to_string(),
                    r.title,
                    plain(&r.snippet),
                    None,
                ))
            })
            .collect(),
        Platform::Npm => decode::<Npm>(bytes)?
            .objects
            .0
            .into_iter()
            .map(|r| {
                let p = r.package;
                source(
                    engine,
                    p.links
                        .npm
                        .unwrap_or_else(|| format!("https://www.npmjs.com/package/{}", p.name)),
                    p.name,
                    format!("v{} — {}", p.version, p.description.unwrap_or_default()),
                    None,
                )
            })
            .collect(),
        Platform::Juejin => {
            let d = decode::<Juejin>(bytes)?;
            if d.err_no != 0 {
                return Err(WebSearchError::InvalidResponse);
            }
            d.data
                .0
                .into_iter()
                .filter_map(|r| r.result_model.article_info)
                .map(|r| {
                    source(
                        engine,
                        format!("https://juejin.cn/post/{}", r.article_id),
                        r.title,
                        r.brief_content,
                        None,
                    )
                })
                .collect()
        }
        Platform::Qiita => decode::<Items<Qiita>>(bytes)?
            .0
            .into_iter()
            .map(|r| source(engine, r.url, r.title, plain(&r.body), None))
            .collect(),
        Platform::Douban => douban(&String::from_utf8_lossy(bytes)),
        _ => return Err(WebSearchError::InvalidResponse),
    };
    Ok(SourceResponse::new(sources, maximum))
}

fn douban(html: &str) -> Vec<crate::WebSearchResult> {
    let doc = dom_query::Document::from(html);
    doc.select("div.result")
        .iter()
        .filter_map(|card| {
            let a = card.select("h3 a");
            let address = a.attr("href")?;
            let url = Url::parse(&address).ok()?;
            let target = url
                .query_pairs()
                .find(|(k, _)| k == "url")
                .map(|(_, v)| v.into_owned())
                .unwrap_or_else(|| url.to_string());
            Some(source(
                "douban",
                target,
                a.text().to_string(),
                card.select(".info").text().to_string(),
                None,
            ))
        })
        .collect()
}

#[derive(Deserialize)]
struct Github {
    #[serde(default)]
    items: Items<Repository>,
}
#[derive(Deserialize)]
struct Repository {
    html_url: String,
    full_name: Option<String>,
    name: Option<String>,
    description: Option<String>,
    stargazers_count: Option<u64>,
}
#[derive(Deserialize)]
struct Topic {
    id: u64,
    title: String,
    #[serde(default)]
    content: String,
}
#[derive(Deserialize)]
struct Bilibili {
    code: i64,
    data: BilibiliData,
}
#[derive(Deserialize)]
struct BilibiliData {
    #[serde(default)]
    result: Items<Section>,
}
#[derive(Deserialize)]
struct Section {
    #[serde(default)]
    data: Items<Video>,
}
#[derive(Deserialize)]
struct Video {
    #[serde(default)]
    arcurl: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    desc: String,
}
#[derive(Deserialize)]
struct Reddit {
    data: RedditData,
}
#[derive(Deserialize)]
struct RedditData {
    #[serde(default)]
    children: Items<Post>,
}
#[derive(Deserialize)]
struct Post {
    data: PostData,
}
#[derive(Deserialize)]
struct PostData {
    url: String,
    title: String,
    #[serde(default)]
    selftext: String,
}
#[derive(Deserialize)]
struct Hackernews {
    #[serde(default)]
    hits: Items<Story>,
}
#[derive(Deserialize)]
struct Story {
    #[serde(rename = "objectID")]
    object_id: String,
    url: Option<String>,
    title: Option<String>,
    story_title: Option<String>,
    points: Option<u64>,
    num_comments: Option<u64>,
}
#[derive(Deserialize)]
struct Stackoverflow {
    error_message: Option<String>,
    #[serde(default)]
    items: Items<Question>,
}
#[derive(Deserialize)]
struct Question {
    link: String,
    title: String,
    #[serde(default)]
    is_answered: bool,
    #[serde(default)]
    score: i64,
    #[serde(default)]
    answer_count: u64,
}
#[derive(Deserialize)]
struct Wikipedia {
    query: WikiQuery,
}
#[derive(Deserialize)]
struct WikiQuery {
    #[serde(default)]
    search: Items<WikiResult>,
}
#[derive(Deserialize)]
struct WikiResult {
    title: String,
    #[serde(default)]
    snippet: String,
}
#[derive(Deserialize)]
struct Npm {
    #[serde(default)]
    objects: Items<PackageObject>,
}
#[derive(Deserialize)]
struct PackageObject {
    package: Package,
}
#[derive(Deserialize)]
struct Package {
    name: String,
    #[serde(default)]
    version: String,
    description: Option<String>,
    #[serde(default)]
    links: PackageLinks,
}
#[derive(Default, Deserialize)]
struct PackageLinks {
    npm: Option<String>,
}
#[derive(Deserialize)]
struct Juejin {
    err_no: i64,
    #[serde(default)]
    data: Items<ArticleResult>,
}
#[derive(Deserialize)]
struct ArticleResult {
    result_model: ArticleModel,
}
#[derive(Deserialize)]
struct ArticleModel {
    article_info: Option<Article>,
}
#[derive(Deserialize)]
struct Article {
    article_id: String,
    title: String,
    #[serde(default)]
    brief_content: String,
}
#[derive(Deserialize)]
struct Qiita {
    url: String,
    title: String,
    #[serde(default)]
    body: String,
}

#[cfg(test)]
mod tests;
