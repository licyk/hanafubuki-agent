use super::*;

#[test]
fn project_http_client_rejects_invalid_explicit_proxy_configuration() {
    assert!(build_mcp_http_client(McpHttpProxyPolicy::System).is_ok());
    assert!(build_mcp_http_client(McpHttpProxyPolicy::Disabled).is_ok());
    assert!(matches!(
        build_mcp_http_client(McpHttpProxyPolicy::Explicit("://invalid".into())),
        Err(McpError::InvalidConfig(_))
    ));
}

/// An explicit proxy leaves this machine's servers alone: a server on the
/// loopback is reached directly though nothing answers at the proxy.
#[tokio::test]
async fn an_explicit_proxy_reaches_loopback_servers_directly() {
    use axum::routing::get;
    use axum::Router;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/mcp", get(|| async { "direct" })),
        )
        .await
        .unwrap();
    });
    // Nothing listens on the discard port.
    let client =
        build_mcp_http_client(McpHttpProxyPolicy::Explicit("http://127.0.0.1:9".into())).unwrap();

    for host in ["127.0.0.1".to_string(), "localhost".to_string()] {
        let body = client
            .get(format!("http://{host}:{}/mcp", address.port()))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "direct", "{host}");
    }
    server.abort();
}

#[tokio::test]
async fn oauth_http_client_follows_only_bounded_same_origin_redirects_and_rejects_cycles() {
    use axum::response::Redirect;
    use axum::routing::get;
    use axum::Router;

    let leaked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let cross_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cross_address = cross_listener.local_addr().unwrap();
    let cross_app = Router::new().route(
        "/end",
        get({
            let leaked = leaked.clone();
            move |headers: axum::http::HeaderMap| {
                let leaked = leaked.clone();
                async move {
                    if headers.contains_key(axum::http::header::AUTHORIZATION) {
                        leaked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    "unexpected"
                }
            }
        }),
    );
    let cross_server =
        tokio::spawn(async move { axum::serve(cross_listener, cross_app).await.unwrap() });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let cross_location = format!("http://{cross_address}/end");
    let app = Router::new()
        .route("/start", get(|| async { Redirect::temporary("/end") }))
        .route("/end", get(|| async { "done" }))
        .route(
            "/cycle-a",
            get(|| async { Redirect::temporary("/cycle-b") }),
        )
        .route(
            "/cycle-b",
            get(|| async { Redirect::temporary("/cycle-a") }),
        )
        .route(
            "/cross-origin",
            get(move || {
                let cross_location = cross_location.clone();
                async move {
                    axum::response::Response::builder()
                        .status(axum::http::StatusCode::TEMPORARY_REDIRECT)
                        .header(axum::http::header::LOCATION, cross_location)
                        .body(axum::body::Body::empty())
                        .unwrap()
                }
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = crate::client_adapter::McpOAuthHttpClient::new(
        build_mcp_http_client(McpHttpProxyPolicy::Disabled).unwrap(),
    );

    let response = client
        .execute_request(
            reqwest::Request::new(
                reqwest::Method::GET,
                format!("http://{address}/start").parse().unwrap(),
            ),
            true,
        )
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let error = client
        .execute_request(
            reqwest::Request::new(
                reqwest::Method::GET,
                format!("http://{address}/cycle-a").parse().unwrap(),
            ),
            true,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("cycle"));

    let mut cross_origin_request = reqwest::Request::new(
        reqwest::Method::GET,
        format!("http://{address}/cross-origin").parse().unwrap(),
    );
    cross_origin_request.headers_mut().insert(
        reqwest::header::AUTHORIZATION,
        reqwest::header::HeaderValue::from_static("Bearer do-not-leak"),
    );
    let error = client
        .execute_request(cross_origin_request, true)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("cross-origin"));
    assert!(!error.to_string().contains("do-not-leak"));
    assert_eq!(
        leaked.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "credential-bearing request reached the redirected origin"
    );

    server.abort();
    cross_server.abort();
}
