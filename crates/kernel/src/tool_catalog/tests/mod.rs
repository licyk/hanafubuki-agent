use hanafubuki_agent_approval::{ApprovalAction, AuthorizedAction, PermissionRequirement};
use hanafubuki_agent_contracts::test_support::detached_entry;
use hanafubuki_agent_contracts::{
    AgentToolError, ToolAvailability, ToolDescriptor, ToolEffect, ToolFuture, ToolInvocation,
    ToolResult, ToolSourceContribution, ToolSourceStep,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_model_api::ModelToolSpec;
use hanafubuki_agent_tool_contracts::ToolProviderCapabilities;
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::Arc;

use crate::AgentSessionMode;

use super::*;

fn spec(name: &str) -> ModelToolSpec {
    ModelToolSpec {
        strict: true,
        name: name.to_string(),
        description: format!("{name} description"),
        input_schema: json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        }),
    }
}

/// Plans pre-built tools, as a tool source's would be.
struct DynamicContributor {
    registrations: Vec<AgentPlannedTool>,
}

impl AgentToolContributor for DynamicContributor {
    fn id(&self) -> &str {
        "dynamic-test"
    }

    fn contribute(
        &self,
        _context: AgentToolCatalogContext,
        _cancellation: CancellationToken,
    ) -> AgentToolContributionFuture<'_> {
        let registrations = self.registrations.clone();
        Box::pin(async move { Ok(AgentToolContribution::from_tools(registrations)) })
    }
}

/// A sourced tool's handler; planning never runs it.
struct TestHandler {
    descriptor: ToolDescriptor,
}

impl AgentToolHandler for TestHandler {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn permission<'a>(
        &'a self,
        _invocation: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
        Box::pin(async { Ok(PermissionRequirement::NotRequired) })
    }

    fn invoke(
        &self,
        _invocation: ToolInvocation,
        _approval: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>> {
        Box::pin(async { Err(AgentToolError::Failed("test handler".to_string())) })
    }
}

/// The source a test's sourced tools belong to; planning never asks it.
struct TestSource;

impl AgentToolSource for TestSource {
    fn id(&self) -> &str {
        "dynamic-test"
    }

    fn contribute<'a>(
        &'a self,
        _step: &'a ToolSourceStep,
    ) -> ToolFuture<'a, Result<ToolSourceContribution, AgentToolError>> {
        Box::pin(async { Ok(ToolSourceContribution::default()) })
    }
}

/// An optional sourced tool: a read-only `ConcurrentRead` tool unless the
/// test changes the descriptor.
fn sourced_tool(
    provider: &str,
    tool_id: &str,
    generation: [u8; 32],
    descriptor: ToolDescriptor,
) -> SourcedTool {
    SourcedTool {
        handler: Arc::new(TestHandler { descriptor }),
        provider_id: AgentToolProviderId::parse(provider).unwrap(),
        tool_id: AgentToolId::parse(tool_id).unwrap(),
        generation: AgentToolGeneration(generation),
        namespace_description: None,
        subject: None,
        trust: ToolSourceTrust::Optional,
    }
}

fn descriptor(name: ToolName, description: &str, exposure: ToolExposure) -> ToolDescriptor {
    ToolDescriptor {
        name,
        description: description.to_string(),
        input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
        exposure,
        execution_class: AgentToolExecutionClass::ConcurrentRead,
        effect: ToolEffect::ReadOnly,
        action: AgentActivityAction::USE_TOOL,
        memory_effect: AgentMemoryContextEffect::None,
        timeout: None,
    }
}

fn plan_sourced(tool: SourcedTool, availability: ToolAvailability) -> AgentPlannedTool {
    let source: Arc<dyn AgentToolSource> = Arc::new(TestSource);
    AgentPlannedTool::sourced(detached_entry("test-source", source), tool, availability).unwrap()
}

fn dynamic_tool(name: &str, exposure: ToolExposure) -> SourcedTool {
    sourced_tool(
        "dynamic",
        &format!("remote-{name}"),
        [name.len() as u8; 32],
        descriptor(
            ToolName::parse(name).unwrap(),
            &format!("{name} description"),
            exposure,
        ),
    )
}

fn dynamic_registration(name: &str, exposure: ToolExposure) -> AgentPlannedTool {
    plan_sourced(dynamic_tool(name, exposure), ToolAvailability::Function)
}

fn plan_context(mode: AgentSessionMode, strict_json_schema: bool) -> AgentToolCatalogContext {
    AgentToolCatalogContext {
        session_id: serde_json::from_value(json!(1)).unwrap(),
        turn_id: serde_json::from_value(json!(1)).unwrap(),
        generation: serde_json::from_value(json!(1)).unwrap(),
        session_revision: serde_json::from_value(json!(1)).unwrap(),
        sampling_step: 1,
        compression_tools: false,
        mode,
        input_capabilities: hanafubuki_agent_model_api::ModelInputCapabilities::default(),
        provider_capabilities: ToolProviderCapabilities {
            function_tools: true,
            strict_json_schema,
            maximum_tool_name_bytes: 64,
            namespace_tools: false,
            native_tool_search: false,
            freeform_tools: false,
            hosted_web_search: false,
            parallel_tool_calls: true,
            multimodal_tool_output_transport:
                hanafubuki_agent_tool_contracts::MultimodalToolOutputTransport::Unsupported,
            required_named_tool_choice: true,
        },
        selected_deferred_tools: BTreeSet::new(),
        memory_contribution_enabled: false,
    }
}

mod dynamic;
mod fixed;
mod provider_table;
mod registered;
mod search;
mod sources;
