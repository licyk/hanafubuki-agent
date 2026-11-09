// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/rmcp-client/src/perform_oauth_login.rs` at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: a tokio listener reading the request head
// itself in place of `tiny_http`; the flow is `mcp_oauth_start` and
// `mcp_oauth_complete`, which checks the whole callback URL; the browser is
// opened by the caller, which also decides when to cancel.

//! The OAuth login of an MCP server through the browser, with the redirect
//! received on a loopback port.

use std::future::Future;
use std::net::Ipv4Addr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::AgentMcpApplication;
use crate::{
    McpApplicationSnapshot, McpOAuthCallbackRequest, McpOAuthCancelRequest, McpOAuthLoginRequest,
};

/// How long the login waits for the browser, as in Codex.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const CALLBACK_PATH: &str = "/callback";
/// A request head larger than this is not the authorization server's
/// redirect.
const MAX_REQUEST_HEAD_BYTES: usize = 32 * 1024;
/// How long one connection may take to send its request head, so a stalled
/// one does not hold up the callback behind it.
const REQUEST_HEAD_TIMEOUT: Duration = Duration::from_secs(10);

const AUTHORIZED_PAGE: &str = "<!doctype html><meta charset=\"utf-8\"><title>Authorized</title>\
<p>The authorization was received. You can close this tab.</p>";
const REFUSED_PAGE: &str = "<!doctype html><meta charset=\"utf-8\"><title>Not authorized</title>\
<p>The authorization was refused. You can close this tab.</p>";

impl AgentMcpApplication {
    pub async fn mcp_oauth_login(
        &self,
        server_id: hanafubuki_agent_mcp::McpServerId,
        open_url: impl FnOnce(&str) + Send,
        cancelled: impl Future<Output = ()> + Send,
    ) -> Result<McpApplicationSnapshot, String> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(listener_error)?;
        let port = listener.local_addr().map_err(listener_error)?.port();
        let login = self
            .mcp_oauth_start(McpOAuthLoginRequest {
                server_id: server_id.clone(),
                redirect_uri: format!("http://127.0.0.1:{port}{CALLBACK_PATH}"),
            })
            .await?;
        open_url(&login.authorization_url);
        let callback = tokio::select! {
            callback = tokio::time::timeout(LOGIN_TIMEOUT, wait_for_callback(&listener, port)) => {
                callback.unwrap_or_else(|_| {
                    Err("no OAuth authorization came back within 5 minutes".to_string())
                })
            }
            () = cancelled => Err("the OAuth login was cancelled".to_string()),
        };
        drop(listener);
        match callback {
            Ok(callback_url) => {
                self.mcp_oauth_complete(McpOAuthCallbackRequest {
                    server_id,
                    flow_id: login.flow_id,
                    callback_url,
                })
                .await
            }
            Err(error) => {
                // The server would otherwise stay `authorizing` until the flow
                // expires; a flow already gone is no further error.
                let _ = self
                    .mcp_oauth_cancel(McpOAuthCancelRequest {
                        server_id,
                        flow_id: login.flow_id,
                    })
                    .await;
                Err(error)
            }
        }
    }
}

fn listener_error(error: std::io::Error) -> String {
    format!("the OAuth callback listener cannot start: {error}")
}

/// Answers the loopback's requests until the authorization server's redirect
/// comes: the callback URL to complete the flow with, or the refusal it
/// carries. Anything else (a browser's `/favicon.ico`) is answered 404.
async fn wait_for_callback(listener: &TcpListener, port: u16) -> Result<String, String> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|error| format!("the OAuth callback listener failed: {error}"))?;
        let Ok(Some(target)) =
            tokio::time::timeout(REQUEST_HEAD_TIMEOUT, read_request_target(&mut stream)).await
        else {
            continue;
        };
        match parse_callback(&target) {
            Callback::Authorized => {
                respond(&mut stream, "200 OK", AUTHORIZED_PAGE).await;
                return Ok(format!("http://127.0.0.1:{port}{target}"));
            }
            Callback::Refused(refusal) => {
                respond(&mut stream, "400 Bad Request", REFUSED_PAGE).await;
                return Err(refusal);
            }
            Callback::Unrelated => respond(&mut stream, "404 Not Found", "Not found").await,
        }
    }
}

/// The target of a `GET` request, once its whole head is read: a connection
/// closed with unread input is reset, and the browser would show that instead
/// of the page.
async fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut buffer = [0; 4096];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        if head.len() > MAX_REQUEST_HEAD_BYTES {
            return None;
        }
        let read = stream.read(&mut buffer).await.ok()?;
        if read == 0 {
            return None;
        }
        head.extend_from_slice(&buffer[..read]);
    }
    let line = head.split(|byte| *byte == b'\n').next()?;
    let mut parts = std::str::from_utf8(line).ok()?.trim_end().split(' ');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("GET"), Some(target), Some(version))
            if target.starts_with('/') && version.starts_with("HTTP/1.") =>
        {
            Some(target.to_string())
        }
        _ => None,
    }
}

enum Callback {
    Authorized,
    Refused(String),
    Unrelated,
}

/// What a request to the loopback is. The code and state are not checked
/// here: `mcp_oauth_complete` checks the whole URL against the flow.
fn parse_callback(target: &str) -> Callback {
    let Ok(url) = url::Url::parse(&format!("http://127.0.0.1{target}")) else {
        return Callback::Unrelated;
    };
    if url.path() != CALLBACK_PATH {
        return Callback::Unrelated;
    }
    let mut code = false;
    let mut error = None;
    let mut description = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = true,
            "error" => error = Some(value.into_owned()),
            "error_description" => description = Some(value.into_owned()),
            _ => {}
        }
    }
    match (error, description) {
        (Some(error), Some(description)) => Callback::Refused(format!(
            "the authorization server refused the login: {error}: {description}"
        )),
        (Some(error), None) => Callback::Refused(format!(
            "the authorization server refused the login: {error}"
        )),
        (None, _) if code => Callback::Authorized,
        (None, _) => Callback::Unrelated,
    }
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // The flow does not depend on the browser getting its page.
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}
