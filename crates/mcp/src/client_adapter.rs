use std::collections::HashSet;

use rmcp::model::{ClientConfig, ProtocolVersion, RequestMetaObject, RequestParamsMeta};
use rmcp::service::ClientLifecycleMode;
use rmcp::transport::auth::{
    OAuthHttpClient, OAuthHttpClientError, OAuthHttpClientFuture, OAuthHttpRedirectPolicy,
    OAuthHttpRequest,
};

use crate::McpError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum McpHttpProxyPolicy {
    System,
    Disabled,
    Explicit(String),
}

pub fn build_mcp_http_client(policy: McpHttpProxyPolicy) -> Result<reqwest::Client, McpError> {
    let mut builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
    builder = match policy {
        McpHttpProxyPolicy::System => builder,
        McpHttpProxyPolicy::Disabled => builder.no_proxy(),
        McpHttpProxyPolicy::Explicit(address) => builder.proxy(
            reqwest::Proxy::all(&address)
                .map_err(|_| McpError::InvalidConfig("MCP proxy URL is invalid".into()))?,
        ),
    };
    builder
        .build()
        .map_err(|_| McpError::InvalidConfig("MCP HTTP client could not be constructed".into()))
}

pub(crate) const MAXIMUM_MCP_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const MAXIMUM_MCP_OAUTH_RESPONSE_BYTES: usize = 1024 * 1024;

pub(crate) fn lifecycle_mode(protocol_mode: crate::McpProtocolMode) -> ClientLifecycleMode {
    match protocol_mode {
        crate::McpProtocolMode::Legacy => ClientLifecycleMode::Initialize,
        crate::McpProtocolMode::V20260728 => ClientLifecycleMode::Auto {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            legacy_version: Some(ProtocolVersion::V_2025_06_18),
        },
    }
}

pub(crate) struct McpOAuthHttpClient {
    client: reqwest::Client,
}

impl McpOAuthHttpClient {
    pub(crate) fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    pub(crate) async fn execute_request(
        &self,
        mut request: reqwest::Request,
        follow_redirects: bool,
    ) -> Result<reqwest::Response, OAuthHttpClientError> {
        let mut visited = HashSet::new();
        visited.insert(request.url().to_string());
        let mut redirects = 0usize;
        loop {
            let retry = request
                .try_clone()
                .ok_or_else(|| oauth_http_error("OAuth request body cannot be replayed"))?;
            let response = self
                .client
                .execute(request)
                .await
                .map_err(|_| oauth_http_error("OAuth HTTP request failed"))?;
            if !follow_redirects || !response.status().is_redirection() {
                return Ok(response);
            }
            if redirects >= 10 {
                return Err(oauth_http_error("OAuth redirect limit exceeded"));
            }
            let location = response
                .headers()
                .get(http::header::LOCATION)
                .ok_or_else(|| oauth_http_error("OAuth redirect has no location"))?
                .to_str()
                .map_err(|_| oauth_http_error("OAuth redirect location is invalid"))?;
            let next_url = response
                .url()
                .join(location)
                .map_err(|_| oauth_http_error("OAuth redirect URL is invalid"))?;
            if !same_origin(response.url(), &next_url) {
                return Err(oauth_http_error("cross-origin OAuth redirect rejected"));
            }
            if !visited.insert(next_url.to_string()) {
                return Err(oauth_http_error("OAuth redirect cycle rejected"));
            }
            request = retry;
            *request.url_mut() = next_url;
            if matches!(
                response.status(),
                http::StatusCode::MOVED_PERMANENTLY
                    | http::StatusCode::FOUND
                    | http::StatusCode::SEE_OTHER
            ) && request.method() != http::Method::GET
                && request.method() != http::Method::HEAD
            {
                *request.method_mut() = http::Method::GET;
                *request.body_mut() = None;
                request.headers_mut().remove(http::header::CONTENT_LENGTH);
                request.headers_mut().remove(http::header::CONTENT_TYPE);
                request
                    .headers_mut()
                    .remove(http::header::TRANSFER_ENCODING);
            }
            redirects += 1;
        }
    }
}

impl OAuthHttpClient for McpOAuthHttpClient {
    fn execute(&self, request: OAuthHttpRequest) -> OAuthHttpClientFuture<'_> {
        Box::pin(async move {
            let timeout = request.timeout;
            let follow_redirects = request.redirect_policy == OAuthHttpRedirectPolicy::Follow;
            let mut request = reqwest::Request::try_from(request.request)
                .map_err(|_| oauth_http_error("OAuth request could not be constructed"))?;
            *request.timeout_mut() = timeout;
            let mut response = self.execute_request(request, follow_redirects).await?;
            let mut builder = http::Response::builder()
                .status(response.status())
                .version(response.version());
            for (name, value) in response.headers() {
                builder = builder.header(name, value);
            }
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| oauth_http_error("OAuth response body failed"))?
            {
                if chunk.len() > MAXIMUM_MCP_OAUTH_RESPONSE_BYTES - body.len() {
                    return Err(oauth_http_error("OAuth response body exceeds its limit"));
                }
                body.extend_from_slice(&chunk);
            }
            builder
                .body(body)
                .map_err(|_| oauth_http_error("OAuth response could not be constructed"))
        })
    }
}

fn same_origin(left: &url::Url, right: &url::Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
        && right.username().is_empty()
        && right.password().is_none()
}

fn oauth_http_error(message: &'static str) -> OAuthHttpClientError {
    Box::new(std::io::Error::other(message))
}

pub(crate) fn request_meta(
    protocol_version: &ProtocolVersion,
    client_info: &ClientConfig,
) -> Option<RequestMetaObject> {
    (*protocol_version == ProtocolVersion::V_2026_07_28).then(|| {
        RequestMetaObject::with_client_context(
            protocol_version.clone(),
            client_info.client_info.clone(),
            client_info.capabilities.clone(),
        )
    })
}

pub(crate) fn apply_request_meta<T: RequestParamsMeta>(
    mut params: T,
    request_meta: Option<&RequestMetaObject>,
) -> T {
    if let Some(request_meta) = request_meta {
        params.set_meta(request_meta.clone());
    }
    params
}
