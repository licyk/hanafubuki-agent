use std::future::Future;
use std::time::{Duration, Instant};

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_tool_contracts::ToolOutput;
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResult, ClientRequest, ReadResourceRequest,
    ReadResourceRequestParams, RequestMetaObject, ServerResult,
};
use rmcp::service::PeerRequestOptions;
use rmcp::{Peer, RoleClient};

use crate::McpError;

pub(crate) async fn call_tool_cancellable(
    peer: &Peer<RoleClient>,
    request: CallToolRequestParams,
    timeout: Duration,
    pause_state: crate::interaction::ElicitationPauseState,
    cancellation: CancellationToken,
) -> Result<CallToolResult, McpError> {
    if cancellation.is_cancelled() {
        return Err(McpError::Cancelled);
    }
    let request = ClientRequest::CallToolRequest(CallToolRequest::new(request));
    let mut handle = peer
        .send_cancellable_request(request, PeerRequestOptions::no_options())
        .await
        .map_err(|error| McpError::Protocol(bounded_error(error.to_string())))?;
    let outcome = tokio::select! {
        response = active_time_timeout(timeout, pause_state.subscribe(), &mut handle.rx) => {
            Some(response)
        }
        _ = wait_cancelled(&cancellation) => None,
    };
    match outcome {
        Some(Ok(response)) => {
            let response = response
                .map_err(|_| {
                    McpError::Protocol("MCP transport closed before the tool response".into())
                })?
                .map_err(|error| McpError::Protocol(bounded_error(error.to_string())))?;
            match response {
                ServerResult::CallToolResult(result) => Some(result),
                // rmcp falls back to a custom result when one content block
                // does not decode; that block alone becomes a placeholder.
                ServerResult::CustomResult(raw) => {
                    crate::output::decode_call_tool_result_leniently(raw.0)
                }
                _ => None,
            }
            .ok_or_else(|| {
                McpError::Protocol("MCP server returned an unexpected response type".into())
            })
        }
        Some(Err(())) => {
            handle
                .cancel(Some("local MCP tool timeout".to_string()))
                .await
                .map_err(|error| McpError::Protocol(bounded_error(error.to_string())))?;
            Err(McpError::Timeout)
        }
        None => {
            handle
                .cancel(Some("local Agent cancellation".to_string()))
                .await
                .map_err(|error| McpError::Protocol(bounded_error(error.to_string())))?;
            Err(McpError::Cancelled)
        }
    }
}

/// Reads one resource through `Peer::send_request`, like tools/call, so an
/// entry rmcp cannot decode costs only itself
/// ([`crate::output::convert_read_resource_output`]).
pub(crate) async fn read_resource(
    peer: &Peer<RoleClient>,
    uri: String,
    timeout: Duration,
    request_meta: Option<&RequestMetaObject>,
) -> Result<ToolOutput, McpError> {
    if uri.len() > 16 * 1024 || !uri.contains(':') {
        return Err(McpError::InvalidConfig("resource URI is invalid".into()));
    }
    let request = ClientRequest::ReadResourceRequest(ReadResourceRequest::new(
        crate::client_adapter::apply_request_meta(
            ReadResourceRequestParams::new(uri),
            request_meta,
        ),
    ));
    let result = tokio::time::timeout(timeout, peer.send_request(request))
        .await
        .map_err(|_| McpError::Timeout)?
        .map_err(|error| McpError::Protocol(bounded_error(error.to_string())))?;
    crate::output::convert_read_resource_output(result)
}

pub(crate) async fn active_time_timeout<T, Fut>(
    duration: Duration,
    mut pause_state: tokio::sync::watch::Receiver<bool>,
    operation: Fut,
) -> Result<T, ()>
where
    Fut: Future<Output = T>,
{
    let mut remaining = duration;
    tokio::pin!(operation);
    loop {
        if *pause_state.borrow_and_update() {
            tokio::select! {
                result = &mut operation => return Ok(result),
                changed = pause_state.changed() => {
                    if changed.is_err() {
                        return tokio::time::timeout(remaining, operation)
                            .await
                            .map_err(|_| ());
                    }
                    let _paused = *pause_state.borrow_and_update();
                }
            }
            continue;
        }

        let active_start = Instant::now();
        tokio::select! {
            result = &mut operation => return Ok(result),
            _ = tokio::time::sleep(remaining) => return Err(()),
            changed = pause_state.changed() => {
                if changed.is_err() {
                    return tokio::time::timeout(remaining, operation)
                        .await
                        .map_err(|_| ());
                }
                if *pause_state.borrow_and_update() {
                    remaining = remaining.saturating_sub(active_start.elapsed());
                    if remaining.is_zero() {
                        return Err(());
                    }
                }
            }
        }
    }
}

pub(crate) async fn wait_cancelled(cancellation: &CancellationToken) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn bounded_error(message: String) -> String {
    crate::sanitize_mcp_diagnostic(&message, 2048)
}

#[cfg(test)]
mod tests;
