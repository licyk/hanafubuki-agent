use super::*;

/// Relays the server's stdout frames until the process or the transport
/// ends. Why the server ended is in `exit_reason` before `inbound` closes,
/// so the connection's watcher, which wakes when it closes, can name it.
pub(super) fn run_stdio_worker(
    manager: Arc<ExecutorManager>,
    receipt: ReservedExecutorReceipt,
    commands: mpsc::Receiver<WorkerCommand>,
    inbound: tokio_mpsc::Sender<ServerJsonRpcMessage>,
    diagnostic: Arc<Mutex<String>>,
    exit_reason: Arc<Mutex<Option<String>>>,
    cancellation: CancellationToken,
) {
    let reason = relay(
        &manager,
        &receipt,
        &commands,
        &inbound,
        &diagnostic,
        &cancellation,
    );
    *exit_reason.lock() = reason;
}

/// `None` when the client ended it (a close, the transport dropped, a
/// cancellation); otherwise why the server's side ended.
fn relay(
    manager: &ExecutorManager,
    receipt: &ReservedExecutorReceipt,
    commands: &mpsc::Receiver<WorkerCommand>,
    inbound: &tokio_mpsc::Sender<ServerJsonRpcMessage>,
    diagnostic: &Mutex<String>,
    cancellation: &CancellationToken,
) -> Option<String> {
    let mut offset = 0u64;
    let mut protocol = Vec::<u8>::new();
    loop {
        while let Ok(command) = commands.try_recv() {
            match command {
                WorkerCommand::Send {
                    message,
                    completion,
                } => {
                    let result = serde_json::to_vec(&message)
                        .map_err(|error| McpExecutorTransportError::Failed(error.to_string()))
                        .and_then(|mut bytes| {
                            if bytes.len() > crate::client_adapter::MAXIMUM_MCP_MESSAGE_BYTES {
                                return Err(McpExecutorTransportError::Failed(
                                    "MCP outbound JSON-RPC frame exceeds its limit".into(),
                                ));
                            }
                            bytes.push(b'\n');
                            write_reserved_executor_stdin(manager, receipt, bytes, false).map_err(
                                |error| McpExecutorTransportError::Failed(bounded_error(error)),
                            )
                        });
                    let _ = completion.send(result);
                }
                WorkerCommand::Close { completion } => {
                    let _ = completion.send(Ok(()));
                    return None;
                }
            }
        }
        match read_reserved_executor_output(
            manager,
            receipt,
            Some(offset),
            Some(MCP_PROTOCOL_READ_BYTES),
        ) {
            Ok(output) => {
                if output.truncated {
                    return Some(ended(diagnostic, "MCP stdout was truncated before framing"));
                }
                offset = output.next_offset;
                for chunk in output.chunks {
                    match chunk.stream {
                        OutputStream::Stdout => {
                            if chunk.data.len()
                                > crate::client_adapter::MAXIMUM_MCP_MESSAGE_BYTES
                                    .saturating_sub(protocol.len())
                            {
                                return Some(ended(
                                    diagnostic,
                                    "MCP JSON-RPC frame exceeds its limit",
                                ));
                            }
                            protocol.extend_from_slice(&chunk.data);
                        }
                        OutputStream::Stderr => {
                            append_diagnostic(diagnostic, &String::from_utf8_lossy(&chunk.data))
                        }
                        OutputStream::Merged => {
                            return Some(ended(
                                diagnostic,
                                "MCP pipe output was unexpectedly merged",
                            ));
                        }
                    }
                }
                while let Some(newline) = protocol.iter().position(|byte| *byte == b'\n') {
                    let mut line = protocol.drain(..=newline).collect::<Vec<_>>();
                    line.pop();
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    if line.is_empty() {
                        continue;
                    }
                    match serde_json::from_slice::<ServerJsonRpcMessage>(&line) {
                        Ok(message) => {
                            if inbound.blocking_send(message).is_err() {
                                return None;
                            }
                        }
                        Err(error) => {
                            return Some(ended(
                                diagnostic,
                                &format!("invalid MCP JSON-RPC frame: {error}"),
                            ));
                        }
                    }
                }
                if output.status.is_terminal() {
                    if !protocol.is_empty() {
                        append_diagnostic(
                            diagnostic,
                            "MCP stdout ended with an incomplete JSON-RPC frame\n",
                        );
                    }
                    return Some(match output.exit_code {
                        Some(code) => format!("the MCP server process exited with code {code}"),
                        None => "the MCP server process exited".to_string(),
                    });
                }
            }
            Err(error) => {
                return Some(ended(
                    diagnostic,
                    &format!("MCP executor output failed: {}", bounded_error(error)),
                ));
            }
        }
        if cancellation.wait_timeout(MCP_POLL_INTERVAL) {
            return None;
        }
    }
}

/// Records why the relay ended in the diagnostic and returns it.
fn ended(diagnostic: &Mutex<String>, reason: &str) -> String {
    append_diagnostic(diagnostic, &format!("{reason}\n"));
    reason.to_string()
}

pub(super) fn append_diagnostic(target: &Mutex<String>, text: &str) {
    let mut target = target.lock();
    target.push_str(text);
    if target.len() > MCP_DIAGNOSTIC_BYTES {
        let remove = target.len() - MCP_DIAGNOSTIC_BYTES;
        let boundary = target
            .char_indices()
            .find_map(|(index, _)| (index >= remove).then_some(index))
            .unwrap_or(target.len());
        target.drain(..boundary);
    }
}

pub(super) fn bounded_error(message: String) -> String {
    crate::sanitize_mcp_diagnostic(&message, 2048)
}
