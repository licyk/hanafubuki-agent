use hanafubuki_agent_contracts::{ExtensionPoint, ToolAvailability, ToolPlanContext};

use super::*;

/// Plans the tools plugins registered, from one registry snapshot taken for
/// the sampling step.
pub(crate) struct RegisteredAgentToolContributor {
    tools: RegistrySnapshot<Arc<dyn AgentToolHandler>>,
}

impl RegisteredAgentToolContributor {
    pub(crate) fn new(tools: RegistrySnapshot<Arc<dyn AgentToolHandler>>) -> Self {
        Self { tools }
    }
}

impl AgentToolContributor for RegisteredAgentToolContributor {
    fn id(&self) -> &str {
        "registered"
    }

    fn contribute(
        &self,
        context: AgentToolCatalogContext,
        _cancellation: CancellationToken,
    ) -> AgentToolContributionFuture<'_> {
        Box::pin(async move {
            let provider_id = AgentToolProviderId::parse("builtin".to_string())
                .map_err(|_| AgentToolCatalogError::InvalidToolName)?;
            let plan = ToolPlanContext {
                image_input: context.input_capabilities.image_input,
                multimodal_tool_output: context
                    .provider_capabilities
                    .multimodal_tool_output_transport
                    .is_supported(),
                freeform_tools: context.provider_capabilities.freeform_tools,
            };
            let mut tools = Vec::new();
            for entry in self.tools.entries() {
                if entry.is_retired() {
                    continue;
                }
                // A panicking `availability` leaves the tool out of the step.
                let availability = entry
                    .guard(ExtensionPoint::ToolAvailability, |tool| {
                        tool.availability(&plan)
                    })
                    .unwrap_or(ToolAvailability::Omitted);
                if availability == ToolAvailability::Omitted {
                    continue;
                }
                tools.push(AgentPlannedTool::registered(
                    provider_id.clone(),
                    entry.clone(),
                    availability,
                )?);
            }
            Ok(AgentToolContribution::from_tools(tools))
        })
    }
}
