use super::*;

impl McpManager {
    pub async fn disconnect(&self, server_id: &McpServerId) -> Result<(), McpError> {
        let entry = {
            let mut registry = self.registry.lock();
            let entry = registry.active.remove(server_id);
            if let Some(entry) = &entry {
                registry.retired.push(entry.clone());
            }
            let status = entry.as_ref().map_or_else(
                || McpServerStatus::disabled(server_id.clone()),
                |entry| McpServerStatus {
                    server_id: server_id.clone(),
                    phase: McpConnectionPhase::Disabled,
                    connection_generation: Some(entry.connection.generation()),
                    catalog_generation: entry.tools.first().map(|tool| tool.catalog_generation),
                    tool_count: 0,
                    resource_count: 0,
                    resource_template_count: 0,
                    resolved_program: entry.connection.resolved_program(),
                    capabilities: Some(entry.connection.capabilities().stale()),
                    cleanup_pending: true,
                    last_error: None,
                    reconnect: None,
                },
            );
            registry.statuses.insert(server_id.clone(), status);
            entry
        };
        if entry.is_none() {
            return Ok(());
        }
        // The registry's retired entry must be the only baseline strong
        // reference before cleanup checks for active plan/call leases.
        drop(entry);
        log::info!("MCP server disconnect id={}", server_id.as_str());
        self.emit(server_id.clone(), McpEventKind::ServerStatus);
        self.cleanup_retired().await?;
        if self
            .registry
            .lock()
            .retired
            .iter()
            .any(|entry| &entry.config.id == server_id)
        {
            log::warn!(
                "MCP server disconnect unconfirmed id={}: an active tool plan or call still retains the connection",
                server_id.as_str()
            );
            return Err(McpError::CleanupUnconfirmed(
                "an active tool plan or call still retains this MCP connection".into(),
            ));
        }
        Ok(())
    }

    pub async fn cleanup_retired(&self) -> Result<(), McpError> {
        self.cleanup_retired_until(None).await
    }

    /// Shuts down every retired connection no plan or call still holds,
    /// each within its `shutdown_timeout` and, with a `deadline`, by then: a
    /// connection that is not confirmed closed in time stays retired with
    /// its cleanup obligation, as one whose shutdown failed.
    async fn cleanup_retired_until(
        &self,
        deadline: Option<std::time::Instant>,
    ) -> Result<(), McpError> {
        let ready = {
            let mut registry = self.registry.lock();
            let mut ready = Vec::new();
            let mut retained = Vec::new();
            for entry in registry.retired.drain(..) {
                if Arc::strong_count(&entry) == 1 && Arc::strong_count(&entry.connection) == 1 {
                    ready.push(entry);
                } else {
                    retained.push(entry);
                }
            }
            registry.retired = retained;
            ready
        };
        let mut failed = Vec::new();
        let mut first_error = None;
        for entry in ready {
            let config_generation = config_generation(&entry.config).unwrap_or([0; 32]);
            let runtime_generation = Some(entry.connection.generation());
            let result = match deadline {
                None => {
                    entry
                        .connection
                        .shutdown(entry.config.shutdown_timeout())
                        .await
                }
                Some(deadline) => {
                    let timeout = entry
                        .config
                        .shutdown_timeout()
                        .min(deadline.saturating_duration_since(std::time::Instant::now()));
                    tokio::time::timeout_at(deadline.into(), entry.connection.shutdown(timeout))
                        .await
                        .unwrap_or_else(|_| {
                            Err(McpError::CleanupUnconfirmed(
                                "the connection did not close before the shutdown deadline".into(),
                            ))
                        })
                }
            };
            if let Err(error) = result {
                log::warn!(
                    "MCP cleanup unconfirmed id={} transport={}: {error}",
                    entry.config.id.as_str(),
                    transport_name(&entry.config)
                );
                self.diagnostics.record(
                    entry.config.id.clone(),
                    McpDiagnosticStage::Cleanup,
                    McpDiagnosticOutcome::OutcomeUnknown,
                    "cleanup_unconfirmed",
                    error.to_string(),
                    transport_name(&entry.config),
                    config_generation,
                    runtime_generation,
                    None,
                    true,
                    "retry_cleanup",
                );
                if first_error.is_none() {
                    first_error = Some(error.clone());
                }
                failed.push((entry, bounded_error(error.to_string())));
            } else {
                self.diagnostics.record(
                    entry.config.id.clone(),
                    McpDiagnosticStage::Shutdown,
                    McpDiagnosticOutcome::Success,
                    "shutdown_confirmed",
                    "MCP transport and owned process shutdown was confirmed",
                    transport_name(&entry.config),
                    config_generation,
                    runtime_generation,
                    None,
                    false,
                    "none",
                );
            }
        }
        let changed_servers = {
            let mut registry = self.registry.lock();
            let mut changed_servers = HashSet::new();
            for (entry, error) in failed {
                if let Some(status) = registry.statuses.get_mut(&entry.config.id) {
                    if !status.cleanup_pending || status.last_error.as_deref() != Some(&error) {
                        changed_servers.insert(entry.config.id.clone());
                    }
                    status.cleanup_pending = true;
                    status.last_error = Some(error);
                }
                registry.retired.push(entry);
            }
            let pending_servers = registry
                .retired
                .iter()
                .map(|entry| entry.config.id.clone())
                .collect::<HashSet<_>>();
            for status in registry.statuses.values_mut() {
                let cleanup_pending = pending_servers.contains(&status.server_id);
                if status.cleanup_pending != cleanup_pending {
                    status.cleanup_pending = cleanup_pending;
                    changed_servers.insert(status.server_id.clone());
                }
            }
            changed_servers.into_iter().collect::<Vec<_>>()
        };
        for server_id in changed_servers {
            self.emit(server_id, McpEventKind::ServerStatus);
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }

    /// Closes every connection and confirms each owned process gone, by
    /// `deadline` in all: each connection's own `shutdown_timeout` still
    /// bounds it, and one not confirmed by the deadline stays retired with
    /// its cleanup obligation (`CleanupUnconfirmed`) for a later cleanup.
    pub async fn shutdown(&self, deadline: std::time::Instant) -> Result<(), McpError> {
        log::info!("MCP manager shutdown starting");
        self.lifetime.cancel();
        self.interactions.cancel_all();
        {
            let mut registry = self.registry.lock();
            let active = registry
                .active
                .drain()
                .map(|(_, entry)| entry)
                .collect::<Vec<_>>();
            registry.retired.extend(active);
        }
        self.cleanup_retired_until(Some(deadline)).await?;
        if !self.registry.lock().retired.is_empty() {
            log::warn!(
                "MCP manager shutdown unconfirmed: active plan or call leases still retain {} connection(s)",
                self.registry.lock().retired.len()
            );
            return Err(McpError::CleanupUnconfirmed(
                "active MCP plan or call leases still retain a connection".into(),
            ));
        }
        log::info!("MCP manager shutdown complete");
        Ok(())
    }

    pub(super) fn emit(&self, server_id: McpServerId, kind: McpEventKind) {
        self.events.emit(McpEventNotice { server_id, kind });
    }
}
