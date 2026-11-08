use super::*;

/// Plans the kernel's own tools, all `Exclusive`: each acts on the turn
/// itself.
pub(crate) struct CoreAgentToolContributor;

impl AgentToolContributor for CoreAgentToolContributor {
    fn id(&self) -> &str {
        "core"
    }

    fn contribute(
        &self,
        _context: AgentToolCatalogContext,
        cancellation: CancellationToken,
    ) -> AgentToolContributionFuture<'_> {
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(AgentToolCatalogError::Cancelled);
            }
            core_tool_registrations().map(AgentToolContribution::from_tools)
        })
    }
}

pub(super) fn core_tool_registrations() -> Result<Vec<AgentPlannedTool>, AgentToolCatalogError> {
    let provider_id = AgentToolProviderId::parse("builtin".to_string())
        .map_err(|_| AgentToolCatalogError::InvalidToolName)?;
    crate::AgentCoreTool::ALL
        .into_iter()
        .map(|tool| {
            AgentPlannedTool::fixed(
                provider_id.clone(),
                tool.model_spec(),
                AgentToolExecutionClass::Exclusive,
            )
        })
        .collect()
}
