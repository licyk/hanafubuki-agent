use super::*;

#[tokio::test]
async fn every_redirect_target_is_revalidated() {
    let private_destination = start_server(|_| TestResponse {
        status: 200,
        content_type: "text/plain",
        headers: Vec::new(),
        body: "must not be reached".to_string(),
    })
    .await;
    let destination = private_destination.base_url.clone();
    let public_test_origin = start_server(move |_| TestResponse {
        status: 302,
        content_type: "text/plain",
        headers: vec![("Location".to_string(), format!("{destination}/secret"))],
        body: String::new(),
    })
    .await;
    let service = WebSearchService::new(fetch_config(&public_test_origin.base_url)).unwrap();
    let error = service
        .fetch_page(
            WebFetchRequest {
                url: page_url(&format!("{}/start", public_test_origin.base_url)),
                maximum_content_bytes: 1024,
                from_offset: 0,
                content_generation: None,
            },
            CancellationToken::default(),
        )
        .await
        .unwrap_err();

    assert_eq!(error, WebFetchError::BlockedTarget);
}

#[tokio::test]
async fn loopback_non_web_ports_and_hostless_targets_fail_closed() {
    let service = WebSearchService::new(WebSearchConfig::default()).unwrap();
    for url in [
        "http://127.0.0.1/private",
        "file:///etc/passwd",
        "http://example.com:8080/",
    ] {
        assert!(service
            .fetch_page(
                WebFetchRequest {
                    url: page_url(url),
                    maximum_content_bytes: 1024,
                    from_offset: 0,
                    content_generation: None,
                },
                CancellationToken::default(),
            )
            .await
            .is_err());
    }
}
