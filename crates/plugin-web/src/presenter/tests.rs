use hanafubuki_agent_contracts::ToolResult;
use serde_json::json;

use super::*;

#[test]
fn a_search_shows_its_query_and_results() {
    let arguments = json!({ "query": " rust async ", "maximumResults": null });
    let output = ToolResult::success(json!({
        "query": "rust async",
        "providers": [],
        "results": [{
            "title": "Async Rust", "url": "https://rust-lang.github.io/async-book/",
            "snippet": "The book", "source": "duckduckgo", "engines": [], "publishedAt": null
        }],
        "answers": [], "suggestions": [], "partialFailures": [], "cacheHit": false
    }))
    .output;

    assert_eq!(
        WebPresenter.subject(WEB_SEARCH, &arguments).as_deref(),
        Some("rust async")
    );
    assert_eq!(
        WebPresenter.display(WEB_SEARCH, &arguments, &output),
        Some(AgentToolDisplay::WebSearch {
            query: "rust async".to_string(),
            results: vec![AgentWebResult {
                title: "Async Rust".to_string(),
                url: "https://rust-lang.github.io/async-book/".to_string(),
                snippet: "The book".to_string(),
            }],
        })
    );
}

#[test]
fn a_fetched_page_shows_its_text_and_an_error_takes_the_fallback() {
    let arguments = json!({ "url": "https://example.com" });
    let page = |next_offset: Option<u64>| {
        ToolResult::success(json!({
            "finalUrl": "https://example.com/", "title": "Example", "content": "Hello",
            "contentType": "text/html", "backend": "direct_text", "fetchedBytes": 5,
            "contentBytes": 5, "contentOffset": 0, "nextOffset": next_offset,
            "totalContentBytes": 5, "contentGeneration": "ab", "documentTruncated": false,
            "truncated": false, "redirectCount": 0, "untrustedExternalContent": true
        }))
        .output
    };

    assert_eq!(
        WebPresenter.subject(FETCH_WEB_PAGE, &arguments).as_deref(),
        Some("https://example.com")
    );
    assert_eq!(
        WebPresenter.display(FETCH_WEB_PAGE, &arguments, &page(None)),
        Some(AgentToolDisplay::WebPage {
            url: "https://example.com/".to_string(),
            title: Some("Example".to_string()),
            text: "Hello".to_string(),
            truncated: false,
        })
    );
    let Some(AgentToolDisplay::WebPage { truncated, .. }) =
        WebPresenter.display(FETCH_WEB_PAGE, &arguments, &page(Some(5)))
    else {
        panic!("a page with more to read is a page");
    };
    assert!(truncated);
    let failed = ToolResult::model_visible_error(json!({ "error": "blocked" })).output;
    assert_eq!(
        WebPresenter.display(FETCH_WEB_PAGE, &arguments, &failed),
        None
    );
}
