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

use super::*;
use hanafubuki_agent_web_search::{EngineConfig, FetchPreference, WebSearchProvider as E};
#[tokio::test]
async fn extraction_fallback_obeys_preference_and_keeps_source_target() {
    for preference in [FetchPreference::Exa, FetchPreference::Firecrawl] {
        let target = start_server(|_| TestResponse {
            status: 403,
            content_type: "text/plain",
            headers: Vec::new(),
            body: "blocked".into(),
        })
        .await;
        let page = format!("{}/article", target.base_url);
        let expected = page.clone();
        let reader=start_server(move|path| {
            let text="Independent source paragraph with enough evidence to be a full body. ".repeat(10);
            let body=if path=="/exa" {serde_json::json!({"results":[{"id":expected,"text":text}],"statuses":[{"id":expected,"status":"success"}]}).to_string()} else {serde_json::json!({"data":{"markdown":text}}).to_string()};
            TestResponse {status:200,content_type:"application/json",headers:Vec::new(),body}
        }).await;
        let mut config = fetch_config(&target.base_url);
        config.search.fetch.prefer = preference;
        config.search.fetch.exa_endpoint = format!("{}/exa", reader.base_url);
        config.search.fetch.firecrawl_endpoint = format!("{}/firecrawl", reader.base_url);
        for engine in [E::Exa, E::Firecrawl] {
            config.search.engines.insert(
                engine,
                EngineConfig {
                    api_keys: vec!["fixture".into()],
                    ..Default::default()
                },
            );
        }
        let response = WebSearchService::new(config)
            .unwrap()
            .fetch_page(
                WebFetchRequest {
                    url: page_url(&page),
                    maximum_content_bytes: 100_000,
                    from_offset: 0,
                    content_generation: None,
                },
                CancellationToken::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.backend,
            if preference == FetchPreference::Exa {
                WebFetchBackend::Exa
            } else {
                WebFetchBackend::Firecrawl
            }
        );
        assert_eq!(response.final_url, page);
        assert_eq!(reader.requests.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn crawl4ai_follows_failed_keyed_tiers_and_extracts_html() {
    let target = start_server(|_| TestResponse {
        status: 403,
        content_type: "text/plain",
        headers: Vec::new(),
        body: "blocked".into(),
    })
    .await;
    let reader=start_server(|path|TestResponse {status:if path=="/crawl" {200} else {503},content_type:"application/json",headers:Vec::new(),body:serde_json::json!({"html":format!("<main><p>{}</p></main>","Final reader evidence. ".repeat(40))}).to_string()}).await;
    let mut config = fetch_config(&target.base_url);
    config.search.fetch.exa_endpoint = format!("{}/exa", reader.base_url);
    config.search.fetch.firecrawl_endpoint = format!("{}/firecrawl", reader.base_url);
    config.search.fetch.crawl4ai_endpoint = format!("{}/crawl", reader.base_url);
    config.search.fetch.crawl4ai_api_key = "fixture".into();
    for engine in [E::Exa, E::Firecrawl] {
        config.search.engines.insert(
            engine,
            EngineConfig {
                api_keys: vec!["fixture".into()],
                ..Default::default()
            },
        );
    }
    let response = WebSearchService::new(config)
        .unwrap()
        .fetch_page(
            WebFetchRequest {
                url: page_url(&target.base_url),
                maximum_content_bytes: 100_000,
                from_offset: 0,
                content_generation: None,
            },
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(response.backend, WebFetchBackend::Crawl4ai);
    assert!(response.content.contains("Final reader"));
    assert_eq!(reader.requests.load(Ordering::SeqCst), 3);
}
