use super::*;

#[tokio::test]
async fn optional_jina_reader_is_an_explicit_bounded_fallback() {
    let target = start_server(|_| TestResponse {
        status: 403,
        content_type: "text/plain",
        headers: Vec::new(),
        body: "blocked".to_string(),
    })
    .await;
    let reader = start_server(|path| {
        assert!(path.starts_with("/http://127.0.0.1:"));
        TestResponse {
            status: 200,
            content_type: "text/markdown",
            headers: Vec::new(),
            body: "Title: Reader Result\n\n# Converted\nUseful content".to_string(),
        }
    })
    .await;
    let mut config = fetch_config(&target.base_url)
        .with_test_allowed_private_origin(&reader.base_url)
        .with_test_jina_reader_url(format!("{}/", reader.base_url));
    config.jina_reader_enabled = true;
    let service = WebSearchService::new(config).unwrap();
    let response = service
        .fetch_page(
            WebFetchRequest {
                url: page_url(&format!("{}/blocked", target.base_url)),
                maximum_content_bytes: 100_000,
                from_offset: 0,
                content_generation: None,
            },
            CancellationToken::default(),
        )
        .await
        .unwrap();

    assert_eq!(response.backend, WebFetchBackend::JinaReader);
    assert_eq!(response.title.as_deref(), Some("Reader Result"));
    assert!(response.content.contains("# Converted"));
}

#[tokio::test]
async fn direct_fetch_accepts_raw_pages_larger_than_two_mib() {
    let server = start_server(|_| TestResponse {
        status: 200,
        content_type: "text/plain",
        headers: Vec::new(),
        body: "large readable page ".repeat(180_000),
    })
    .await;
    let service = WebSearchService::new(fetch_config(&server.base_url)).unwrap();

    let response = service
        .fetch_page(
            WebFetchRequest {
                url: page_url(&format!("{}/large", server.base_url)),
                maximum_content_bytes: 100_000,
                from_offset: 0,
                content_generation: None,
            },
            CancellationToken::default(),
        )
        .await
        .unwrap();

    assert_eq!(response.backend, WebFetchBackend::DirectText);
    assert!(response.fetched_bytes > 2 * 1024 * 1024);
    assert!(response.document_truncated);
    assert_eq!(response.total_content_bytes, 1024 * 1024);
    assert_eq!(response.content_bytes, 100_000);
}

#[tokio::test]
async fn oversized_direct_responses_fall_back_to_reader_from_headers_or_stream() {
    const DIRECT_LIMIT: usize = 8 * 1024 * 1024;
    let streamed_content = "x".repeat(DIRECT_LIMIT + 1);
    let chunked_body = format!(
        "{:x}\r\n{}\r\n0\r\n\r\n",
        streamed_content.len(),
        streamed_content
    );
    let target = start_server(move |path| match path {
        "/declared-too-large" => TestResponse {
            status: 200,
            content_type: "text/plain",
            headers: vec![("Content-Length".to_string(), (DIRECT_LIMIT + 1).to_string())],
            body: String::new(),
        },
        "/streamed-too-large" => TestResponse {
            status: 200,
            content_type: "text/plain",
            headers: vec![("Transfer-Encoding".to_string(), "chunked".to_string())],
            body: chunked_body.clone(),
        },
        _ => panic!("unexpected direct target: {path}"),
    })
    .await;
    let reader = start_server(|path| {
        assert!(path.starts_with("/http://127.0.0.1:"));
        TestResponse {
            status: 200,
            content_type: "text/markdown",
            headers: Vec::new(),
            body: "Title: Bounded Reader\n\n# Reader fallback\nLarge page converted".to_string(),
        }
    })
    .await;
    let mut config = fetch_config(&target.base_url)
        .with_test_allowed_private_origin(&reader.base_url)
        .with_test_jina_reader_url(format!("{}/", reader.base_url));
    config.jina_reader_enabled = true;
    let service = WebSearchService::new(config).unwrap();

    for path in ["declared-too-large", "streamed-too-large"] {
        let response = service
            .fetch_page(
                WebFetchRequest {
                    url: page_url(&format!("{}/{path}", target.base_url)),
                    maximum_content_bytes: 100_000,
                    from_offset: 0,
                    content_generation: None,
                },
                CancellationToken::default(),
            )
            .await
            .unwrap();
        assert_eq!(response.backend, WebFetchBackend::JinaReader);
        assert!(response.content.contains("Large page converted"));
    }

    assert_eq!(target.requests.load(Ordering::SeqCst), 2);
    assert_eq!(reader.requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn reader_keeps_its_own_timeout_after_direct_fetch_times_out() {
    let target = start_stalling_server().await;
    let reader = start_server(|_| TestResponse {
        status: 200,
        content_type: "text/markdown",
        headers: Vec::new(),
        body: "Title: Reader After Timeout\n\n# Recovered".to_string(),
    })
    .await;
    let mut config = fetch_config(&target.base_url)
        .with_test_allowed_private_origin(&reader.base_url)
        .with_test_jina_reader_url(format!("{}/", reader.base_url));
    config.jina_reader_enabled = true;
    config.page_fetch_timeout = Duration::from_millis(50);
    config.jina_reader_timeout = Duration::from_secs(1);
    config.page_fetch_total_timeout = Duration::from_secs(2);
    let service = WebSearchService::new(config).unwrap();

    let response = service
        .fetch_page(
            WebFetchRequest {
                url: page_url(&format!("{}/slow", target.base_url)),
                maximum_content_bytes: 100_000,
                from_offset: 0,
                content_generation: None,
            },
            CancellationToken::default(),
        )
        .await
        .unwrap();

    assert_eq!(response.backend, WebFetchBackend::JinaReader);
    assert_eq!(response.title.as_deref(), Some("Reader After Timeout"));
    assert_eq!(target.requests.load(Ordering::SeqCst), 1);
    assert_eq!(reader.requests.load(Ordering::SeqCst), 1);
}
