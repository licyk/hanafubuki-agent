use hanafubuki_agent_contracts::{
    AgentToolError, ExtensionPoint, ToolAvailability, ToolPlanContext, ToolSourceStep,
};

use super::contributor::push_optional_omission;
use super::*;

/// Plans one registered tool source's contribution for a sampling step: asks
/// the source under a lease on its entry, then plans each of its tools as a
/// handler bound to that entry. A tool that cannot be planned is omitted when
/// it is optional and fails the source when it is required.
///
/// A source whose plugin stops while it is asked (it was turned off, or the
/// host is shutting down) has left the configuration: whatever it answers, it
/// contributes no tools and the step records an omission. `Required` only
/// makes a running source's failure fail the plan.
///
/// A source that has not answered within `deadline` is cancelled and fails
/// like a source that returned an error.
pub(super) struct SourcedAgentToolContributor {
    entry: Arc<RegistryEntry<Arc<dyn AgentToolSource>>>,
    deadline: Duration,
}

impl SourcedAgentToolContributor {
    pub(super) fn new(
        entry: Arc<RegistryEntry<Arc<dyn AgentToolSource>>>,
        deadline: Duration,
    ) -> Self {
        Self { entry, deadline }
    }
}

impl AgentToolContributor for SourcedAgentToolContributor {
    fn id(&self) -> &str {
        self.entry.item().id()
    }

    fn trust(&self) -> AgentToolTrust {
        match self.entry.item().trust() {
            ToolSourceTrust::Required => AgentToolTrust::RequiredExternal,
            ToolSourceTrust::Optional => AgentToolTrust::OptionalExternal,
        }
    }

    fn contribute(
        &self,
        context: AgentToolCatalogContext,
        cancellation: CancellationToken,
    ) -> AgentToolContributionFuture<'_> {
        Box::pin(async move {
            let call = cancellation.child();
            let Some(_lease) = self.entry.acquire(&call) else {
                // The source's plugin is stopping; it contributes nothing.
                return Ok(AgentToolContribution::default());
            };
            let step = ToolSourceStep {
                session_id: context.session_id,
                turn_id: context.turn_id,
                generation: context.generation,
                session_revision: context.session_revision,
                memory_contribution: context.memory_contribution_enabled,
                cancellation: call.clone(),
            };
            let source = self.entry.item();
            // A panicking or overrunning source fails like a source that
            // returned an error: its trust decides whether the step goes on
            // without it.
            let answer = tokio::time::timeout(
                self.deadline,
                self.entry
                    .guard_async(ExtensionPoint::ToolSource, |source| {
                        source.contribute(&step)
                    }),
            )
            .await;
            let answer = match answer {
                Ok(answer) => answer.unwrap_or_else(|_| {
                    Err(AgentToolError::Failed(
                        "the source failed with an internal error".to_string(),
                    ))
                }),
                Err(_) => {
                    call.cancel();
                    Err(AgentToolError::Failed(format!(
                        "the source did not answer within {} s and was cancelled",
                        self.deadline.as_secs_f64()
                    )))
                }
            };
            if matches!(answer, Err(AgentToolError::Cancelled)) && cancellation.is_cancelled() {
                return Err(AgentToolCatalogError::Cancelled);
            }
            if self.entry.is_retired() {
                let mut stopped = AgentToolContribution::default();
                push_optional_omission(
                    &mut stopped.omissions,
                    source.id(),
                    None,
                    "the source stopped while the step was planned".to_string(),
                );
                return Ok(stopped);
            }
            let contribution = match answer {
                Ok(contribution) => contribution,
                Err(AgentToolError::Cancelled) => {
                    return Err(AgentToolCatalogError::ProviderFailed(
                        "the source cancelled its contribution".to_string(),
                    ));
                }
                Err(AgentToolError::Failed(message)) => {
                    return Err(AgentToolCatalogError::ProviderFailed(message));
                }
            };
            let plan = ToolPlanContext {
                image_input: context.input_capabilities.image_input,
                multimodal_tool_output: context
                    .provider_capabilities
                    .multimodal_tool_output_transport
                    .is_supported(),
                freeform_tools: context.provider_capabilities.freeform_tools,
            };
            let mut planned = AgentToolContribution {
                tools: Vec::with_capacity(contribution.tools.len()),
                omissions: contribution.omissions,
                lease: contribution.lease,
            };
            for tool in contribution.tools {
                let availability = self
                    .entry
                    .guard(ExtensionPoint::ToolAvailability, |_| {
                        tool.handler.availability(&plan)
                    })
                    .unwrap_or(ToolAvailability::Omitted);
                if availability == ToolAvailability::Omitted {
                    continue;
                }
                let trust = tool.trust;
                let name = tool.handler.descriptor().name.flattened_alias();
                match AgentPlannedTool::sourced(self.entry.clone(), tool, availability) {
                    Ok(tool) => planned.tools.push(tool),
                    Err(error) if trust == ToolSourceTrust::Optional => {
                        push_optional_omission(
                            &mut planned.omissions,
                            source.id(),
                            Some(name),
                            error.to_string(),
                        );
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(planned)
        })
    }
}
