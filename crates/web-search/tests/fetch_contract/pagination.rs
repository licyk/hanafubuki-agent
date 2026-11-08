use super::*;

#[tokio::test]
async fn pagination_uses_exact_utf8_offsets_generation_and_cached_document() {
    let expected = "第一页 alpha 第二页 beta 第三页 gamma ".repeat(20);
    let expected_content = expected.trim().to_string();
    let body = expected.clone();
    let server = start_server(move |_| TestResponse {
        status: 200,
        content_type: "text/plain; charset=utf-8",
        headers: Vec::new(),
        body: body.clone(),
    })
    .await;
    let service = WebSearchService::new(fetch_config(&server.base_url)).unwrap();
    let url = page_url(&format!("{}/long-text", server.base_url));
    let mut offset = 0;
    let mut generation = None;
    let mut collected = String::new();
    let mut pages = 0;

    loop {
        let response = service
            .fetch_page(
                WebFetchRequest {
                    url: url.clone(),
                    maximum_content_bytes: 37,
                    from_offset: offset,
                    content_generation: generation.clone(),
                },
                CancellationToken::default(),
            )
            .await
            .unwrap();
        assert_eq!(response.content_offset, offset);
        assert_eq!(response.total_content_bytes, expected_content.len());
        assert_eq!(response.content_bytes, response.content.len());
        if let Some(previous) = generation.as_deref() {
            assert_eq!(response.content_generation, previous);
        }
        if pages == 0 {
            let invalid_offset = service
                .fetch_page(
                    WebFetchRequest {
                        url: url.clone(),
                        maximum_content_bytes: 37,
                        from_offset: 1,
                        content_generation: Some(response.content_generation.clone()),
                    },
                    CancellationToken::default(),
                )
                .await
                .unwrap_err();
            assert_eq!(invalid_offset, WebFetchError::InvalidContentOffset);
        }
        collected.push_str(&response.content);
        pages += 1;
        let Some(next) = response.next_offset else {
            assert!(!response.document_truncated);
            break;
        };
        assert!(next > offset);
        offset = next;
        generation = Some(response.content_generation);
    }

    assert!(pages > 1);
    assert_eq!(collected, expected_content);
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn continuation_fails_closed_for_invalid_offset_or_changed_document() {
    let revision = Arc::new(AtomicUsize::new(0));
    let server_revision = revision.clone();
    let server = start_server(move |_| {
        let value = server_revision.fetch_add(1, Ordering::SeqCst);
        TestResponse {
            status: 200,
            content_type: "text/plain",
            headers: Vec::new(),
            body: if value == 0 {
                "你".repeat(100)
            } else {
                "好".repeat(100)
            },
        }
    })
    .await;
    let mut config = fetch_config(&server.base_url);
    config.cache_ttl = Duration::ZERO;
    let service = WebSearchService::new(config).unwrap();
    let url = page_url(&format!("{}/changing", server.base_url));
    let first = service
        .fetch_page(
            WebFetchRequest {
                url: url.clone(),
                maximum_content_bytes: 30,
                from_offset: 0,
                content_generation: None,
            },
            CancellationToken::default(),
        )
        .await
        .unwrap();

    let invalid = service
        .fetch_page(
            WebFetchRequest {
                url: url.clone(),
                maximum_content_bytes: 30,
                from_offset: 1,
                content_generation: Some(first.content_generation.clone()),
            },
            CancellationToken::default(),
        )
        .await
        .unwrap_err();
    // With caching disabled the changed generation is detected before the
    // invalid UTF-8 offset can be applied to a different document.
    assert_eq!(invalid, WebFetchError::StaleContentGeneration);

    let stale = service
        .fetch_page(
            WebFetchRequest {
                url,
                maximum_content_bytes: 30,
                from_offset: first.next_offset.unwrap(),
                content_generation: Some(first.content_generation),
            },
            CancellationToken::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(stale, WebFetchError::StaleContentGeneration);
}
