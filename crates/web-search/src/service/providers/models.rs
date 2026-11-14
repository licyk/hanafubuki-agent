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

// Derived from dsh-free-search 6ab5168 (MIT), model built-in search engines.

use crate::service::{normalize_results, WebSearchService};
use crate::{
    WebSearchError, WebSearchProvider as E, WebSearchRequest, WebSearchResponse, WebSearchResult,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct Input<'a> {
    model: &'a str,
    input: &'a str,
    tools: [SearchTool; 1],
}
#[derive(Serialize)]
struct SearchTool {
    #[serde(rename = "type")]
    kind: &'static str,
}
#[derive(Serialize)]
struct Chat<'a> {
    model: &'a str,
    max_tokens: usize,
    messages: [Message<'a>; 1],
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ChatTool>,
}
#[derive(Serialize)]
struct Message<'a> {
    role: &'static str,
    content: &'a str,
}
#[derive(Serialize)]
struct ChatTool {
    #[serde(rename = "type")]
    kind: &'static str,
    name: &'static str,
    max_uses: usize,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Response {
    error: Option<serde::de::IgnoredAny>,
    output: Vec<Item>,
    steps: Vec<Item>,
    content: Vec<Block>,
    choices: Vec<Choice>,
    citations: Vec<String>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Item {
    #[serde(rename = "type")]
    kind: String,
    content: Vec<Block>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    text: String,
    annotations: Vec<Citation>,
    citations: Vec<Citation>,
    content: Hits,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Hits {
    List(Vec<Citation>),
    Other(serde::de::IgnoredAny),
}
impl Default for Hits {
    fn default() -> Self {
        Self::List(Vec::new())
    }
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Citation {
    #[serde(rename = "type")]
    kind: String,
    url: String,
    title: String,
    cited_text: String,
    page_age: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Choice {
    message: Answer,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Answer {
    content: String,
}

impl WebSearchService {
    pub(in crate::service) async fn search_model(
        &self,
        engine: E,
        r: &WebSearchRequest,
        key: &str,
        cancellation: &CancellationToken,
    ) -> Result<WebSearchResponse, WebSearchError> {
        let (base, suffix, default_model) = match engine {
            E::Openai => ("https://api.openai.com/v1", "/responses", "gpt-6-luna"),
            E::Gemini => (
                "https://generativelanguage.googleapis.com/v1beta",
                "/interactions",
                "gemini-3.8-flash",
            ),
            E::Claude => (
                "https://api.anthropic.com/v1",
                "/messages",
                "claude-sonnet-5-5",
            ),
            E::DeepseekOfficial => (
                "https://api.deepseek.com/anthropic/v1",
                "/messages",
                "deepseek-v4-flash",
            ),
            E::Perplexity => ("https://api.perplexity.ai", "/chat/completions", "sonar"),
            _ => {
                return Err(WebSearchError::InvalidConfiguration(
                    "not a model search engine",
                ))
            }
        };
        let mut url = self.endpoint(engine, base)?;
        let path = url.path().trim_end_matches('/');
        if !path.ends_with(suffix) {
            url.set_path(&format!("{path}{suffix}"));
        }
        let model = self
            .inner
            .config
            .search
            .engines
            .get(&engine)
            .and_then(|c| c.model.as_deref())
            .unwrap_or(default_model);
        let builder = self.inner.client.post(url);
        let builder = match engine {
            E::Openai | E::Gemini => {
                let builder = if engine == E::Openai {
                    builder.bearer_auth(key)
                } else {
                    builder.header("x-goog-api-key", key)
                };
                builder.json(&Input {
                    model,
                    input: &r.query,
                    tools: [SearchTool {
                        kind: if engine == E::Openai {
                            "web_search"
                        } else {
                            "google_search"
                        },
                    }],
                })
            }
            _ => {
                let builder = if engine == E::Perplexity {
                    builder.bearer_auth(key)
                } else {
                    builder
                        .header("x-api-key", key)
                        .header("anthropic-version", "2023-06-01")
                };
                let builder = if engine == E::DeepseekOfficial {
                    builder.bearer_auth(key)
                } else {
                    builder
                };
                builder.json(&Chat {
                    model,
                    max_tokens: if engine == E::DeepseekOfficial {
                        4096
                    } else {
                        1024
                    },
                    messages: [Message {
                        role: "user",
                        content: &r.query,
                    }],
                    tools: if engine == E::Perplexity {
                        Vec::new()
                    } else {
                        vec![ChatTool {
                            kind: "web_search_20250305",
                            name: "web_search",
                            max_uses: if engine == E::DeepseekOfficial { 1 } else { 5 },
                        }]
                    },
                })
            }
        };
        let body = self.send_bounded(builder, cancellation).await?;
        let decoded: Response =
            serde_json::from_slice(&body).map_err(|_| WebSearchError::InvalidResponse)?;
        if decoded.error.is_some() {
            return Err(WebSearchError::InvalidResponse);
        }
        let mut results = Vec::new();
        let answer = decoded
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default();
        for url in decoded.citations {
            results.push(citation(
                engine,
                Citation {
                    title: url.clone(),
                    url,
                    ..Citation::default()
                },
                &answer,
            ));
        }
        let blocks = decoded
            .output
            .into_iter()
            .chain(decoded.steps)
            .filter(|i| matches!(i.kind.as_str(), "message" | "model_output"))
            .flat_map(|i| i.content)
            .chain(decoded.content);
        for block in blocks {
            for item in block.annotations.into_iter().chain(block.citations) {
                if matches!(
                    item.kind.as_str(),
                    "url_citation" | "web_search_result_location"
                ) {
                    results.push(citation(engine, item, &block.text));
                }
            }
            if block.kind == "web_search_tool_result" {
                if let Hits::List(hits) = block.content {
                    for hit in hits {
                        if hit.kind == "web_search_result" {
                            results.push(citation(engine, hit, ""));
                        }
                    }
                }
            }
        }
        Ok(WebSearchResponse {
            query: r.query.clone(),
            providers: vec![engine],
            results: normalize_results(results, r.maximum_results),
            answers: if answer.is_empty() {
                Vec::new()
            } else {
                vec![answer]
            },
            suggestions: Vec::new(),
            partial_failures: Vec::new(),
            cache_hit: false,
            uncertainty: None,
        })
    }
}

fn citation(engine: E, item: Citation, text: &str) -> WebSearchResult {
    WebSearchResult {
        title: if item.title.is_empty() {
            item.url.clone()
        } else {
            item.title
        },
        url: item.url,
        snippet: if item.cited_text.is_empty() {
            text.into()
        } else {
            item.cited_text
        },
        source: engine.id().into(),
        engines: vec![engine.id().into()],
        published_at: item.page_age,
    }
}
