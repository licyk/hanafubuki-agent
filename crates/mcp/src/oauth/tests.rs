use super::*;
use crate::McpProtocolMode;

use std::convert::Infallible;

use axum::{
    body::{to_bytes, Body},
    http::{Request, Response, StatusCode},
    routing::{get, post},
    Router,
};

fn json_response(status: StatusCode, value: serde_json::Value) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&value).unwrap()))
        .unwrap()
}

fn request_base(request: &Request<Body>) -> String {
    format!(
        "http://{}",
        request.headers().get("host").unwrap().to_str().unwrap()
    )
}

async fn protected_resource(request: Request<Body>) -> Result<Response<Body>, Infallible> {
    let base = request_base(&request);
    Ok(Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(
            "www-authenticate",
            format!("Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource\""),
        )
        .body(Body::empty())
        .unwrap())
}

async fn resource_metadata(request: Request<Body>) -> Result<Response<Body>, Infallible> {
    let base = request_base(&request);
    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "resource": format!("{base}/mcp"),
            "authorization_servers": [base],
            "scopes_supported": ["mcp", "offline_access"]
        }),
    ))
}

async fn authorization_metadata(request: Request<Body>) -> Result<Response<Body>, Infallible> {
    let base = request_base(&request);
    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "issuer": base,
            "authorization_response_iss_parameter_supported": true,
            "authorization_endpoint": format!("{base}/authorize"),
            "token_endpoint": format!("{base}/token"),
            "registration_endpoint": format!("{base}/register"),
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "token_endpoint_auth_methods_supported": ["none"],
            "code_challenge_methods_supported": ["S256"],
            "scopes_supported": ["mcp", "offline_access"]
        }),
    ))
}

async fn register_client(request: Request<Body>) -> Result<Response<Body>, Infallible> {
    let body = to_bytes(request.into_body(), 64 * 1024).await.unwrap();
    let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
    Ok(json_response(
        StatusCode::CREATED,
        serde_json::json!({
            "client_id": "desktop-test-client",
            "redirect_uris": request["redirect_uris"],
            "token_endpoint_auth_method": "none"
        }),
    ))
}

async fn exchange_token(request: Request<Body>) -> Result<Response<Body>, Infallible> {
    let body = to_bytes(request.into_body(), 64 * 1024).await.unwrap();
    let parameters = url::form_urlencoded::parse(&body)
        .into_owned()
        .collect::<std::collections::BTreeMap<_, _>>();
    let grant_type = parameters.get("grant_type").map(String::as_str);
    let access_token = match grant_type {
        Some("authorization_code") => "access-from-code",
        Some("refresh_token") => "access-from-refresh",
        _ => {
            return Ok(json_response(
                StatusCode::BAD_REQUEST,
                serde_json::json!({"error": "unsupported_grant_type"}),
            ));
        }
    };
    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "access_token": access_token,
            "token_type": "Bearer",
            "refresh_token": "fixture-refresh-token",
            "expires_in": 3600,
            "scope": "mcp offline_access"
        }),
    ))
}

async fn start_oauth_fixture() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/mcp", get(protected_resource))
        .route(
            "/.well-known/oauth-protected-resource",
            get(resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(authorization_metadata),
        )
        .route("/register", post(register_client))
        .route("/token", post(exchange_token));
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}"), task)
}

fn oauth_config(base: &str) -> McpServerConfig {
    McpServerConfig {
        id: McpServerId::parse("oauth-fixture").unwrap(),
        enabled: true,
        required: false,
        protocol_mode: McpProtocolMode::Legacy,
        transport: McpTransportConfig::StreamableHttp(crate::McpStreamableHttpConfig {
            url: format!("{base}/mcp"),
            public_headers: std::collections::BTreeMap::new(),
            credential_headers: std::collections::BTreeMap::new(),
            oauth: McpOAuthPolicy::Required,
        }),
        connect_timeout_seconds: 5,
        initialize_timeout_seconds: 5,
        list_timeout_seconds: 5,
        call_timeout_seconds: 5,
        shutdown_timeout_seconds: 5,
        maximum_concurrent_calls: 1,
        tool_maximum_concurrent_calls: std::collections::BTreeMap::new(),
        enabled_tools: std::collections::BTreeSet::new(),
        disabled_tools: std::collections::BTreeSet::new(),
        approval_policy: crate::McpApprovalPolicy::Ask,
        tool_approval_policies: std::collections::BTreeMap::new(),
        exposure_policy: crate::McpExposurePolicy::Deferred,
        tool_exposure_policies: std::collections::BTreeMap::new(),
    }
}

#[test]
fn callback_must_match_the_exact_active_redirect() {
    let redirect = validate_redirect_uri("http://127.0.0.1:43123/mcp/callback").unwrap();
    validate_callback(
        &redirect,
        "http://127.0.0.1:43123/mcp/callback?code=one&state=two",
    )
    .unwrap();
    for callback in [
        "http://127.0.0.1:43124/mcp/callback?code=one&state=two",
        "http://localhost:43123/mcp/callback?code=one&state=two",
        "http://127.0.0.1:43123/other?code=one&state=two",
        "http://user@127.0.0.1:43123/mcp/callback?code=one&state=two",
        "http://127.0.0.1:43123/mcp/callback?code=one&state=two#fragment",
    ] {
        assert!(
            validate_callback(&redirect, callback).is_err(),
            "{callback}"
        );
    }
}

#[test]
fn oauth_credential_key_is_stable_and_does_not_embed_the_server_id() {
    let server_id = McpServerId::parse("private-server-name").unwrap();
    let first = oauth_credential_key(&server_id).unwrap();
    let second = oauth_credential_key(&server_id).unwrap();
    assert_eq!(first, second);
    assert!(first.as_str().starts_with("oauth."));
    assert!(!first.as_str().contains(server_id.as_str()));
}

#[test]
fn redirect_rejects_credentials_query_fragment_and_non_http_schemes() {
    for redirect in [
        "file:///tmp/callback",
        "http://user@localhost/callback",
        "http://localhost/callback?query=1",
        "http://localhost/callback#fragment",
    ] {
        assert!(validate_redirect_uri(redirect).is_err(), "{redirect}");
    }
}

#[tokio::test]
async fn oauth_fixture_covers_discovery_pkce_exchange_refresh_and_logout() {
    let (base, server) = start_oauth_fixture().await;
    let config = oauth_config(&base);
    let store = Arc::new(crate::MemoryMcpSecretStore::default());
    let manager = McpOAuthManager::new(
        store.clone(),
        crate::build_mcp_http_client(crate::McpHttpProxyPolicy::Disabled).unwrap(),
    );
    let redirect = "http://127.0.0.1:53682/mcp/oauth/callback";

    let login = manager
        .start_login(&config, redirect.to_string())
        .await
        .unwrap();
    let authorization_url = url::Url::parse(&login.authorization_url).unwrap();
    let query = authorization_url
        .query_pairs()
        .into_owned()
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        query.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert!(query
        .get("code_challenge")
        .is_some_and(|value| !value.is_empty()));
    let state = query.get("state").unwrap();

    let mut wrong_callback = url::Url::parse(redirect).unwrap();
    wrong_callback
        .query_pairs_mut()
        .append_pair("code", "fixture-code")
        .append_pair("state", state)
        .append_pair("iss", "https://attacker.invalid");
    assert!(manager
        .complete_login(&config, login.flow_id, wrong_callback.into())
        .await
        .is_err());

    let login = manager
        .start_login(&config, redirect.to_string())
        .await
        .unwrap();
    let authorization_url = url::Url::parse(&login.authorization_url).unwrap();
    let query = authorization_url
        .query_pairs()
        .into_owned()
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut callback = url::Url::parse(redirect).unwrap();
    callback
        .query_pairs_mut()
        .append_pair("code", "fixture-code")
        .append_pair("state", query.get("state").unwrap())
        .append_pair("iss", &base);
    let status = manager
        .complete_login(&config, login.flow_id, callback.into())
        .await
        .unwrap();
    assert_eq!(status.phase, McpOAuthPhase::Authorized);
    assert_eq!(
        manager.access_token(&config).await.unwrap().as_deref(),
        Some("access-from-code")
    );

    let refreshed = manager.refresh(&config).await.unwrap();
    assert_eq!(refreshed.phase, McpOAuthPhase::Authorized);
    assert_eq!(
        manager.access_token(&config).await.unwrap().as_deref(),
        Some("access-from-refresh")
    );
    let key = oauth_credential_key(&config.id).unwrap();
    let before_failed_refresh = store.get(&key).await.unwrap().unwrap();
    server.abort();
    let _ = server.await;
    assert!(manager.refresh(&config).await.is_err());
    assert_eq!(
        store.get(&key).await.unwrap().as_deref(),
        Some(before_failed_refresh.as_slice())
    );
    assert_eq!(
        manager.logout(&config).await.unwrap().phase,
        McpOAuthPhase::LoggedOut
    );
    assert!(manager.access_token(&config).await.is_err());
}

#[tokio::test]
async fn oauth_flow_identity_prevents_callback_mixup_and_cancelled_flow_reuse() {
    let (base, server) = start_oauth_fixture().await;
    let mut first = oauth_config(&base);
    first.id = McpServerId::parse("oauth-first").unwrap();
    let mut second = oauth_config(&base);
    second.id = McpServerId::parse("oauth-second").unwrap();
    let store = Arc::new(crate::MemoryMcpSecretStore::default());
    let manager = McpOAuthManager::new(
        store,
        crate::build_mcp_http_client(crate::McpHttpProxyPolicy::Disabled).unwrap(),
    );
    let redirect = "http://127.0.0.1:53682/mcp/oauth/callback";
    let first_login = manager
        .start_login(&first, redirect.to_string())
        .await
        .unwrap();
    let second_login = manager
        .start_login(&second, redirect.to_string())
        .await
        .unwrap();
    let second_authorization = url::Url::parse(&second_login.authorization_url).unwrap();
    let second_state = second_authorization
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
        .unwrap();
    let mut second_callback = url::Url::parse(redirect).unwrap();
    second_callback
        .query_pairs_mut()
        .append_pair("code", "second-code")
        .append_pair("state", &second_state)
        .append_pair("iss", &base);

    assert_eq!(
        manager
            .complete_login(&first, second_login.flow_id, second_callback.to_string())
            .await
            .unwrap_err(),
        McpError::StaleGeneration
    );
    assert_eq!(
        manager
            .cancel_login(&second, second_login.flow_id)
            .unwrap()
            .phase,
        McpOAuthPhase::LoggedOut
    );
    assert_eq!(
        manager
            .complete_login(&second, second_login.flow_id, second_callback.to_string())
            .await
            .unwrap_err(),
        McpError::StaleGeneration
    );
    assert_eq!(
        manager
            .cancel_login(&first, first_login.flow_id)
            .unwrap()
            .phase,
        McpOAuthPhase::LoggedOut
    );

    server.abort();
    let _ = server.await;
}
