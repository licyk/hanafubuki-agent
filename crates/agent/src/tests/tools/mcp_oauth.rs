//! The browser login of an OAuth MCP server: the loopback receives the
//! authorization server's redirect, against the OAuth fixture, with a test
//! playing the browser.

use super::mcp::mcp_application;
use super::*;
use crate::McpServerWriteRequest;
use hanafubuki_agent_mcp::test_support::oauth::{oauth_config, start_oauth_fixture};
use hanafubuki_agent_mcp::{McpOAuthPhase, McpServerConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;

#[tokio::test]
async fn an_oauth_login_completes_on_the_browsers_redirect() {
    let (base, fixture) = start_oauth_fixture().await;
    let paths = test_paths("mcp-oauth-login");
    let application = mcp_application(&paths);
    let server = configure(&application, oauth_config(&base)).await;
    let (login, authorization) = start_login(&application, &server, std::future::pending());
    let authorization = authorization.await.unwrap();

    let redirect = &authorization["redirect_uri"];
    assert!(redirect.starts_with("http://127.0.0.1:"), "{redirect}");
    assert!(redirect.ends_with("/callback"), "{redirect}");
    let favicon = browse(&redirect.replace("/callback", "/favicon.ico")).await;
    assert!(favicon.starts_with("HTTP/1.1 404"), "{favicon}");
    let mut callback = url::Url::parse(redirect).unwrap();
    callback
        .query_pairs_mut()
        .append_pair("code", "fixture-code")
        .append_pair("state", &authorization["state"])
        .append_pair("iss", &base);
    let page = browse(callback.as_str()).await;
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert!(page.contains("You can close this tab."), "{page}");

    let snapshot = login.await.unwrap().unwrap();
    assert_eq!(phase(&snapshot, &server), McpOAuthPhase::Authorized);
    fixture.abort();
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A refusal from the authorization server, or a cancellation, ends the login
/// with its reason, cancels the flow and closes the loopback.
#[tokio::test]
async fn a_refused_or_cancelled_oauth_login_cancels_its_flow() {
    let (base, fixture) = start_oauth_fixture().await;
    let paths = test_paths("mcp-oauth-login-refused");
    let application = mcp_application(&paths);
    let server = configure(&application, oauth_config(&base)).await;

    let (login, authorization) = start_login(&application, &server, std::future::pending());
    let authorization = authorization.await.unwrap();
    let mut callback = url::Url::parse(&authorization["redirect_uri"]).unwrap();
    callback
        .query_pairs_mut()
        .append_pair("error", "access_denied")
        .append_pair("error_description", "The user said no");
    let page = browse(callback.as_str()).await;
    assert!(page.starts_with("HTTP/1.1 400"), "{page}");
    let error = login.await.unwrap().unwrap_err();
    assert!(error.contains("access_denied: The user said no"), "{error}");
    let snapshot = application.mcp_snapshot().await.unwrap();
    assert_eq!(phase(&snapshot, &server), McpOAuthPhase::LoggedOut);

    let (cancel, cancelled) = oneshot::channel::<()>();
    let (login, authorization) = start_login(&application, &server, async {
        let _ = cancelled.await;
    });
    let redirect = authorization.await.unwrap()["redirect_uri"].clone();
    assert_eq!(
        phase(&application.mcp_snapshot().await.unwrap(), &server),
        McpOAuthPhase::Authorizing
    );
    cancel.send(()).unwrap();
    let error = login.await.unwrap().unwrap_err();
    assert_eq!(error, "the OAuth login was cancelled");
    assert_eq!(
        phase(&application.mcp_snapshot().await.unwrap(), &server),
        McpOAuthPhase::LoggedOut
    );
    let address = url::Url::parse(&redirect).unwrap();
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", address.port().unwrap()))
            .await
            .is_err(),
        "the loopback is closed"
    );
    fixture.abort();
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Writes `server`, disabled so nothing connects it on the side.
async fn configure(
    application: &AgentMcpApplication,
    mut server: McpServerConfig,
) -> McpServerConfig {
    server.enabled = false;
    let current = application.mcp_snapshot().await.unwrap();
    application
        .mcp_write_server(McpServerWriteRequest {
            expected_revision: current.settings.revision,
            server: server.clone(),
        })
        .await
        .unwrap();
    server
}

type Login = tokio::task::JoinHandle<Result<McpApplicationSnapshot, String>>;

/// Starts a login of `server`; the second value is the query of the
/// authorization URL it hands to the browser.
fn start_login(
    application: &AgentMcpApplication,
    server: &McpServerConfig,
    cancelled: impl std::future::Future<Output = ()> + Send + 'static,
) -> (Login, oneshot::Receiver<BTreeMap<String, String>>) {
    let (authorization, opened) = oneshot::channel();
    let application = application.clone();
    let server_id = server.id.clone();
    let login = tokio::spawn(async move {
        application
            .mcp_oauth_login(
                server_id,
                move |url| {
                    let query = url::Url::parse(url)
                        .unwrap()
                        .query_pairs()
                        .into_owned()
                        .collect();
                    authorization.send(query).unwrap();
                },
                cancelled,
            )
            .await
    });
    (login, opened)
}

/// What a browser gets for `url`.
async fn browse(url: &str) -> String {
    let url = url::Url::parse(url).unwrap();
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", url.port().unwrap()))
        .await
        .unwrap();
    let target = match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_string(),
    };
    stream
        .write_all(
            format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: text/html\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    response
}

fn phase(snapshot: &McpApplicationSnapshot, server: &McpServerConfig) -> McpOAuthPhase {
    snapshot
        .oauth
        .iter()
        .find(|status| status.server_id == server.id)
        .unwrap()
        .phase
}
