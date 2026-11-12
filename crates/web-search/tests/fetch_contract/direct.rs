use super::*;

#[tokio::test]
async fn direct_html_fetch_extracts_bounded_untrusted_markdown() {
    let server = start_server(|_| TestResponse {
        status: 200,
        content_type: "text/html; charset=utf-8",
        headers: Vec::new(),
        body: r#"<html><head><title>Reference</title><script>secret()</script></head>
            <body><nav>ignore navigation</nav><main><h1>Heading</h1>
            <p>Useful <a href="/guide#part">guide</a> content.</p></main></body></html>"#
            .to_string(),
    })
    .await;
    let service = WebSearchService::new(fetch_config(&server.base_url)).unwrap();
    let response = service
        .fetch_page(
            WebFetchRequest {
                url: page_url(&format!("{}/page#ignored", server.base_url)),
                maximum_content_bytes: 50_000,
                from_offset: 0,
                content_generation: None,
            },
            CancellationToken::default(),
        )
        .await
        .unwrap();

    assert_eq!(response.title.as_deref(), Some("Reference"));
    assert_eq!(response.backend, WebFetchBackend::DirectHtml);
    assert!(response.content.contains("# Heading"));
    assert!(response
        .content
        .contains(&format!("[guide]({}/guide)", server.base_url)));
    assert!(!response.content.contains("secret"));
    assert!(!response.content.contains("navigation"));
    assert!(response.untrusted_external_content);
    assert!(response.content_bytes <= 50_000);
}

#[tokio::test]
async fn direct_fetch_uses_the_configured_global_proxy() {
    let target = start_server(|_| TestResponse {
        status: 200,
        content_type: "text/plain",
        headers: Vec::new(),
        body: "target must not be reached directly".to_string(),
    })
    .await;
    let expected_target = format!("{}/through-proxy", target.base_url);
    let proxy_target = expected_target.clone();
    let proxy = start_server(move |request_target| {
        assert_eq!(request_target, proxy_target);
        TestResponse {
            status: 200,
            content_type: "text/plain",
            headers: Vec::new(),
            body: "content returned by global proxy".to_string(),
        }
    })
    .await;
    let mut config = fetch_config(&target.base_url);
    config.proxy = WebSearchProxy::Explicit(proxy.base_url.clone());
    let service = WebSearchService::new(config).unwrap();

    let response = service
        .fetch_page(
            WebFetchRequest {
                url: page_url(&expected_target),
                maximum_content_bytes: 1024,
                from_offset: 0,
                content_generation: None,
            },
            CancellationToken::default(),
        )
        .await
        .unwrap();

    assert_eq!(response.content, "content returned by global proxy");
    assert_eq!(proxy.requests.load(Ordering::SeqCst), 1);
    assert_eq!(target.requests.load(Ordering::SeqCst), 0);
}

/// A redirect is followed through the proxy too: each hop is a request the
/// proxy receives, naming its whole URL, and the target is never reached.
#[tokio::test]
async fn direct_fetch_follows_redirects_through_the_proxy() {
    let target = start_server(|_| TestResponse {
        status: 200,
        content_type: "text/plain",
        headers: Vec::new(),
        body: "target must not be reached directly".to_string(),
    })
    .await;
    let first = format!("{}/first", target.base_url);
    let second = format!("{}/second", target.base_url);
    let (from, to) = (first.clone(), second.clone());
    let proxy = start_server(move |request_target| {
        if request_target == from {
            TestResponse {
                status: 302,
                content_type: "text/plain",
                headers: vec![("Location".to_string(), to.clone())],
                body: String::new(),
            }
        } else {
            assert_eq!(request_target, to);
            TestResponse {
                status: 200,
                content_type: "text/plain",
                headers: Vec::new(),
                body: "redirected through the proxy".to_string(),
            }
        }
    })
    .await;
    let mut config = fetch_config(&target.base_url);
    config.proxy = WebSearchProxy::Explicit(proxy.base_url.clone());
    let service = WebSearchService::new(config).unwrap();

    let response = service
        .fetch_page(
            WebFetchRequest {
                url: page_url(&first),
                maximum_content_bytes: 1024,
                from_offset: 0,
                content_generation: None,
            },
            CancellationToken::default(),
        )
        .await
        .unwrap();

    assert_eq!(response.content, "redirected through the proxy");
    assert_eq!(proxy.requests.load(Ordering::SeqCst), 2);
    assert_eq!(target.requests.load(Ordering::SeqCst), 0);
}

/// A name this machine cannot resolve fails the fetch by default, even
/// through a proxy, and the proxy is never asked; with
/// `resolve_through_proxy` it is left to the proxy, which fetches it. A
/// private address is refused either way.
#[tokio::test]
async fn an_unresolvable_name_is_left_to_the_proxy_only_when_opted_in() {
    const PAGE: &str = "http://page.invalid/news";
    let proxy = start_server(|request_target| {
        assert_eq!(request_target, PAGE);
        TestResponse {
            status: 200,
            content_type: "text/plain",
            headers: Vec::new(),
            body: "resolved by the proxy".to_string(),
        }
    })
    .await;
    let fetch = |resolve_through_proxy: bool, url: &str| {
        let mut config = WebSearchConfig::default();
        config.proxy = WebSearchProxy::Explicit(proxy.base_url.clone());
        config.resolve_through_proxy = resolve_through_proxy;
        let service = WebSearchService::new(config).unwrap();
        let url = page_url(url);
        async move {
            service
                .fetch_page(
                    WebFetchRequest {
                        url,
                        maximum_content_bytes: 1024,
                        from_offset: 0,
                        content_generation: None,
                    },
                    CancellationToken::default(),
                )
                .await
        }
    };

    assert!(matches!(
        fetch(false, PAGE).await,
        Err(WebFetchError::DnsResolutionFailed)
    ));
    assert_eq!(proxy.requests.load(Ordering::SeqCst), 0);

    let response = fetch(true, PAGE).await.unwrap();
    assert_eq!(response.content, "resolved by the proxy");
    assert_eq!(proxy.requests.load(Ordering::SeqCst), 1);

    assert!(matches!(
        fetch(true, "http://10.0.0.1/").await,
        Err(WebFetchError::BlockedTarget)
    ));
    assert_eq!(proxy.requests.load(Ordering::SeqCst), 1);
}
