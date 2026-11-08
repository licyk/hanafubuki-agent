use super::*;

#[derive(Clone)]
pub struct McpToolLease {
    pub binding: McpToolBinding,
    pub approval_policy: McpApprovalPolicy,
    pub(super) timeout: Duration,
    pub(super) connection: Arc<McpLiveConnection>,
    pub(super) call_gate: Arc<Semaphore>,
    pub(super) interactions: McpInteractionHub,
    pub(super) diagnostics: McpDiagnosticStore,
    pub(super) config_generation: [u8; 32],
    pub(super) transport: String,
}

impl McpToolLease {
    /// The server's call timeout. It counts active time only: it pauses
    /// while an elicitation waits for the user.
    pub fn call_timeout(&self) -> Duration {
        self.timeout
    }

    /// Whether the connection the lease pins has closed, so a call returns
    /// [`McpError::CatalogChanged`].
    pub fn is_stale(&self) -> bool {
        self.connection.is_closed()
    }

    pub async fn call(
        &self,
        arguments: serde_json::Map<String, serde_json::Value>,
        context: crate::McpCallContext,
        cancellation: CancellationToken,
    ) -> Result<ToolOutput, McpError> {
        // The lease pins the connection the step's plan was built from; once
        // it closed, the server's current tools are another catalog.
        if self.is_stale() {
            return Err(McpError::CatalogChanged);
        }
        let gate = self.call_gate.clone();
        let permit = tokio::select! {
            permit = tokio::time::timeout(self.timeout, gate.acquire_owned()) => {
                permit
                    .map_err(|_| McpError::Timeout)?
                    .map_err(|_| McpError::Unavailable("MCP server call gate is closed".into()))?
            }
            _ = wait_cancelled(&cancellation) => return Err(McpError::Cancelled),
        };
        let _permit = permit;
        let active_call = self.interactions.enter_call(
            self.binding.server_id.clone(),
            self.binding.connection_generation,
            context.clone(),
        );
        let pause_state = active_call.pause_state();
        #[cfg(any(test, feature = "test-support"))]
        crate::test_support::panic_if_armed(&self.binding);
        let result = self
            .connection
            .call_tool(
                &self.binding,
                arguments,
                self.timeout,
                pause_state,
                cancellation,
            )
            .await;
        let (outcome, code, retryable) = match &result {
            Ok(_) => (McpDiagnosticOutcome::Success, "tool_call_completed", false),
            Err(McpError::Cancelled) => {
                (McpDiagnosticOutcome::Cancelled, "tool_call_cancelled", true)
            }
            Err(McpError::CleanupUnconfirmed(_)) => (
                McpDiagnosticOutcome::OutcomeUnknown,
                "tool_call_cleanup_unconfirmed",
                true,
            ),
            Err(_) => (McpDiagnosticOutcome::Error, "tool_call_failed", true),
        };
        self.diagnostics.record(
            self.binding.server_id.clone(),
            McpDiagnosticStage::ToolCall,
            outcome,
            code,
            result
                .as_ref()
                .err()
                .map(ToString::to_string)
                .unwrap_or_else(|| "MCP tool call completed".to_string()),
            self.transport.clone(),
            self.config_generation,
            Some(self.binding.connection_generation),
            Some(self.binding.remote_name.clone()),
            retryable,
            if retryable {
                "inspect_diagnostics"
            } else {
                "none"
            },
        );
        if matches!(result, Err(McpError::Cancelled | McpError::Timeout)) {
            self.interactions.cancel_call(
                &self.binding.server_id,
                self.binding.connection_generation,
                &context,
            );
        }
        result
    }
}
