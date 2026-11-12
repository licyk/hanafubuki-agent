//! How an interface shows the web tools' calls: the query or the address,
//! then the results found or the page read.

use hanafubuki_agent_contracts::{
    structured_output, AgentToolDisplay, AgentToolPresenter, AgentWebResult,
};
use hanafubuki_agent_tool_contracts::ToolOutput;
use hanafubuki_agent_web_search::{WebFetchResponse, WebSearchResponse};
use serde_json::Value;

pub(crate) struct WebPresenter;

const WEB_SEARCH: &str = "web_search";
const FETCH_WEB_PAGE: &str = "fetch_web_page";

impl AgentToolPresenter for WebPresenter {
    fn tools(&self) -> &[&'static str] {
        &[WEB_SEARCH, FETCH_WEB_PAGE]
    }

    fn subject(&self, tool: &str, arguments: &Value) -> Option<String> {
        let key = match tool {
            WEB_SEARCH => "query",
            FETCH_WEB_PAGE => "url",
            _ => return None,
        };
        let text = arguments.get(key)?.as_str()?.trim();
        (!text.is_empty()).then(|| text.to_string())
    }

    fn display(
        &self,
        tool: &str,
        _arguments: &Value,
        output: &ToolOutput,
    ) -> Option<AgentToolDisplay> {
        let value = structured_output(output)?.clone();
        match tool {
            WEB_SEARCH => {
                let response: WebSearchResponse = serde_json::from_value(value).ok()?;
                Some(AgentToolDisplay::WebSearch {
                    query: response.query,
                    results: response
                        .results
                        .into_iter()
                        .map(|result| AgentWebResult {
                            title: result.title,
                            url: result.url,
                            snippet: result.snippet,
                        })
                        .collect(),
                })
            }
            FETCH_WEB_PAGE => {
                let page: WebFetchResponse = serde_json::from_value(value).ok()?;
                Some(AgentToolDisplay::WebPage {
                    url: page.final_url,
                    title: page.title,
                    text: page.content,
                    truncated: page.truncated || page.next_offset.is_some(),
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
