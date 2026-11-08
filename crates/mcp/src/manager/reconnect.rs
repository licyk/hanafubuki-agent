//! Liveness and automatic reconnect. A connection whose transport closed on
//! its own (its stdio process exited, its HTTP transport ended), or whose
//! tool call panicked in the client, makes its server `Failed` with the
//! reason at once: the entry is retired, so its tools leave the next step's
//! plan and the process is disposed through the retired list with its
//! receipt, like any other. The manager then connects the server again, with
//! the configuration and secrets of the connection it lost, after the delays
//! of its `RestartPolicy`; once the window's reconnects are used up the
//! server stays `Failed` until a connect from outside (`connect_or_refresh`),
//! which empties the window.

use std::sync::Weak;
use std::time::Instant;

use super::*;

impl McpManager {
    /// Watches `connection`, just made active for `server_id`, until its
    /// transport closes. A connection the manager retired first (a refresh,
    /// a disconnect, shutdown) closes when it is cleaned up and is ignored.
    pub(super) fn watch_connection(
        &self,
        server_id: McpServerId,
        connection: Arc<McpLiveConnection>,
    ) {
        let manager = self.clone();
        let closed = connection.closed();
        // Weak: the cleanup disposes a connection only once nothing else
        // holds it.
        let watched = Arc::downgrade(&connection);
        tokio::spawn(async move {
            crate::liveness::transport_closed(closed).await;
            if let Some((config, secrets)) =
                manager.retire_lost(&server_id, &watched, McpConnectionLoss::Closed)
            {
                manager.reconnect(config, secrets).await;
            }
        });
    }

    /// Marks the server of `lease` `Failed` after its tool call panicked in
    /// the client, with the panic's `message`: the connection the lease pins
    /// is in an unknown state, so it is retired and the server reconnected,
    /// as if its transport had closed. Nothing happens if the server has
    /// another connection by now.
    pub fn fail_after_panic(&self, lease: &McpToolLease, message: String) {
        let server_id = lease.binding.server_id.clone();
        let watched = Arc::downgrade(&lease.connection);
        let Some((config, secrets)) =
            self.retire_lost(&server_id, &watched, McpConnectionLoss::Panicked(message))
        else {
            return;
        };
        let manager = self.clone();
        tokio::spawn(async move { manager.reconnect(config, secrets).await });
    }

    /// Retires the active entry of `server_id` if its connection is
    /// `watched`, and marks the server `Failed` with why it was lost.
    fn retire_lost(
        &self,
        server_id: &McpServerId,
        watched: &Weak<McpLiveConnection>,
        loss: McpConnectionLoss,
    ) -> Option<(McpServerConfig, McpConnectionSecrets)> {
        let mut registry = self.registry.lock();
        let current = registry
            .active
            .get(server_id)
            .is_some_and(|entry| std::ptr::eq(Arc::as_ptr(&entry.connection), watched.as_ptr()));
        if !current {
            return None;
        }
        let entry = registry.active.remove(server_id)?;
        let (code, reason) = match loss {
            McpConnectionLoss::Closed => ("connection_closed", entry.connection.closed_reason()),
            McpConnectionLoss::Panicked(message) => (
                "tool_call_panicked",
                format!("the MCP client panicked during a tool call: {message}"),
            ),
        };
        let reason = bounded_error(reason);
        let mut status = entry_status(server_id, McpConnectionPhase::Failed, None);
        status.cleanup_pending = true;
        status.last_error = Some(reason.clone());
        registry.statuses.insert(server_id.clone(), status);
        let lost = (entry.config.clone(), entry.secrets.clone());
        let runtime_generation = entry.connection.generation();
        let stderr = entry.connection.diagnostic_tail();
        registry.retired.push(entry);
        drop(registry);
        log::warn!(
            "MCP server connection lost id={}: {reason}",
            server_id.as_str()
        );
        self.diagnostics.record(
            server_id.clone(),
            McpDiagnosticStage::ConnectionLost,
            McpDiagnosticOutcome::Error,
            code,
            reason,
            transport_name(&lost.0),
            config_generation(&lost.0).unwrap_or([0; 32]),
            Some(runtime_generation),
            None,
            true,
            "reconnect",
        );
        if let Some(stderr) = stderr {
            self.diagnostics.set_stderr_tail(server_id.clone(), &stderr);
        }
        self.emit(server_id.clone(), McpEventKind::ServerStatus);
        self.emit(server_id.clone(), McpEventKind::ToolCatalog);
        Some(lost)
    }

    async fn reconnect(&self, config: McpServerConfig, secrets: McpConnectionSecrets) {
        loop {
            // Disposes the closed connection once no step holds it. A cleanup
            // that does not confirm is the server's status and diagnostics;
            // the reconnect does not depend on it.
            let _ = self.cleanup_retired().await;
            if !self.awaits_reconnect(&config.id) {
                return;
            }
            let Some(delay) = self.schedule_reconnect(&config) else {
                return;
            };
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = wait_cancelled(&self.lifetime) => return,
            }
            if !self.awaits_reconnect(&config.id) {
                return;
            }
            log::info!("MCP server reconnecting id={}", config.id.as_str());
            let connected = self
                .connect(config.clone(), secrets.clone(), self.lifetime.child())
                .await;
            if connected.is_ok() {
                return;
            }
        }
    }

    /// Counts the next automatic reconnect of the server in its window and
    /// shows it in the status; `None` once the window's are used up.
    fn schedule_reconnect(&self, config: &McpServerConfig) -> Option<Duration> {
        let server_id = &config.id;
        let now = Instant::now();
        let mut registry = self.registry.lock();
        let window = registry.reconnects.entry(server_id.clone()).or_default();
        let (made, delay) = window.next(&self.reconnect_policy, now);
        if delay.is_some() {
            window.record(now);
        }
        let maximum = self.reconnect_policy.delays.len();
        if let Some(status) = registry.statuses.get_mut(server_id) {
            status.reconnect = Some(crate::McpReconnectStatus {
                next_attempt: delay.map(|_| made + 1),
                maximum,
            });
        }
        drop(registry);
        let (code, message) = match delay {
            Some(delay) => (
                "reconnect_scheduled",
                format!(
                    "automatic reconnect {} of {maximum} in {} ms",
                    made + 1,
                    delay.as_millis()
                ),
            ),
            None => (
                "reconnects_exhausted",
                format!("the {maximum} automatic reconnects of the window are used up"),
            ),
        };
        self.diagnostics.record(
            server_id.clone(),
            McpDiagnosticStage::ConnectionLost,
            McpDiagnosticOutcome::Warning,
            code,
            message,
            transport_name(config),
            config_generation(config).unwrap_or([0; 32]),
            None,
            None,
            delay.is_none(),
            if delay.is_some() {
                "wait"
            } else {
                "retry_connect"
            },
        );
        self.emit(server_id.clone(), McpEventKind::ServerStatus);
        delay
    }

    /// Whether `server_id` is still `Failed` with nothing connecting it: a
    /// connect, refresh or disconnect from outside since takes precedence.
    fn awaits_reconnect(&self, server_id: &McpServerId) -> bool {
        let registry = self.registry.lock();
        !registry.active.contains_key(server_id)
            && !registry.operations.contains(server_id)
            && registry
                .statuses
                .get(server_id)
                .is_some_and(|status| status.phase == McpConnectionPhase::Failed)
    }
}

/// Why the manager retires a server's active connection on its own.
enum McpConnectionLoss {
    /// Its transport closed.
    Closed,
    /// One of its tool calls panicked in the client, with this message.
    Panicked(String),
}
