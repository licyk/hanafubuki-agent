use super::*;

/// A started connection with its discovered catalog and resource counts,
/// before it replaces the active entry.
struct McpPreparedConnection {
    connection: Arc<McpLiveConnection>,
    catalog: McpToolCatalog,
    resources: McpResourceDiscovery,
}

impl McpManager {
    /// Connects `config`, or refreshes its connection. The configuration was
    /// validated where it entered: on save (`McpSettings::validate`) or on
    /// read (the settings repair drops a server that fails it). A connect
    /// from outside the manager starts the server afresh: its automatic
    /// reconnects are counted from zero again.
    pub async fn connect_or_refresh(
        &self,
        config: McpServerConfig,
        secrets: McpConnectionSecrets,
        cancellation: CancellationToken,
    ) -> Result<McpServerStatus, McpError> {
        self.registry.lock().reconnects.remove(&config.id);
        self.connect(config, secrets, cancellation).await
    }

    /// [`Self::connect_or_refresh`] without emptying the reconnect window,
    /// which an automatic reconnect counts in.
    pub(super) async fn connect(
        &self,
        config: McpServerConfig,
        secrets: McpConnectionSecrets,
        cancellation: CancellationToken,
    ) -> Result<McpServerStatus, McpError> {
        let requested_config_generation = config_generation(&config)?;
        let transport = transport_name(&config).to_string();
        let server_id = config.id.clone();
        log::debug!(
            "MCP server connect_or_refresh id={} transport={transport}",
            server_id.as_str()
        );
        let coalesced = {
            let mut registry = self.registry.lock();
            if let Some(operation) = registry.connect_operations.get(&config.id) {
                if operation.config_generation != requested_config_generation {
                    return Err(McpError::Unavailable(
                        "another operation is already active for this server".into(),
                    ));
                }
                Some(operation.clone())
            } else {
                if !registry.operations.insert(config.id.clone()) {
                    return Err(McpError::Unavailable(
                        "another operation is already active for this server".into(),
                    ));
                }
                registry.connect_operations.insert(
                    config.id.clone(),
                    Arc::new(McpConnectOperation {
                        config_generation: requested_config_generation,
                        completed: AtomicBool::new(false),
                        completion: Notify::new(),
                        result: Mutex::new(None),
                    }),
                );
                None
            }
        };
        if let Some(operation) = coalesced {
            while !operation.completed.load(Ordering::Acquire) {
                tokio::select! {
                    _ = operation.completion.notified() => {},
                    _ = crate::request::wait_cancelled(&cancellation) => {
                        return Err(McpError::Cancelled);
                    }
                }
            }
            return operation.result.lock().clone().ok_or_else(|| {
                McpError::Unavailable("coalesced MCP connection result is unavailable".into())
            })?;
        }
        self.diagnostics.begin_attempt(config.id.clone());
        {
            let mut registry = self.registry.lock();
            let existing = registry.active.get(&config.id).cloned();
            let phase = if existing.is_some() {
                McpConnectionPhase::Refreshing
            } else {
                McpConnectionPhase::Starting
            };
            let status = entry_status(&config.id, phase, existing.as_deref());
            registry.statuses.insert(config.id.clone(), status);
        }

        let candidate = match self
            .prepare_connection(&config, secrets.clone(), cancellation.clone())
            .await
        {
            // Shutdown overtook the connect: the manager may already have
            // closed every connection, so this one is disposed, not installed.
            Ok(prepared) if cancellation.is_cancelled() => self
                .dispose_failed_candidate(&config, prepared.connection)
                .await
                .and(Err(McpError::Cancelled)),
            candidate => candidate,
        };
        match candidate {
            Ok(prepared) => {
                self.record_connected(&config, &transport, requested_config_generation, &prepared);
                let McpPreparedConnection {
                    connection,
                    catalog,
                    resources,
                } = prepared;
                let (resource_count, resource_template_count) =
                    (resources.resource_count, resources.template_count);
                let tools = catalog.tools;
                let entry = Arc::new(McpConnectionEntry {
                    config: config.clone(),
                    secrets,
                    connection: connection.clone(),
                    tool_call_gates: tool_call_gates(&config, &tools),
                    tools,
                    resource_count,
                    resource_template_count,
                    call_gate: Arc::new(Semaphore::new(config.maximum_concurrent_calls)),
                });
                let status = entry_status(&config.id, McpConnectionPhase::Ready, Some(&entry));
                let completion = {
                    let mut registry = self.registry.lock();
                    if let Some(old) = registry.active.insert(config.id.clone(), entry) {
                        registry.retired.push(old);
                    }
                    registry.operations.remove(&config.id);
                    let completion = registry.connect_operations.remove(&config.id);
                    registry.statuses.insert(config.id, status.clone());
                    completion
                };
                self.watch_connection(status.server_id.clone(), &connection);
                self.emit(status.server_id.clone(), McpEventKind::ServerStatus);
                self.emit(status.server_id.clone(), McpEventKind::ToolCatalog);
                let cleanup = self.cleanup_retired().await;
                if let Some(operation) = completion {
                    *operation.result.lock() = Some(cleanup.clone().map(|_| status.clone()));
                    operation.completed.store(true, Ordering::Release);
                    operation.completion.notify_waiters();
                }
                cleanup?;
                log::info!(
                    "MCP server ready id={} transport={transport} tools={} generation={}",
                    server_id.as_str(),
                    status.tool_count,
                    status
                        .connection_generation
                        .map_or_else(|| "none".to_string(), |g| format!("{g:?}"))
                );
                Ok(status)
            }
            Err(error) => {
                let (stage, outcome, code) = diagnostic_error(&error);
                self.diagnostics.record(
                    config.id.clone(),
                    stage,
                    outcome,
                    code,
                    error.to_string(),
                    transport.clone(),
                    requested_config_generation,
                    None,
                    None,
                    !matches!(error, McpError::InvalidConfig(_)),
                    "inspect_diagnostics",
                );
                let mut registry = self.registry.lock();
                registry.operations.remove(&config.id);
                let completion = registry.connect_operations.remove(&config.id);
                let retained = registry.active.get(&config.id).cloned();
                let phase = if retained.is_some() {
                    McpConnectionPhase::Ready
                } else {
                    McpConnectionPhase::Failed
                };
                let mut status = entry_status(&config.id, phase, retained.as_deref());
                status.cleanup_pending = registry
                    .retired
                    .iter()
                    .any(|entry| entry.config.id == config.id);
                status.last_error = Some(bounded_error(error.to_string()));
                registry.statuses.insert(config.id.clone(), status);
                drop(registry);
                if let Some(operation) = completion {
                    *operation.result.lock() = Some(Err(error.clone()));
                    operation.completed.store(true, Ordering::Release);
                    operation.completion.notify_waiters();
                }
                self.emit(config.id.clone(), McpEventKind::ServerStatus);
                log::warn!(
                    "MCP server connect_or_refresh failed id={} transport={transport}: {error}",
                    server_id.as_str()
                );
                Err(error)
            }
        }
    }

    async fn prepare_connection(
        &self,
        config: &McpServerConfig,
        secrets: McpConnectionSecrets,
        cancellation: CancellationToken,
    ) -> Result<McpPreparedConnection, McpError> {
        match &config.transport {
            McpTransportConfig::Stdio(_) => {
                let connection = McpStdioConnection::connect(
                    config,
                    secrets.stdio_environment,
                    secrets.credential_reference_generation,
                    self.executor.clone(),
                    self.workers.clone(),
                    self.interactions.clone(),
                    cancellation,
                )
                .await?;
                let connection = Arc::new(McpLiveConnection::Stdio(connection));
                match connection.discover_tools(config).await {
                    Ok(catalog) => {
                        let resources =
                            discover_resources(&connection, config.list_timeout()).await;
                        Ok(McpPreparedConnection {
                            connection,
                            catalog,
                            resources,
                        })
                    }
                    Err(error) => {
                        self.dispose_failed_candidate(config, connection).await?;
                        Err(error)
                    }
                }
            }
            McpTransportConfig::StreamableHttp(_) => {
                let connection = McpStreamableHttpConnection::connect(
                    config,
                    secrets.http_headers,
                    secrets.credential_reference_generation,
                    self.interactions.clone(),
                    self.http_client(),
                    cancellation,
                )
                .await?;
                let connection = Arc::new(McpLiveConnection::StreamableHttp(connection));
                match connection.discover_tools(config).await {
                    Ok(catalog) => {
                        let resources =
                            discover_resources(&connection, config.list_timeout()).await;
                        Ok(McpPreparedConnection {
                            connection,
                            catalog,
                            resources,
                        })
                    }
                    Err(error) => {
                        self.dispose_failed_candidate(config, connection).await?;
                        Err(error)
                    }
                }
            }
        }
    }

    /// Records the success path of a connection: transport, initialize, the
    /// tool catalog (with what it skipped or adjusted) and resources.
    fn record_connected(
        &self,
        config: &McpServerConfig,
        transport: &str,
        config_generation: [u8; 32],
        prepared: &McpPreparedConnection,
    ) {
        let connection = &prepared.connection;
        let runtime_generation = Some(connection.generation());
        let record = |stage, outcome, code: &str, message: String, retryable, next: &str| {
            self.diagnostics.record(
                config.id.clone(),
                stage,
                outcome,
                code,
                message,
                transport,
                config_generation,
                runtime_generation,
                None,
                retryable,
                next,
            )
        };
        if connection.resolved_program().is_some() {
            record(
                McpDiagnosticStage::ResolveProgram,
                McpDiagnosticOutcome::Success,
                "resolve_program_completed",
                "stdio program resolved to an executable path".into(),
                false,
                "spawn",
            );
        }
        record(
            McpDiagnosticStage::Spawn,
            McpDiagnosticOutcome::Success,
            "transport_started",
            "MCP transport was established".into(),
            false,
            "initialize",
        );
        record(
            McpDiagnosticStage::Initialize,
            McpDiagnosticOutcome::Success,
            "initialize_completed",
            "MCP initialize and capability negotiation completed".into(),
            false,
            "discover_tools",
        );
        let catalog_issues = self.record_catalog_issues(
            config,
            McpDiagnosticStage::DiscoverTools,
            runtime_generation,
            &prepared.catalog.issues,
        );
        let tools = prepared.catalog.tools.len();
        match catalog_issues {
            None => record(
                McpDiagnosticStage::DiscoverTools,
                McpDiagnosticOutcome::Success,
                "tool_discovery_completed",
                format!("discovered {tools} MCP tools"),
                false,
                "none",
            ),
            Some(summary) => record(
                McpDiagnosticStage::DiscoverTools,
                McpDiagnosticOutcome::Warning,
                "tool_discovery_partial",
                format!("discovered {tools} MCP tools; {summary}"),
                false,
                "inspect_diagnostics",
            ),
        }
        let resources = &prepared.resources;
        let scope = McpResourceDiagnosticScope {
            diagnostics: &self.diagnostics,
            server_id: &config.id,
            transport,
            config_generation,
            runtime_generation,
        };
        scope.record_undecodable(
            McpResourceEntryKind::Resource,
            &resources.undecodable_resources,
        );
        scope.record_undecodable(
            McpResourceEntryKind::Template,
            &resources.undecodable_templates,
        );
        match resources.partial_summary() {
            None => record(
                McpDiagnosticStage::DiscoverResources,
                McpDiagnosticOutcome::Success,
                "resource_discovery_completed",
                format!(
                    "discovered {} resources and {} templates",
                    resources.resource_count, resources.template_count
                ),
                false,
                "none",
            ),
            Some(summary) => record(
                McpDiagnosticStage::DiscoverResources,
                McpDiagnosticOutcome::Warning,
                "resource_discovery_partial",
                summary,
                resources.failure.is_some(),
                "inspect_diagnostics",
            ),
        }
        if let Some(stderr) = connection.diagnostic_tail() {
            self.diagnostics.set_stderr_tail(config.id.clone(), &stderr);
        }
    }

    async fn dispose_failed_candidate(
        &self,
        config: &McpServerConfig,
        connection: Arc<McpLiveConnection>,
    ) -> Result<(), McpError> {
        if let Err(error) = connection.shutdown(config.shutdown_timeout()).await {
            // Retired for its cleanup only; it is never reconnected.
            let retained = Arc::new(McpConnectionEntry {
                config: config.clone(),
                secrets: McpConnectionSecrets::empty(0),
                connection,
                tools: Vec::new(),
                resource_count: 0,
                resource_template_count: 0,
                call_gate: Arc::new(Semaphore::new(config.maximum_concurrent_calls)),
                tool_call_gates: HashMap::new(),
            });
            self.registry.lock().retired.push(retained);
            return Err(McpError::CleanupUnconfirmed(bounded_error(
                error.to_string(),
            )));
        }
        Ok(())
    }
}
