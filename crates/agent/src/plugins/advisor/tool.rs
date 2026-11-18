use super::AdvisorService;
use hanafubuki_agent_approval::{ApprovalAction, AuthorizedAction, PermissionRequirement};
use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentMemoryContextEffect, AgentToolError, AgentToolHandler,
    AgentToolSource, SourcedTool, ToolDescriptor, ToolEffect, ToolFuture, ToolInvocation,
    ToolResult, ToolSourceContribution, ToolSourceStep, ToolSourceTrust,
};
use hanafubuki_agent_tool_contracts::{
    ToolExecutionClass, ToolExposure, ToolGeneration, ToolId, ToolName, ToolProviderId,
};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub(super) struct AdvisorSource {
    service: Arc<AdvisorService>,
    tool: SourcedTool,
}

impl AdvisorSource {
    pub(super) fn new(service: Arc<AdvisorService>) -> Result<Self, kardia_core::Error> {
        let name = "ask_advisor";
        let descriptor = ToolDescriptor {
            name: ToolName::parse(name).map_err(kardia_core::Error::new)?,
            description:"Ask the configured Advisor for an independent second opinion on the current conversation. Optionally supply a specific question and an unverified draft.".into(),
            input_schema:json!({"type":"object","properties":{"question":{"type":"string"},"draft":{"type":"string"}},"additionalProperties":false}),
            exposure: ToolExposure::Direct,
            execution_class: ToolExecutionClass::ConcurrentRead,
            effect: ToolEffect::ReadOnly,
            action: AgentActivityAction::fixed("ask_advisor"),
            memory_effect: AgentMemoryContextEffect::None,
            timeout: None,
        };
        let generation = ToolGeneration(Sha256::digest(descriptor.input_schema.to_string()).into());
        let tool = SourcedTool {
            handler: Arc::new(AskAdvisor {
                descriptor,
                service: service.clone(),
            }),
            provider_id: ToolProviderId::parse("advisor").map_err(kardia_core::Error::new)?,
            tool_id: ToolId::parse(name).map_err(kardia_core::Error::new)?,
            generation,
            namespace_description: None,
            subject: Some("Advisor".into()),
            trust: ToolSourceTrust::Optional,
        };
        Ok(Self { service, tool })
    }
}

impl AgentToolSource for AdvisorSource {
    fn id(&self) -> &str {
        "advisor"
    }
    fn trust(&self) -> ToolSourceTrust {
        ToolSourceTrust::Optional
    }
    fn contribute<'a>(
        &'a self,
        step: &'a ToolSourceStep,
    ) -> ToolFuture<'a, Result<ToolSourceContribution, AgentToolError>> {
        Box::pin(async move {
            Ok(ToolSourceContribution::from_tools(
                if self.service.active(step.session_id) {
                    vec![self.tool.clone()]
                } else {
                    vec![]
                },
            ))
        })
    }
}
struct AskAdvisor {
    descriptor: ToolDescriptor,
    service: Arc<AdvisorService>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct Arguments {
    question: String,
    draft: Option<String>,
}

impl AgentToolHandler for AskAdvisor {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }
    fn permission<'a>(
        &'a self,
        _: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
        Box::pin(async { Ok(PermissionRequirement::NotRequired) })
    }
    fn invoke(
        &self,
        call: ToolInvocation,
        _: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>> {
        Box::pin(async move {
            let id = call.context.session_id;
            if !self.service.active(id) {
                return Err(AgentToolError::Failed("Advisor is inactive".into()));
            }
            let args: Arguments = serde_json::from_value(call.arguments)
                .map_err(|e| AgentToolError::Failed(e.to_string()))?;
            let result = self
                .service
                .consult(id, args.question, args.draft, call.cancellation)
                .await
                .map_err(|e| AgentToolError::Failed(e.to_string()))?;
            Ok(ToolResult::success(json!(result)))
        })
    }
}

pub(super) struct AdvisorPresenter;

impl hanafubuki_agent_contracts::AgentToolPresenter for AdvisorPresenter {
    fn tools(&self) -> &[&'static str] {
        &["ask_advisor"]
    }
    fn subject(&self, _: &str, args: &serde_json::Value) -> Option<String> {
        args.get("question")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    }
    fn display(
        &self,
        _: &str,
        _: &serde_json::Value,
        output: &hanafubuki_agent_tool_contracts::ToolOutput,
    ) -> Option<hanafubuki_agent_contracts::AgentToolDisplay> {
        let answer = hanafubuki_agent_contracts::structured_output(output)?;
        Some(hanafubuki_agent_contracts::AgentToolDisplay::Text {
            text: format!(
                "{}/{}\n{}",
                answer["provider"].as_str()?,
                answer["model"].as_str()?,
                answer["text"].as_str()?
            ),
        })
    }
}
