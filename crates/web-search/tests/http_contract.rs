#![cfg(feature = "test-support")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_web_search::{
    WebSearchCategory, WebSearchConfig, WebSearchFailureCode, WebSearchProvider,
    WebSearchProviderPreference, WebSearchProxy, WebSearchRequest, WebSearchService,
    WebSearchTimeRange,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

struct TestServer {
    base_url: String,
    requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn start_server(
    handler: impl Fn(&str) -> (u16, String) + Send + Sync + 'static,
) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let request_count = requests.clone();
    let handler = Arc::new(handler);
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let handler = handler.clone();
            let request_count = request_count.clone();
            tokio::spawn(async move {
                let mut request = vec![0u8; 4096];
                let Ok(read) = stream.read(&mut request).await else {
                    return;
                };
                request_count.fetch_add(1, Ordering::SeqCst);
                let request = String::from_utf8_lossy(&request[..read]);
                let target = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let (status, body) = handler(target);
                let reason = if status == 200 { "OK" } else { "Error" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    TestServer {
        base_url: format!("http://{address}"),
        requests,
        task,
    }
}

fn request(provider: WebSearchProvider) -> WebSearchRequest {
    WebSearchRequest {
        query: "rust cancellation".to_string(),
        maximum_results: 3,
        provider,
        category: Default::default(),
        time_range: None,
        region: None,
    }
}

fn test_config(duckduckgo_search_url: String) -> WebSearchConfig {
    let mut config = WebSearchConfig::default().with_test_duckduckgo_url(duckduckgo_search_url);
    config.proxy = WebSearchProxy::Disabled;
    config.minimum_request_interval = Duration::ZERO;
    config.cache_ttl = Duration::from_secs(60);
    config
}

#[tokio::test]
async fn duckduckgo_primary_uses_bounded_post_form_with_region_and_time_range() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut chunk = [0u8; 1024];
            let read = stream.read(&mut chunk).await.unwrap();
            request.extend_from_slice(&chunk[..read]);
            let Some(header_end) = request.windows(4).position(|value| value == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }
        let request = String::from_utf8_lossy(&request);
        assert!(request.starts_with("POST /html HTTP/1.1\r\n"));
        let body = request.split_once("\r\n\r\n").unwrap().1;
        assert!(body.contains("q=rust+cancellation"));
        assert!(body.contains("b="));
        assert!(body.contains("df=m"));
        assert!(body.contains("kl=cn-zh"));
        let response_body = r#"<div class="result"><a class="result__a" href="https://example.com/post">POST result</a></div>"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
            response_body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    let service = WebSearchService::new(test_config(format!("http://{address}/html"))).unwrap();
    let mut request = request(WebSearchProvider::Duckduckgo);
    request.time_range = Some(WebSearchTimeRange::Month);
    request.region = Some("cn-zh".to_string());

    let response = service
        .search(request, CancellationToken::default())
        .await
        .unwrap();
    assert_eq!(response.results[0].title, "POST result");
    server.await.unwrap();
}

#[tokio::test]
async fn duckduckgo_html_is_bounded_normalized_deduplicated_and_cached() {
    let server = start_server(|target| {
        assert!(target.starts_with("/html"));
        (200, r#"
            <div class="result">
              <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fguide%3Futm_source%3Dsearch">Guide</a>
              <div class="result__snippet">A useful guide.</div>
              <span class="result__url">example.com</span>
            </div>
            <div class="result">
              <a class="result__a" href="https://example.com/guide">Duplicate guide</a>
              <div class="result__snippet">Duplicate.</div>
            </div>
            <div class="result">
              <a class="result__a" href="file:///etc/passwd">Invalid scheme</a>
            </div>
        "#.to_string())
    })
    .await;
    let service = WebSearchService::new(test_config(format!("{}/html", server.base_url))).unwrap();

    let first = service
        .search(
            request(WebSearchProvider::Duckduckgo),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(first.results.len(), 1);
    assert_eq!(first.results[0].title, "Guide");
    assert_eq!(first.results[0].url, "https://example.com/guide");
    assert!(!first.cache_hit);

    let second = service
        .search(
            request(WebSearchProvider::Duckduckgo),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert!(second.cache_hit);
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn auto_falls_back_from_searxng_with_a_safe_partial_failure() {
    let server = start_server(|target| {
        if target.starts_with("/searx/search?") {
            return (503, "unavailable".to_string());
        }
        assert!(target.starts_with("/html"));
        (
            200,
            r#"
            <div class="result">
              <a class="result__a" href="https://example.org/result">Fallback</a>
              <div class="result__snippet">Fallback result.</div>
            </div>
        "#
            .to_string(),
        )
    })
    .await;
    let mut config = test_config(format!("{}/html", server.base_url));
    config.searxng_base_url = Some(format!("{}/searx", server.base_url));
    let service = WebSearchService::new(config).unwrap();

    let response = service
        .search(
            request(WebSearchProvider::Auto),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(response.results[0].title, "Fallback");
    assert_eq!(
        response.providers,
        vec![WebSearchProvider::Searxng, WebSearchProvider::Duckduckgo]
    );
    assert_eq!(response.partial_failures.len(), 1);
    assert_eq!(
        response.partial_failures[0].provider,
        WebSearchProvider::Searxng
    );
    assert_eq!(
        response.partial_failures[0].code,
        WebSearchFailureCode::HttpStatus
    );
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn searxng_json_is_projected_to_the_same_bounded_result_contract() {
    let server = start_server(|target| {
        assert!(target.starts_with(
            "/searx/search?q=rust+cancellation&format=json&categories=news&pageno=1&time_range=week"
        ));
        (
            200,
            r#"{
                "results": [
                    {
                        "title": "SearXNG result",
                        "url": "https://example.net/news?utm_source=search",
                        "content": "A bounded result.",
                        "engine": "brave",
                        "engines": ["brave", "duckduckgo"],
                        "publishedDate": "2026-07-22"
                    },
                    {
                        "title": "Duplicate",
                        "url": "https://example.net/news",
                        "content": "Duplicate"
                    }
                ],
                "answers": ["A direct answer"],
                "suggestions": ["rust cancellation token"]
            }"#
            .to_string(),
        )
    })
    .await;
    let mut config = test_config(format!("{}/html", server.base_url));
    config.searxng_base_url = Some(format!("{}/searx", server.base_url));
    let service = WebSearchService::new(config).unwrap();
    let mut request = request(WebSearchProvider::Searxng);
    request.category = WebSearchCategory::News;
    request.time_range = Some(WebSearchTimeRange::Week);

    let response = service
        .search(request, CancellationToken::default())
        .await
        .unwrap();
    assert_eq!(response.providers, vec![WebSearchProvider::Searxng]);
    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].url, "https://example.net/news");
    assert_eq!(response.results[0].source, "brave");
    assert_eq!(response.answers, vec!["A direct answer"]);
    assert_eq!(response.suggestions, vec!["rust cancellation token"]);
}

#[tokio::test]
async fn a_configured_provider_cannot_be_overridden_by_model_arguments() {
    let server = start_server(|target| {
        assert!(target.starts_with("/html"));
        (
            200,
            r#"
            <div class="result">
              <a class="result__a" href="https://example.edu/policy">Configured provider</a>
            </div>
        "#
            .to_string(),
        )
    })
    .await;
    let mut config = test_config(format!("{}/html", server.base_url));
    config.provider_preference = WebSearchProviderPreference::Duckduckgo;
    config.searxng_base_url = Some(format!("{}/searx", server.base_url));
    let service = WebSearchService::new(config).unwrap();

    let response = service
        .search(
            request(WebSearchProvider::Searxng),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(response.providers, vec![WebSearchProvider::Duckduckgo]);
    assert_eq!(response.results[0].title, "Configured provider");
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn total_provider_failures_are_not_cached() {
    let server = start_server(|_| (503, "unavailable".to_string())).await;
    let service = WebSearchService::new(test_config(format!("{}/html", server.base_url))).unwrap();

    for _ in 0..2 {
        let response = service
            .search(
                request(WebSearchProvider::Duckduckgo),
                CancellationToken::default(),
            )
            .await
            .unwrap();
        assert!(response.results.is_empty());
        assert!(!response.cache_hit);
    }
    // Each failed search exercises the POST primary and GET fallback.
    assert_eq!(server.requests.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn in_flight_http_is_cancelled_without_waiting_for_the_request_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    let mut config = test_config(format!("http://{address}/html"));
    config.cache_ttl = Duration::ZERO;
    let service = WebSearchService::new(config).unwrap();
    let cancellation = CancellationToken::default();
    let trigger = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        trigger.cancel();
    });

    let started = std::time::Instant::now();
    let result = service
        .search(request(WebSearchProvider::Duckduckgo), cancellation)
        .await;
    assert!(matches!(
        result,
        Err(hanafubuki_agent_web_search::WebSearchError::Cancelled)
    ));
    assert!(started.elapsed() < Duration::from_secs(1));
    server.abort();
}
