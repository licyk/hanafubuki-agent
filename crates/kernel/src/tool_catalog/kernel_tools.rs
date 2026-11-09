use super::*;

/// Plans the kernel's own tools, all `Exclusive`: each acts on the turn
/// itself or on the session's compression state.
pub(crate) struct CoreAgentToolContributor;

impl AgentToolContributor for CoreAgentToolContributor {
    fn id(&self) -> &str {
        "core"
    }

    fn contribute(
        &self,
        context: AgentToolCatalogContext,
        cancellation: CancellationToken,
    ) -> AgentToolContributionFuture<'_> {
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(AgentToolCatalogError::Cancelled);
            }
            core_tool_registrations(context.compression_tools)
                .map(AgentToolContribution::from_tools)
        })
    }
}

pub(super) fn core_tool_registrations(
    compression_tools: bool,
) -> Result<Vec<AgentPlannedTool>, AgentToolCatalogError> {
    let provider_id = AgentToolProviderId::parse("builtin".to_string())
        .map_err(|_| AgentToolCatalogError::InvalidToolName)?;
    let compression: &[crate::AgentCoreTool] = if compression_tools {
        &crate::AgentCoreTool::COMPRESSION
    } else {
        &[]
    };
    crate::AgentCoreTool::STANDARD
        .iter()
        .chain(compression)
        .copied()
        .map(|tool| {
            AgentPlannedTool::fixed(
                provider_id.clone(),
                tool.model_spec(),
                AgentToolExecutionClass::Exclusive,
            )
        })
        .collect()
}
