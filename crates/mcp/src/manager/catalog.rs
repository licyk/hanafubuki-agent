use super::*;

impl McpManager {
    pub fn tool_leases(&self) -> Vec<McpToolLease> {
        self.step_binding().tool_leases().to_vec()
    }

    pub async fn refresh_changed_catalogs(
        &self,
        cancellation: CancellationToken,
    ) -> Result<(), McpError> {
        let candidates = {
            let mut registry = self.registry.lock();
            let candidates = registry
                .active
                .iter()
                .filter(|(_, entry)| {
                    entry.connection.tools_changed()
                        && Arc::strong_count(&entry.connection) == 1
                        && !registry.operations.contains(&entry.config.id)
                })
                .map(|(server_id, entry)| (server_id.clone(), entry.clone()))
                .collect::<Vec<_>>();
            for (server_id, _) in &candidates {
                registry.operations.insert(server_id.clone());
            }
            candidates
        };
        for (server_id, entry) in candidates {
            let config_generation = config_generation(&entry.config).unwrap_or([0; 32]);
            let runtime_generation = Some(entry.connection.generation());
            let transport = transport_name(&entry.config);
            self.diagnostics.record(
                server_id.clone(),
                McpDiagnosticStage::ToolListChanged,
                McpDiagnosticOutcome::Success,
                "tool_list_changed_received",
                "MCP server requested tool catalog refresh",
                transport,
                config_generation,
                runtime_generation,
                None,
                false,
                "refresh",
            );
            if cancellation.is_cancelled() {
                self.registry.lock().operations.remove(&server_id);
                return Err(McpError::Cancelled);
            }
            let discovered = entry.connection.discover_tools(&entry.config).await;
            let mut registry = self.registry.lock();
            registry.operations.remove(&server_id);
            match discovered {
                Ok(catalog) => {
                    let tools = catalog.tools;
                    let still_current = registry
                        .active
                        .get(&server_id)
                        .is_some_and(|current| Arc::ptr_eq(&current.connection, &entry.connection));
                    if still_current {
                        let tool_call_gates = tool_call_gates(&entry.config, &tools);
                        let replacement = Arc::new(McpConnectionEntry {
                            config: entry.config.clone(),
                            secrets: entry.secrets.clone(),
                            connection: entry.connection.clone(),
                            tools,
                            resource_count: entry.resource_count,
                            resource_template_count: entry.resource_template_count,
                            call_gate: entry.call_gate.clone(),
                            tool_call_gates,
                        });
                        let status =
                            entry_status(&server_id, McpConnectionPhase::Ready, Some(&replacement));
                        registry.active.insert(server_id.clone(), replacement);
                        registry.statuses.insert(server_id, status);
                        entry.connection.clear_tools_changed();
                        let issues = self.record_catalog_issues(
                            &entry.config,
                            McpDiagnosticStage::Refresh,
                            runtime_generation,
                            &catalog.issues,
                        );
                        self.diagnostics.record(
                            entry.config.id.clone(),
                            McpDiagnosticStage::Refresh,
                            if issues.is_some() {
                                McpDiagnosticOutcome::Warning
                            } else {
                                McpDiagnosticOutcome::Success
                            },
                            "catalog_refresh_completed",
                            issues.map_or_else(
                                || "MCP tool catalog was atomically refreshed".to_string(),
                                |summary| {
                                    format!("MCP tool catalog was atomically refreshed; {summary}")
                                },
                            ),
                            transport,
                            config_generation,
                            runtime_generation,
                            None,
                            false,
                            "next_sampling_step",
                        );
                        self.emit(entry.config.id.clone(), McpEventKind::ToolCatalog);
                    }
                }
                Err(error) => {
                    self.diagnostics.record(
                        server_id.clone(),
                        McpDiagnosticStage::Refresh,
                        McpDiagnosticOutcome::Error,
                        "catalog_refresh_failed",
                        error.to_string(),
                        transport,
                        config_generation,
                        runtime_generation,
                        None,
                        true,
                        "inspect_diagnostics",
                    );
                    if let Some(status) = registry.statuses.get_mut(&server_id) {
                        status.last_error = Some(bounded_error(error.to_string()));
                    }
                    self.emit(server_id.clone(), McpEventKind::ServerStatus);
                }
            }
        }
        Ok(())
    }

    /// Records each tool a catalog skipped or adjusted as a Warning naming
    /// only the tool, up to [`MAX_RECORDED_CATALOG_ISSUES`] so a bad listing
    /// cannot evict the server's other diagnostics, and returns a summary
    /// for the caller's own entry when there was anything to record.
    pub(super) fn record_catalog_issues(
        &self,
        config: &McpServerConfig,
        stage: McpDiagnosticStage,
        runtime_generation: Option<McpConnectionGeneration>,
        issues: &[McpCatalogIssue],
    ) -> Option<String> {
        if issues.is_empty() {
            return None;
        }
        let config_generation = config_generation(config).unwrap_or([0; 32]);
        for issue in issues.iter().take(MAX_RECORDED_CATALOG_ISSUES) {
            log::warn!(
                "MCP tool catalog issue id={} tool={} kind={}",
                config.id.as_str(),
                crate::sanitize_mcp_diagnostic(
                    &issue.remote_name,
                    MAX_MCP_CATALOG_ISSUE_NAME_BYTES
                ),
                issue.kind.code()
            );
            self.diagnostics.record(
                config.id.clone(),
                stage,
                McpDiagnosticOutcome::Warning,
                issue.kind.code(),
                issue.kind.message(),
                transport_name(config),
                config_generation,
                runtime_generation,
                Some(issue.remote_name.clone()),
                false,
                "none",
            );
        }
        let skipped = issues
            .iter()
            .filter(|issue| issue.kind.skips_tool())
            .count();
        let adjusted = issues.len() - skipped;
        let mut summary = format!("skipped {skipped} and adjusted {adjusted} MCP tools");
        if issues.len() > MAX_RECORDED_CATALOG_ISSUES {
            summary.push_str(&format!(
                " (the first {MAX_RECORDED_CATALOG_ISSUES} are listed)"
            ));
        }
        log::warn!("MCP tool catalog id={}: {summary}", config.id.as_str());
        Some(summary)
    }
}

/// Per-tool catalog issues recorded per listing; the summary counts all.
pub(super) const MAX_RECORDED_CATALOG_ISSUES: usize = 16;

pub(super) fn tool_call_gates(
    config: &McpServerConfig,
    tools: &[McpToolBinding],
) -> HashMap<String, Arc<Semaphore>> {
    tools
        .iter()
        .filter_map(|binding| {
            config
                .tool_maximum_concurrent_calls
                .get(&binding.remote_name)
                .copied()
                .map(|limit| (binding.remote_name.clone(), Arc::new(Semaphore::new(limit))))
        })
        .collect()
}
