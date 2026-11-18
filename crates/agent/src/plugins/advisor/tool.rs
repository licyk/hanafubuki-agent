use super::AdvisorService;
use super::Consultation;
use hanafubuki_agent_approval::{ApprovalAction, AuthorizedAction, PermissionRequirement};
use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentMemoryContextEffect, AgentToolError, AgentToolHandler,
    AgentToolSource, SourcedTool, ToolDescriptor, ToolEffect, ToolFuture, ToolInvocation,
    ToolResult, ToolSourceContribution, ToolSourceStep, ToolSourceTrust,
};
use hanafubuki_agent_tool_contracts::{
    ToolExecutionClass, ToolExposure, ToolGeneration, ToolId, ToolName, ToolProviderId,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub(super) struct AdvisorSource {
    service: Arc<AdvisorService>,
    tool: SourcedTool,
    outcome: SourcedTool,
}

impl AdvisorSource {
    pub(super) fn new(service: Arc<AdvisorService>) -> Result<Self, kardia_core::Error> {
        let name = "ask_advisor";
        let descriptor = ToolDescriptor {
            name: ToolName::parse(name).map_err(kardia_core::Error::new)?,
            description:"Ask the configured Advisor for an independent second opinion on the current conversation. Optionally supply a specific question and an unverified draft.".into(),
            input_schema:json!({"type":"object","properties":{"question":{"type":"string"},"draft":{"type":"string"},"followUpTo":{"type":"string"},"gitContext":{"type":"string","enum":["none","summary","full"]},"includeTrackedFiles":{"type":"array","items":{"type":"string"}},"includeUntracked":{"type":"array","items":{"type":"string"}}},"additionalProperties":false}),
            exposure: ToolExposure::Direct,
            execution_class: ToolExecutionClass::ConcurrentRead,
            effect: ToolEffect::ReadOnly,
            action: AgentActivityAction::fixed("ask_advisor"),
            memory_effect: AgentMemoryContextEffect::None,
            timeout: None,
        };
        let tool = sourced(service.clone(), name, descriptor)?;
        let name = "record_advisor_outcome";
        let outcome=sourced(service.clone(),name,ToolDescriptor {
            name:ToolName::parse(name).map_err(kardia_core::Error::new)?,
            description:"Record the adoption and validation of the latest Advisor response in this conversation.".into(),
            input_schema:json!({"type":"object","properties":{"adviceId":{"type":"string"},"adoption":{"enum":["followed","not-followed","unknown"]},"validationStatus":{"enum":["passed","failed","not-run","unknown"]}},"required":["adviceId","adoption","validationStatus"],"additionalProperties":false}),
            exposure:ToolExposure::Direct,execution_class:ToolExecutionClass::Exclusive,effect:ToolEffect::Mutating,
            action:AgentActivityAction::fixed(name),memory_effect:AgentMemoryContextEffect::None,timeout:None,
        })?;
        Ok(Self {
            service,
            tool,
            outcome,
        })
    }
}
fn sourced(
    service: Arc<AdvisorService>,
    name: &str,
    descriptor: ToolDescriptor,
) -> Result<SourcedTool, kardia_core::Error> {
    let generation = ToolGeneration(Sha256::digest(descriptor.input_schema.to_string()).into());
    Ok(SourcedTool {
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
    })
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
                    let mut tools = vec![self.tool.clone()];
                    if self.service.options.read().record_outcomes {
                        tools.push(self.outcome.clone());
                    }
                    tools
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
            if self.descriptor.action.as_str() == "record_advisor_outcome" {
                let feedback: super::outcomes::Feedback = serde_json::from_value(call.arguments)
                    .map_err(|e| AgentToolError::Failed(e.to_string()))?;
                if !self.service.options.read().record_outcomes
                    || !self
                        .service
                        .previous
                        .lock()
                        .get(&id)
                        .is_some_and(|p| p.id == feedback.advice_id)
                {
                    return Err(AgentToolError::Failed("Recording is disabled or the advice ID does not match the latest consultation.".into()));
                }
                let ledger = self.service.ledger.clone();
                self.service
                    .blocking
                    .run("record advisor outcome", move || ledger.record(feedback))
                    .await
                    .map_err(|e| AgentToolError::Failed(e.to_string()))?
                    .map_err(AgentToolError::Failed)?;
                return Ok(ToolResult::success(json!({"recorded":true})));
            }
            let mut args: Consultation = serde_json::from_value(call.arguments)
                .map_err(|e| AgentToolError::Failed(e.to_string()))?;
            args.root = call
                .turn
                .get::<hanafubuki_agent_contracts::TurnWorkspace>()
                .and_then(|w| w.roots.first().cloned());
            let result = self
                .service
                .consult(id, args, call.cancellation)
                .await
                .map_err(|e| AgentToolError::Failed(e.to_string()))?;
            Ok(ToolResult::success(json!(result)))
        })
    }
}

pub(super) struct AdvisorPresenter;

impl hanafubuki_agent_contracts::AgentToolPresenter for AdvisorPresenter {
    fn tools(&self) -> &[&'static str] {
        &["ask_advisor", "record_advisor_outcome"]
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
