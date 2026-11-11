//! A guest's tools: a tool source (`Optional`, as an MCP server's) that
//! plans the tools its manifest declares while the guest runs. Each is named
//! `<plugin>__<tool>` (a namespace of the plugin's name), keeps its identity
//! while its declaration does not change, asks for the user's approval like
//! an MCP tool's, and runs as the guest's `tool` call within the tool's
//! timeout. The kernel validates each schema once per identity and leaves a
//! tool it cannot plan out, and renames one whose name another tool has.

use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalActionKind, ApprovalOrigin, ApprovalRiskClass, AuthorizedAction,
    PermissionRequirement,
};
use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentMemoryContextEffect, AgentToolError, AgentToolHandler,
    AgentToolSource, SourcedTool, ToolDescriptor, ToolEffect, ToolFuture, ToolInvocation,
    ToolResult, ToolSourceContribution, ToolSourceRegistry, ToolSourceStep, ToolSourceTrust,
};
use hanafubuki_agent_packages::ContributedTool;
use hanafubuki_agent_tool_contracts::{
    canonical_json, ToolExecutionClass, ToolExposure, ToolGeneration, ToolId, ToolName, ToolOutput,
    ToolOutputContent, ToolProviderId,
};
use kardia_core::Context;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{provider_id, register_in, GuestCaller, GuestMethod};
use crate::package::GuestPackage;

/// What the kernel's bound on a call adds to the guest's own, so the
/// guest's deadline, which kills a hung guest, is the one that fires.
const KERNEL_MARGIN: Duration = Duration::from_secs(2);

/// `entry` is the guest's entry name, which a tool's identity carries.
pub(super) fn register(
    ctx: &Context,
    guest: &GuestCaller,
    package: &GuestPackage,
    entry: &str,
    issues: &mut Vec<String>,
) {
    if package.contributes.tools.is_empty() {
        return;
    }
    let provider = provider_id(&package.id);
    let Ok(provider_id) = ToolProviderId::parse(provider.clone()) else {
        issues.push(format!(
            "its tools were not registered: `{provider}` is not a tool provider id"
        ));
        return;
    };
    let namespace = namespace(&package.id.plugin_name);
    let tools = package
        .contributes
        .tools
        .iter()
        .filter_map(|tool| {
            let planned = plan(tool, &namespace, &provider_id, package, entry, guest);
            if planned.is_none() {
                issues.push(format!(
                    "the tool `{}` was left out: its name is not a tool name",
                    tool.name
                ));
            }
            planned
        })
        .collect::<Vec<_>>();
    let source = GuestToolSource {
        id: format!("plugin:{}", package.id.key()),
        guest: guest.clone(),
        tools,
    };
    register_in::<ToolSourceRegistry>(ctx, Arc::new(source), "its tool source", issues);
}

/// The model's name space for a plugin: its name, with what a tool name
/// cannot hold replaced by `_`.
fn namespace(plugin: &str) -> String {
    plugin
        .chars()
        .map(|character| match character {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '-' => character,
            _ => '_',
        })
        .collect()
}

fn plan(
    tool: &ContributedTool,
    namespace: &str,
    provider_id: &ToolProviderId,
    package: &GuestPackage,
    entry: &str,
    guest: &GuestCaller,
) -> Option<SourcedTool> {
    let name = ToolName::namespaced(vec![namespace.to_string()], tool.name.clone()).ok()?;
    let tool_id = ToolId::parse(tool.name.clone()).ok()?;
    let timeout = tool.timeout.unwrap_or_else(|| guest.deadline());
    let schema = Value::Object(tool.input_schema.clone());
    let identity = serde_json::to_vec(&json!([
        entry,
        tool.name,
        tool.description,
        canonical_json(&schema),
        timeout.as_millis().to_string(),
    ]))
    .ok()?;
    let descriptor = ToolDescriptor {
        name,
        description: tool.description.clone(),
        input_schema: schema,
        exposure: ToolExposure::Direct,
        execution_class: ToolExecutionClass::Exclusive,
        effect: ToolEffect::Mutating,
        action: AgentActivityAction::USE_TOOL,
        // What a third party's code answers is external content.
        memory_effect: AgentMemoryContextEffect::PollutesContribution,
        timeout: Some(timeout.saturating_add(KERNEL_MARGIN)),
    };
    let handler = GuestTool {
        descriptor,
        guest: guest.clone(),
        provider_id: provider_id.clone(),
        name: tool.name.clone(),
        generation: ToolGeneration(Sha256::digest(&identity).into()),
        timeout,
    };
    Some(SourcedTool {
        provider_id: provider_id.clone(),
        tool_id,
        generation: handler.generation,
        namespace_description: Some(format!("Tools of the installed plugin {}.", package.id)),
        subject: Some(format!("plugin {}", package.id)),
        trust: ToolSourceTrust::Optional,
        handler: Arc::new(handler),
    })
}

struct GuestToolSource {
    id: String,
    guest: GuestCaller,
    /// Planned once, when its guest became active: the declaration does not
    /// change while it runs.
    tools: Vec<SourcedTool>,
}

impl AgentToolSource for GuestToolSource {
    fn id(&self) -> &str {
        &self.id
    }

    fn trust(&self) -> ToolSourceTrust {
        ToolSourceTrust::Optional
    }

    fn contribute<'a>(
        &'a self,
        step: &'a ToolSourceStep,
    ) -> ToolFuture<'a, Result<ToolSourceContribution, AgentToolError>> {
        Box::pin(async move {
            if step.cancellation.is_cancelled() {
                return Err(AgentToolError::Cancelled);
            }
            if !self.guest.running() {
                return Err(AgentToolError::Failed(format!(
                    "the plugin {} is not running",
                    self.guest.id()
                )));
            }
            Ok(ToolSourceContribution::from_tools(self.tools.clone()))
        })
    }
}

struct GuestTool {
    descriptor: ToolDescriptor,
    guest: GuestCaller,
    provider_id: ToolProviderId,
    /// The name its manifest declares, which the guest is called with.
    name: String,
    generation: ToolGeneration,
    timeout: Duration,
}

impl GuestTool {
    /// The action the user approves: this tool, in this version, with these
    /// arguments.
    fn action(&self, arguments: &Value) -> Result<ApprovalAction, AgentToolError> {
        let canonical = serde_json::to_vec(&json!([
            self.provider_id.as_str(),
            self.name,
            hex::encode(self.generation.0),
            canonical_json(arguments),
        ]))
        .map_err(|error| AgentToolError::Failed(error.to_string()))?;
        ApprovalAction::from_canonical_bytes(
            ApprovalActionKind::PluginToolCall,
            ApprovalRiskClass::High,
            ApprovalOrigin::Model,
            format!(
                "Call the tool {} of the plugin {}",
                self.name,
                self.guest.id()
            ),
            &canonical,
        )
        .map_err(|error| AgentToolError::Failed(error.to_string()))
    }
}

impl AgentToolHandler for GuestTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn permission<'a>(
        &'a self,
        invocation: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
        Box::pin(async move {
            if invocation.cancellation.is_cancelled() {
                return Err(AgentToolError::Cancelled);
            }
            Ok(PermissionRequirement::ReviewRequired(
                self.action(&invocation.arguments)?,
            ))
        })
    }

    fn invoke(
        &self,
        invocation: ToolInvocation,
        approval: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>> {
        Box::pin(async move {
            let expected = self.action(&invocation.arguments)?;
            if approval.as_ref().map(AuthorizedAction::action) != Some(&expected) {
                return Err(AgentToolError::Failed(
                    "the plugin tool's approval does not match this call".to_string(),
                ));
            }
            let context = &invocation.context;
            let args = json!({
                "name": self.name,
                "arguments": invocation.arguments,
                "call": {
                    "sessionId": context.session_id.value(),
                    "turnId": context.turn_id.value(),
                    "callId": context.call_id,
                },
            });
            match self
                .guest
                .call(
                    GuestMethod::Tool,
                    args,
                    self.timeout,
                    &invocation.cancellation,
                )
                .await
            {
                Ok(answer) => Ok(result(answer)),
                Err(None) => Err(AgentToolError::Cancelled),
                Err(Some(error)) => Err(AgentToolError::Failed(format!(
                    "the plugin {} failed: {error}",
                    self.guest.id()
                ))),
            }
        })
    }
}

/// A guest's answer as the tool's output: a string is text, anything else
/// structured content.
fn result(answer: Value) -> ToolResult {
    match answer {
        Value::String(text) => ToolOutput {
            content: vec![ToolOutputContent::Text { text }],
            structured_content: None,
            is_error: false,
        }
        .into(),
        other => ToolResult::success(other),
    }
}
