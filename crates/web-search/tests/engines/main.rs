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

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_web_search::{
    EngineConfig, WebSearchConfig, WebSearchProvider as E, WebSearchProxy, WebSearchRequest,
    WebSearchService,
};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn run(
    engine: E,
    response: &str,
    key: &str,
) -> (String, hanafubuki_agent_web_search::WebSearchResponse) {
    let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = server.local_addr().unwrap();
    let response = response.to_string();
    let task = tokio::spawn(async move {
        let (mut stream, _) = server.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut chunk = [0; 4096];
            let read = stream.read(&mut chunk).await.unwrap();
            assert_ne!(read, 0);
            request.extend_from_slice(&chunk[..read]);
            if let Some(end) = request.windows(4).position(|s| s == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (k, v) = line.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).as_bytes()).await.unwrap();
        String::from_utf8(request).unwrap()
    });
    let mut config = WebSearchConfig::default();
    config.proxy = WebSearchProxy::Disabled;
    config.minimum_request_interval = Duration::ZERO;
    config.search.allowed_engines = vec![engine];
    config.search.engines.insert(
        engine,
        EngineConfig {
            endpoint: Some(format!("http://{address}")),
            api_keys: if key.is_empty() {
                Vec::new()
            } else {
                vec![key.into()]
            },
            google_cse_id: "test-cse".into(),
            yandex_folder_id: "test-folder".into(),
            ..EngineConfig::default()
        },
    );
    let service = WebSearchService::new(config).unwrap();
    let request = WebSearchRequest {
        engines: Vec::new(),
        query: "rust async".into(),
        provider: engine,
        maximum_results: 5,
        category: Default::default(),
        time_range: None,
        region: None,
    };
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        service.search(request, CancellationToken::default()),
    )
    .await
    .unwrap()
    .unwrap();
    let sent = task.await.unwrap();
    (sent, result)
}

#[tokio::test]
async fn rest_protocols_keep_headers_bodies_and_tolerate_bad_results() {
    let cases = [
        (
            E::Exa,
            r#"{"results":[false,{"url":"https://example.com/a","title":"Example","highlights":["evidence"]}]}"#,
            "numResults",
        ),
        (
            E::Tavily,
            r#"{"results":[{"url":"https://example.com/a","title":"Example","content":"evidence"}]}"#,
            "search_depth",
        ),
        (
            E::Firecrawl,
            r#"{"data":{"web":[{"url":"https://example.com/a","title":"Example","description":"evidence"}]}}"#,
            "limit",
        ),
        (
            E::Keenable,
            r#"{"results":[{"url":"https://example.com/a","title":"Example","snippet":"evidence"}]}"#,
            "mode",
        ),
        (
            E::Parallel,
            r#"{"results":[{"url":"https://example.com/a","title":"Example","excerpts":["evidence"]}]}"#,
            "search_queries",
        ),
        (
            E::Anysearch,
            r#"{"code":0,"data":{"results":[{"url":"https://example.com/a","title":"Example","snippet":"evidence"}]}}"#,
            "max_results",
        ),
        (
            E::Outcrawl,
            r#"{"hits":[{"url":"https://example.com/a","title":"Example","snippet":"evidence"}]}"#,
            "limit",
        ),
        (
            E::Serpbase,
            r#"{"status":0,"organic":[{"link":"https://example.com/a","title":"Example","snippet":"evidence"}]}"#,
            "hl",
        ),
    ];
    for (engine, body, field) in cases {
        let (request, response) = run(engine, body, "fixture-key").await;
        let json: serde_json::Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert!(json.get(field).is_some(), "{engine:?}");
        assert_eq!(response.results.len(), 1, "{engine:?}");
        assert_eq!(response.results[0].snippet, "evidence");
        assert!(!serde_json::to_string(&response)
            .unwrap()
            .contains("fixture-key"));
    }
}

#[tokio::test]
async fn keyless_mcp_accepts_sse_and_json_but_rejects_tool_errors() {
    let text =
        "Title: Example\nURL: https://example.com/a\nPublished: 2026-11-14\nHighlights:\nevidence";
    let json = serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":text}]}}).to_string();
    let (request, response) = run(E::Exa, &format!("event: message\ndata: {json}\n\n"), "").await;
    assert!(request.contains("web_search_exa"));
    assert_eq!(response.results[0].title, "Example");
    let (_, response) = run(E::Keenable, r#"{"jsonrpc":"2.0","id":1,"result":{"isError":true,"content":[{"type":"text","text":"failure"}]}}"#, "").await;
    assert!(response.results.is_empty());
    assert_eq!(response.partial_failures.len(), 1);
}

#[tokio::test]
async fn model_engines_extract_citations_and_use_native_search_tools() {
    for (engine, body, field) in [
        (
            E::Openai,
            r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"evidence","annotations":[{"type":"url_citation","url":"https://example.com/a","title":"Example"}]}]}]}"#,
            "web_search",
        ),
        (
            E::Gemini,
            r#"{"steps":[{"type":"model_output","content":[{"type":"text","text":"evidence","annotations":[{"type":"url_citation","url":"https://example.com/a","title":"Example"}]}]}]}"#,
            "google_search",
        ),
        (
            E::Claude,
            r#"{"content":[{"type":"web_search_tool_result","content":[{"type":"web_search_result","url":"https://example.com/a","title":"Example"}]}]}"#,
            "web_search_20250305",
        ),
        (
            E::Perplexity,
            r#"{"choices":[{"message":{"content":"evidence"}}],"citations":["https://example.com/a"]}"#,
            "sonar",
        ),
    ] {
        let (request, response) = run(engine, body, "fixture-key").await;
        assert!(request.contains(field));
        assert_eq!(response.results.len(), 1, "{engine:?}");
    }
}

#[tokio::test]
async fn regional_apis_preserve_wire_casing_and_extract_results() {
    for (engine, body, field) in [
        (
            E::Kimi,
            r#"{"results":[{"url":"https://example.com/a","title":"Example","content":"evidence"}]}"#,
            "text_query",
        ),
        (
            E::DeepseekTools,
            r#"{"results":[{"url":"https://example.com/a","title":"Example","content":"evidence"}]}"#,
            "count",
        ),
        (
            E::Metaso,
            r#"{"webpages":[{"link":"https://example.com/a","title":"Example","snippet":"evidence"}]}"#,
            "includeSummary",
        ),
        (
            E::Baidu,
            r#"{"references":[{"url":"https://example.com/a","title":"Example","content":"evidence"}]}"#,
            "search_source",
        ),
        (
            E::Doubao,
            r#"{"Result":{"WebResults":[false,{"Url":"https://example.com/a","Title":"Example","Summary":"evidence"}]}}"#,
            "Query",
        ),
        (
            E::Yuanbao,
            r#"{"Response":{"Pages":[false,{"url":"https://example.com/a","title":"Example","passage":"evidence"}]}}"#,
            "Query",
        ),
    ] {
        let (request, response) = run(engine, body, "fixture-key").await;
        let json: serde_json::Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert!(json.get(field).is_some(), "{engine:?}");
        assert_eq!(
            response.results.len(),
            1,
            "{engine:?}: {:?}",
            response.partial_failures
        );
        assert_eq!(response.results[0].snippet, "evidence");
    }
}

#[tokio::test]
async fn html_and_xml_engines_decode_destination_urls_and_keep_search_policy() {
    let encoded = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        "https://example.com/a",
    );
    let (request, response) = run(E::Bing, &format!("<li class='b_algo'><h2><a href='https://www.bing.com/ck/a?u=a1{encoded}'>Rust async reference</a></h2><p>Evidence</p></li>"), "").await;
    assert!(!request.lines().next().unwrap().contains("mkt="));
    assert_eq!(response.results[0].url, "https://example.com/a");
    let xml = "<yandexsearch><response><results><grouping><group><doc><url>https://example.com/a</url><title>Example</title><passages><passage>evidence</passage></passages><modtime>20261114</modtime></doc></group></grouping></results></response></yandexsearch>";
    let body = serde_json::json!({"rawData":base64::Engine::encode(&base64::engine::general_purpose::STANDARD,xml)}).to_string();
    let (request, response) = run(E::Yandex, &body, "fixture-key").await;
    assert!(request
        .to_ascii_lowercase()
        .contains("authorization: api-key fixture-key"));
    assert_eq!(
        response.results[0].published_at.as_deref(),
        Some("2026-11-14")
    );
}
