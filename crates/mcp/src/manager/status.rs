use super::*;

impl McpManager {
    pub fn statuses(&self) -> Vec<McpServerStatus> {
        let mut statuses = self
            .registry
            .lock()
            .statuses
            .values()
            .cloned()
            .collect::<Vec<_>>();
        statuses.sort_by(|left, right| left.server_id.as_str().cmp(right.server_id.as_str()));
        statuses
    }

    pub fn diagnostics(&self, server_id: &McpServerId) -> crate::McpDiagnosticSnapshot {
        self.diagnostics.snapshot(server_id)
    }

    /// The server's current diagnostics by severity.
    pub fn diagnostic_summary(&self, server_id: &McpServerId) -> crate::McpDiagnosticSummary {
        self.diagnostics.summary(server_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_diagnostic(
        &self,
        config: &McpServerConfig,
        stage: McpDiagnosticStage,
        outcome: McpDiagnosticOutcome,
        code: &'static str,
        message: &'static str,
        retryable: bool,
        next_step: &'static str,
    ) {
        let runtime_generation = self
            .registry
            .lock()
            .active
            .get(&config.id)
            .map(|entry| entry.connection.generation());
        self.diagnostics.record(
            config.id.clone(),
            stage,
            outcome,
            code,
            message,
            transport_name(config),
            config_generation(config).unwrap_or([0; 32]),
            runtime_generation,
            None,
            retryable,
            next_step,
        );
    }

    pub fn report_connection_failure(&self, server_id: McpServerId, error: impl Into<String>) {
        let mut registry = self.registry.lock();
        let retained = registry.active.get(&server_id).cloned();
        let cleanup_pending = registry
            .retired
            .iter()
            .any(|entry| entry.config.id == server_id);
        let phase = if retained.is_some() {
            McpConnectionPhase::Ready
        } else {
            McpConnectionPhase::Failed
        };
        let mut status = entry_status(&server_id, phase, retained.as_deref());
        status.cleanup_pending = cleanup_pending;
        status.last_error = Some(bounded_error(error.into()));
        registry.statuses.insert(server_id.clone(), status);
        drop(registry);
        self.emit(server_id, McpEventKind::ServerStatus);
    }

    pub fn pending_elicitations(&self) -> Vec<McpElicitationRequest> {
        self.interactions.pending()
    }

    pub fn resolve_elicitation(
        &self,
        request_id: McpElicitationRequestId,
        decision: McpElicitationDecision,
    ) -> Result<(), McpError> {
        self.interactions.resolve(request_id, decision)
    }
}

/// The status of `server_id` in `phase`, describing `entry`'s connection
/// and catalog when there is one; no cleanup pending, error or reconnect.
pub(super) fn entry_status(
    server_id: &McpServerId,
    phase: McpConnectionPhase,
    entry: Option<&McpConnectionEntry>,
) -> McpServerStatus {
    McpServerStatus {
        server_id: server_id.clone(),
        phase,
        connection_generation: entry.map(|entry| entry.connection.generation()),
        catalog_generation: entry
            .and_then(|entry| entry.tools.first().map(|tool| tool.catalog_generation)),
        tool_count: entry.map_or(0, |entry| entry.tools.len()),
        resource_count: entry.map_or(0, |entry| entry.resource_count),
        resource_template_count: entry.map_or(0, |entry| entry.resource_template_count),
        resolved_program: entry.and_then(|entry| entry.connection.resolved_program()),
        capabilities: entry.map(|entry| entry.connection.capabilities()),
        cleanup_pending: false,
        last_error: None,
        reconnect: None,
    }
}
