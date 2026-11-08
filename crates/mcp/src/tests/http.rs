use super::*;

async fn reject_discover_as_legacy(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
    let message = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
    if message
        .as_ref()
        .and_then(|message| message.get("method"))
        .and_then(serde_json::Value::as_str)
        == Some("server/discover")
    {
        return axum::Json(json!({
            "jsonrpc": "2.0",
            "id": message.as_ref().and_then(|message| message.get("id")).cloned(),
            "error": {"code": -32601, "message": "Method not found"}
        }))
        .into_response();
    }
    next.run(axum::http::Request::from_parts(
        parts,
        axum::body::Body::from(bytes),
    ))
    .await
}

#[tokio::test]
async fn streamable_http_initializes_discovers_calls_and_shuts_down() {
    let cancellation = TokioCancellationToken::new();
    let service: StreamableHttpService<HttpFixtureServer, LocalSessionManager> =
        StreamableHttpService::new(
            || Ok(HttpFixtureServer::new()),
            Default::default(),
            StreamableHttpServerConfig::default()
                .with_sse_keep_alive(None)
                .with_cancellation_token(cancellation.child_token()),
        );
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_address = proxy_listener.local_addr().unwrap();
    let accepted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server = tokio::spawn({
        let cancellation = cancellation.clone();
        async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move { cancellation.cancelled_owned().await })
                .await;
        }
    });
    let proxy = tokio::spawn({
        let cancellation = cancellation.clone();
        let accepted = accepted.clone();
        async move {
            loop {
                let incoming = tokio::select! {
                    incoming = proxy_listener.accept() => incoming,
                    _ = cancellation.cancelled() => break,
                };
                let Ok((mut downstream, _)) = incoming else {
                    break;
                };
                let attempt = accepted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if attempt == 0 {
                    continue;
                }
                tokio::spawn(async move {
                    let mut upstream = tokio::net::TcpStream::connect(address).await.unwrap();
                    let _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await;
                });
            }
        }
    });

    let mut config = stdio_config();
    config.protocol_mode = McpProtocolMode::V20260728;
    config.transport = McpTransportConfig::StreamableHttp(McpStreamableHttpConfig {
        url: format!("http://{proxy_address}/mcp"),
        public_headers: BTreeMap::from([("x-fixture".into(), "public".into())]),
        credential_headers: BTreeMap::new(),
        oauth: McpOAuthPolicy::Disabled,
    });
    let McpTransportConfig::StreamableHttp(http_config) = &config.transport else {
        unreachable!()
    };
    let transport_config = crate::streamable_http::transport_config(
        http_config,
        McpResolvedSecretHeaders::new(BTreeMap::new()).unwrap(),
    )
    .unwrap();
    assert!(transport_config.reinit_on_expired_session);
    assert_eq!(
        transport_config.max_sse_event_size,
        crate::client_adapter::MAXIMUM_MCP_MESSAGE_BYTES
    );
    let connection = McpStreamableHttpConnection::connect(
        &config,
        McpResolvedSecretHeaders::new(BTreeMap::new()).unwrap(),
        1,
        McpInteractionHub::default(),
        None,
        CancellationToken::default(),
    )
    .await
    .unwrap();
    assert!(accepted.load(std::sync::atomic::Ordering::SeqCst) >= 2);
    let tools = connection.discover_tools(&config).await.unwrap().tools;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].remote_name, "echo");
    let result = connection
        .call_tool(
            &tools[0],
            serde_json::Map::from_iter([("value".into(), json!("over-http"))]),
            Duration::from_secs(5),
            crate::interaction::ElicitationPauseState::new(),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    let output = convert_tool_output(result).unwrap();
    assert!(matches!(
        &output.content[0],
        hanafubuki_agent_tool_contracts::ToolOutputContent::Text { text } if text == "over-http"
    ));
    connection.shutdown(Duration::from_secs(5)).await.unwrap();
    cancellation.cancel();
    server.await.unwrap();
    proxy.await.unwrap();
}

#[tokio::test]
async fn expired_http_session_reinitializes_and_retries_the_same_frozen_binding() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let cancellation = TokioCancellationToken::new();
    let sessions = Arc::new(LocalSessionManager::default());
    let initializations = Arc::new(AtomicUsize::new(0));
    let service: StreamableHttpService<HttpFixtureServer, LocalSessionManager> =
        StreamableHttpService::new(
            {
                let initializations = initializations.clone();
                move || {
                    initializations.fetch_add(1, Ordering::SeqCst);
                    Ok(HttpFixtureServer::new())
                }
            },
            sessions.clone(),
            StreamableHttpServerConfig::default()
                .with_sse_keep_alive(None)
                .with_cancellation_token(cancellation.child_token()),
        );
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn({
        let cancellation = cancellation.clone();
        async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move { cancellation.cancelled_owned().await })
                .await;
        }
    });

    let mut config = stdio_config();
    config.protocol_mode = McpProtocolMode::Legacy;
    config.transport = McpTransportConfig::StreamableHttp(McpStreamableHttpConfig {
        url: format!("http://{address}/mcp"),
        public_headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        oauth: McpOAuthPolicy::Disabled,
    });
    let connection = McpStreamableHttpConnection::connect(
        &config,
        McpResolvedSecretHeaders::new(BTreeMap::new()).unwrap(),
        7,
        McpInteractionHub::default(),
        None,
        CancellationToken::default(),
    )
    .await
    .unwrap();
    let generation = connection.generation();
    let tools = connection.discover_tools(&config).await.unwrap().tools;
    let frozen = tools[0].clone();
    let warmup = connection
        .call_tool(
            &frozen,
            serde_json::Map::from_iter([("value".into(), json!("warmup"))]),
            Duration::from_secs(5),
            crate::interaction::ElicitationPauseState::new(),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert!(matches!(
        &convert_tool_output(warmup).unwrap().content[0],
        hanafubuki_agent_tool_contracts::ToolOutputContent::Text { text } if text == "warmup"
    ));

    // Remove the exact server-side transport session. The client's next POST
    // receives a real 404 and rmcp must reinitialize before retrying the same
    // logical call through the already-frozen binding.
    sessions.sessions.write().await.clear();
    let recovered = connection
        .call_tool(
            &frozen,
            serde_json::Map::from_iter([("value".into(), json!("recovered"))]),
            Duration::from_secs(5),
            crate::interaction::ElicitationPauseState::new(),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert!(matches!(
        &convert_tool_output(recovered).unwrap().content[0],
        hanafubuki_agent_tool_contracts::ToolOutputContent::Text { text } if text == "recovered"
    ));
    assert_eq!(connection.generation(), generation);
    assert_eq!(frozen.connection_generation, generation);
    assert_eq!(initializations.load(Ordering::SeqCst), 2);

    connection.shutdown(Duration::from_secs(5)).await.unwrap();
    cancellation.cancel();
    server.await.unwrap();
}

#[tokio::test]
async fn modern_http_discovery_uses_stateless_request_metadata() {
    let cancellation = TokioCancellationToken::new();
    let sessions = Arc::new(LocalSessionManager::default());
    let service: StreamableHttpService<HttpFixtureServer, LocalSessionManager> =
        StreamableHttpService::new(
            || Ok(HttpFixtureServer::new()),
            sessions.clone(),
            StreamableHttpServerConfig::default()
                .with_legacy_session_mode(false)
                .with_json_response(true)
                .with_stateless_protocol_metadata_required(true)
                .with_sse_keep_alive(None)
                .with_cancellation_token(cancellation.child_token()),
        );
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn({
        let cancellation = cancellation.clone();
        async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move { cancellation.cancelled_owned().await })
                .await;
        }
    });
    let mut config = stdio_config();
    config.protocol_mode = McpProtocolMode::V20260728;
    config.transport = McpTransportConfig::StreamableHttp(McpStreamableHttpConfig {
        url: format!("http://{address}/mcp"),
        public_headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        oauth: McpOAuthPolicy::Disabled,
    });

    let connection = McpStreamableHttpConnection::connect(
        &config,
        McpResolvedSecretHeaders::new(BTreeMap::new()).unwrap(),
        1,
        McpInteractionHub::default(),
        None,
        CancellationToken::default(),
    )
    .await
    .unwrap();
    assert_eq!(connection.capabilities().protocol_version, "2026-07-28");
    assert_eq!(
        connection
            .discover_tools(&config)
            .await
            .unwrap()
            .tools
            .len(),
        1
    );
    assert!(sessions.sessions.read().await.is_empty());

    connection.shutdown(Duration::from_secs(5)).await.unwrap();
    cancellation.cancel();
    server.await.unwrap();
}

#[tokio::test]
async fn modern_http_probe_falls_back_only_after_correlated_legacy_discovery() {
    let cancellation = TokioCancellationToken::new();
    let sessions = Arc::new(LocalSessionManager::default());
    let service: StreamableHttpService<HttpFixtureServer, LocalSessionManager> =
        StreamableHttpService::new(
            || Ok(HttpFixtureServer::legacy()),
            sessions.clone(),
            StreamableHttpServerConfig::default()
                .with_sse_keep_alive(None)
                .with_cancellation_token(cancellation.child_token()),
        );
    let router = axum::Router::new()
        .nest_service("/mcp", service)
        .layer(axum::middleware::from_fn(reject_discover_as_legacy));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn({
        let cancellation = cancellation.clone();
        async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move { cancellation.cancelled_owned().await })
                .await;
        }
    });
    let mut config = stdio_config();
    config.protocol_mode = McpProtocolMode::V20260728;
    config.transport = McpTransportConfig::StreamableHttp(McpStreamableHttpConfig {
        url: format!("http://{address}/mcp"),
        public_headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        oauth: McpOAuthPolicy::Disabled,
    });

    let connection = McpStreamableHttpConnection::connect(
        &config,
        McpResolvedSecretHeaders::new(BTreeMap::new()).unwrap(),
        1,
        McpInteractionHub::default(),
        None,
        CancellationToken::default(),
    )
    .await
    .unwrap();
    assert_eq!(connection.capabilities().protocol_version, "2025-06-18");
    assert_eq!(
        connection
            .discover_tools(&config)
            .await
            .unwrap()
            .tools
            .len(),
        1
    );
    assert_eq!(sessions.sessions.read().await.len(), 1);

    connection.shutdown(Duration::from_secs(5)).await.unwrap();
    cancellation.cancel();
    server.await.unwrap();
}

#[tokio::test]
async fn oversized_http_sse_event_fails_with_a_bounded_secret_free_error() {
    let cancellation = TokioCancellationToken::new();
    let service: StreamableHttpService<OversizedHttpFixtureServer, LocalSessionManager> =
        StreamableHttpService::new(
            || Ok(OversizedHttpFixtureServer::new()),
            Default::default(),
            StreamableHttpServerConfig::default()
                .with_sse_keep_alive(None)
                .with_cancellation_token(cancellation.child_token()),
        );
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn({
        let cancellation = cancellation.clone();
        async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move { cancellation.cancelled_owned().await })
                .await;
        }
    });
    let mut config = stdio_config();
    config.protocol_mode = McpProtocolMode::Legacy;
    config.transport = McpTransportConfig::StreamableHttp(McpStreamableHttpConfig {
        url: format!("http://{address}/mcp"),
        public_headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        oauth: McpOAuthPolicy::Disabled,
    });
    let connection = McpStreamableHttpConnection::connect(
        &config,
        McpResolvedSecretHeaders::new(BTreeMap::new()).unwrap(),
        1,
        McpInteractionHub::default(),
        None,
        CancellationToken::default(),
    )
    .await
    .unwrap();
    let tools = connection.discover_tools(&config).await.unwrap().tools;
    let error = connection
        .call_tool(
            &tools[0],
            serde_json::Map::new(),
            Duration::from_secs(5),
            crate::interaction::ElicitationPauseState::new(),
            CancellationToken::default(),
        )
        .await
        .unwrap_err();
    let message = error.to_string();
    assert!(message.len() <= 2200, "{message}");
    assert!(!message.contains(&"s".repeat(128)));

    let _ = connection.shutdown(Duration::from_secs(5)).await;
    cancellation.cancel();
    server.await.unwrap();
}

async fn reject_tools_list_as_unauthorized(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
    let method = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|message| message.get("method").cloned());
    if method.as_ref().and_then(serde_json::Value::as_str) == Some("tools/list") {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            [(axum::http::header::WWW_AUTHENTICATE, "Bearer")],
        )
            .into_response();
    }
    next.run(axum::http::Request::from_parts(
        parts,
        axum::body::Body::from(bytes),
    ))
    .await
}

/// Requesting pages through the raw-result path keeps an HTTP authorization
/// failure a failure of the whole listing.
#[tokio::test]
async fn an_unauthorized_tool_listing_still_fails_discovery() {
    let cancellation = TokioCancellationToken::new();
    let service: StreamableHttpService<HttpFixtureServer, LocalSessionManager> =
        StreamableHttpService::new(
            || Ok(HttpFixtureServer::legacy()),
            Default::default(),
            StreamableHttpServerConfig::default()
                .with_sse_keep_alive(None)
                .with_cancellation_token(cancellation.child_token()),
        );
    let router = axum::Router::new()
        .nest_service("/mcp", service)
        .layer(axum::middleware::from_fn(reject_tools_list_as_unauthorized));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn({
        let cancellation = cancellation.clone();
        async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move { cancellation.cancelled_owned().await })
                .await;
        }
    });
    let mut config = stdio_config();
    config.protocol_mode = McpProtocolMode::Legacy;
    config.transport = McpTransportConfig::StreamableHttp(McpStreamableHttpConfig {
        url: format!("http://{address}/mcp"),
        public_headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        oauth: McpOAuthPolicy::Disabled,
    });
    let connection = McpStreamableHttpConnection::connect(
        &config,
        McpResolvedSecretHeaders::new(BTreeMap::new()).unwrap(),
        1,
        McpInteractionHub::default(),
        None,
        CancellationToken::default(),
    )
    .await
    .unwrap();
    let error = connection.discover_tools(&config).await.unwrap_err();
    assert!(
        matches!(&error, McpError::Protocol(message) if message.contains("Auth required")),
        "{error}"
    );

    let _ = connection.shutdown(Duration::from_secs(5)).await;
    cancellation.cancel();
    server.await.unwrap();
}
