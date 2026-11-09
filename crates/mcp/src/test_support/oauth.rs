//! An OAuth authorization server for tests: protected-resource and
//! authorization-server metadata, dynamic client registration and a token
//! endpoint, on a loopback port.

use std::convert::Infallible;

use axum::{
    body::{to_bytes, Body},
    http::{Request, Response, StatusCode},
    routing::{get, post},
    Router,
};

use crate::{McpOAuthPolicy, McpProtocolMode, McpServerConfig, McpServerId, McpTransportConfig};
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

/// Serves the fixture until the task is aborted; returns its base URL.
pub async fn start_oauth_fixture() -> (String, tokio::task::JoinHandle<()>) {
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

/// A Streamable HTTP server `oauth-fixture` at the fixture's `/mcp` that
/// requires OAuth.
pub fn oauth_config(base: &str) -> McpServerConfig {
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
