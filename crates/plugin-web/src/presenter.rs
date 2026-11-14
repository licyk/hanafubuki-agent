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

//! Human-facing summaries of structured web results.
use hanafubuki_agent_contracts::{
    structured_output, AgentToolDisplay, AgentToolPresenter, AgentWebResult,
};
use hanafubuki_agent_tool_contracts::ToolOutput;
use hanafubuki_agent_web_search::{
    ResearchReport, SourceResponse, WeatherReport, WebFetchResponse, WebSearchResponse,
};
use serde::Deserialize;
use serde_json::Value;
pub(crate) struct WebPresenter;
impl AgentToolPresenter for WebPresenter {
    fn tools(&self) -> &[&'static str] {
        &[
            "web_search",
            "advanced_search",
            "multi_search",
            "fetch_web_page",
            "platform_search",
            "video_search",
            "domain_search",
            "weather",
            "research",
            "cache_search",
            "free_search_test",
        ]
    }
    fn subject(&self, tool: &str, arguments: &Value) -> Option<String> {
        let key = match tool {
            "fetch_web_page" => "url",
            "weather" => "location",
            "research" => "question",
            _ if self.tools().contains(&tool) => "query",
            _ => return None,
        };
        let text = arguments.get(key)?.as_str()?.trim();
        (!text.is_empty()).then(|| text.into())
    }
    fn display(
        &self,
        tool: &str,
        arguments: &Value,
        output: &ToolOutput,
    ) -> Option<AgentToolDisplay> {
        let value = structured_output(output)?.clone();
        match tool {
            "web_search" | "advanced_search" | "multi_search" => {
                let response: WebSearchResponse = serde_json::from_value(value).ok()?;
                Some(search(response.query, response.results))
            }
            "platform_search" | "video_search" | "domain_search" => {
                let response: SourceResponse = serde_json::from_value(value).ok()?;
                Some(search(
                    self.subject(tool, arguments).unwrap_or_default(),
                    response.sources,
                ))
            }
            "fetch_web_page" => {
                let page: WebFetchResponse = serde_json::from_value(value).ok()?;
                Some(AgentToolDisplay::WebPage {
                    url: page.final_url,
                    title: page.title,
                    text: page.content,
                    truncated: page.truncated || page.next_offset.is_some(),
                })
            }
            "research" => {
                let report: ResearchReport = serde_json::from_value(value).ok()?;
                let mut text = format!("{}\n{}\n", report.question, report.quality.note);
                for doc in report.documents {
                    text.push_str(&format!("\n{} — {}\n{}\n", doc.title, doc.url, doc.content));
                }
                for note in report.notes {
                    text.push_str(&format!("\n{note}"));
                }
                Some(AgentToolDisplay::Text { text })
            }
            "weather" => {
                let report: WeatherReport = serde_json::from_value(value).ok()?;
                let mut text = format!(
                    "{} ({})\n{} · {} °C\n",
                    report.location,
                    report.timezone,
                    report.current.text,
                    number(report.current.temperature)
                );
                for day in report.daily {
                    text.push_str(&format!(
                        "{}: {} · {}–{} °C · {}%\n",
                        day.date,
                        day.text,
                        number(day.min),
                        number(day.max),
                        number(day.precipitation_probability)
                    ));
                }
                Some(AgentToolDisplay::Text { text })
            }
            "cache_search" => {
                let response: CachePages = serde_json::from_value(value).ok()?;
                Some(AgentToolDisplay::WebSearch {
                    query: self.subject(tool, arguments).unwrap_or_default(),
                    results: response
                        .pages
                        .into_iter()
                        .map(|p| AgentWebResult {
                            title: p.title,
                            url: p.url,
                            snippet: format!("{} · {}", p.fetched_at, p.excerpt),
                        })
                        .collect(),
                })
            }
            "free_search_test" => {
                let response: Checks = serde_json::from_value(value).ok()?;
                Some(AgentToolDisplay::Text {
                    text: response
                        .results
                        .into_iter()
                        .map(|r| {
                            format!(
                                "{}: {} ({} results){}",
                                r.engine,
                                r.status,
                                r.results,
                                r.error.map(|e| format!(" · {e}")).unwrap_or_default()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                })
            }
            _ => None,
        }
    }
}
fn search(
    query: String,
    results: Vec<hanafubuki_agent_web_search::WebSearchResult>,
) -> AgentToolDisplay {
    AgentToolDisplay::WebSearch {
        query,
        results: results
            .into_iter()
            .map(|r| AgentWebResult {
                title: r.title,
                url: r.url,
                snippet: r.snippet,
            })
            .collect(),
    }
}
fn number(value: Option<f64>) -> String {
    value
        .map(|n| format!("{n:.1}"))
        .unwrap_or_else(|| "—".into())
}
#[derive(Deserialize)]
struct CachePages {
    pages: Vec<CachePage>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachePage {
    title: String,
    url: String,
    excerpt: String,
    fetched_at: String,
}
#[derive(Deserialize)]
struct Checks {
    results: Vec<Check>,
}
#[derive(Deserialize)]
struct Check {
    engine: String,
    status: String,
    results: usize,
    error: Option<String>,
}
#[cfg(test)]
mod tests;
