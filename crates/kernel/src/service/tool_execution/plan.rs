use hanafubuki_agent_contracts::RegistryService;

use super::*;
use crate::tool_catalog::{
    AgentToolCatalogError, AgentToolPlanner, CoreAgentToolContributor,
    RegisteredAgentToolContributor,
};

impl AgentService {
    /// Plans one sampling step's tools: the kernel's own tools, the tools
    /// plugins registered and the tool sources' contributions, each from one
    /// snapshot of its registry taken now.
    pub(in crate::service) async fn build_tool_plan(
        &self,
        context: AgentToolCatalogContext,
        cancellation: CancellationToken,
    ) -> Result<AgentToolPlan, AgentToolCatalogError> {
        let registries = &self.inner.registries;
        AgentToolPlanner::new(vec![
            Arc::new(CoreAgentToolContributor),
            Arc::new(RegisteredAgentToolContributor::new(
                registries.tools.registry().snapshot(),
            )),
        ])
        .with_sources(registries.tool_sources.registry().snapshot())
        .with_search_cache(self.inner.tool_search_cache.clone())
        .with_spec_cache(self.inner.tool_spec_cache.clone())
        .build(context, cancellation)
        .await
        .map(|plan| plan.with_approval_rules(registries.approval_rules.snapshot()))
    }
}
