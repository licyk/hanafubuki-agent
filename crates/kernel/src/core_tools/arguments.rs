//! The kernel tools' typed arguments and their validation.

use hanafubuki_agent_contracts::{decode_arguments, object_arguments, ToolCallValidationError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::AgentCoreTool;

// Argument types ignore unknown fields and read a missing optional field as
// absent: the model's arguments are read tolerantly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateTaskPlanTool {
    #[serde(default)]
    pub explanation: Option<String>,
    pub plan: Vec<crate::model::AgentTaskPlanStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequestUserInputTool {
    pub questions: Vec<crate::model::AgentUserInputQuestion>,
}

/// One validated call of a kernel tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CoreToolCall {
    NewContext,
    UpdatePlan(UpdateTaskPlanTool),
    RequestUserInput(RequestUserInputTool),
    /// A compression tool: its arguments are the algorithm's to read, as
    /// leniently as billion-context reads them.
    Compression(AgentCoreTool),
}

impl CoreToolCall {
    pub(crate) fn parse(
        tool: AgentCoreTool,
        arguments: &Value,
    ) -> Result<Self, ToolCallValidationError> {
        if tool.is_compression() {
            return Ok(Self::Compression(tool));
        }
        let arguments = object_arguments(arguments)?;
        parse(tool, arguments)
    }
}

fn parse(
    tool: AgentCoreTool,
    arguments: &serde_json::Map<String, Value>,
) -> Result<CoreToolCall, ToolCallValidationError> {
    match tool {
        AgentCoreTool::NewContext => Ok(CoreToolCall::NewContext),
        AgentCoreTool::UpdatePlan => {
            let call: UpdateTaskPlanTool = decode_arguments(arguments)?;
            validate_update_task_plan(&call)?;
            Ok(CoreToolCall::UpdatePlan(call))
        }
        AgentCoreTool::RequestUserInput => {
            let call: RequestUserInputTool = decode_arguments(arguments)?;
            validate_user_input_request(&call)?;
            Ok(CoreToolCall::RequestUserInput(call))
        }
        AgentCoreTool::Compress
        | AgentCoreTool::Decompress
        | AgentCoreTool::SearchContext
        | AgentCoreTool::AcpStatus
        | AgentCoreTool::AcpCache => Ok(CoreToolCall::Compression(tool)),
    }
}

fn validate_update_task_plan(call: &UpdateTaskPlanTool) -> Result<(), ToolCallValidationError> {
    if call.plan.is_empty() || call.plan.len() > 20 {
        return Err(ToolCallValidationError::InvalidArgument("plan"));
    }
    if call
        .explanation
        .as_ref()
        .is_some_and(|value| value.len() > 2048)
    {
        return Err(ToolCallValidationError::InvalidArgument("explanation"));
    }
    let mut in_progress = 0;
    for item in &call.plan {
        if item.step.trim().is_empty() || item.step.len() > 1024 {
            return Err(ToolCallValidationError::InvalidArgument("plan"));
        }
        if matches!(
            item.status,
            crate::model::AgentTaskPlanStepStatus::InProgress
        ) {
            in_progress += 1;
        }
    }
    if in_progress > 1 {
        return Err(ToolCallValidationError::InvalidArgument("plan"));
    }
    Ok(())
}

fn validate_user_input_request(call: &RequestUserInputTool) -> Result<(), ToolCallValidationError> {
    if call.questions.is_empty() || call.questions.len() > 3 {
        return Err(ToolCallValidationError::InvalidArgument("questions"));
    }
    let mut question_ids = std::collections::HashSet::new();
    for question in &call.questions {
        if question.id.is_empty()
            || question.id.len() > 64
            || question.header.is_empty()
            || question.header.len() > 64
            || question.question.is_empty()
            || question.question.len() > 1024
            || question.options.len() < 2
            || question.options.len() > 3
        {
            return Err(ToolCallValidationError::InvalidArgument("questions"));
        }
        if !question_ids.insert(question.id.as_str()) {
            return Err(ToolCallValidationError::InvalidArgument("questions"));
        }
        let mut option_ids = std::collections::HashSet::new();
        for option in &question.options {
            if option.id.is_empty()
                || option.id.len() > 64
                || option.label.is_empty()
                || option.label.len() > 128
                || option.description.len() > 512
                || !option_ids.insert(option.id.as_str())
            {
                return Err(ToolCallValidationError::InvalidArgument("questions"));
            }
        }
    }
    Ok(())
}
