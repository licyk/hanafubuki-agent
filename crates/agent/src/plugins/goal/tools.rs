// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/ext/goal/src/spec.rs` (the goal tools' names, descriptions and
// schemas) and `codex-rs/ext/goal/src/tool.rs` (what they do and answer), at
// commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: "thread" reads "session"; the usage-limited
// status does not exist (the Agent has no account usage limit); a goal's
// answer leaves out the thread id and carries a time budget when it has one;
// the tools refuse a session kept in memory only.

//! `get_goal`, `create_goal` and `update_goal`: the model reads the
//! session's goal, creates one when asked to, and marks it complete, blocked
//! or paused.

use std::sync::Arc;

use hanafubuki_agent_approval::{ApprovalAction, AuthorizedAction, PermissionRequirement};
use hanafubuki_agent_contracts::{
    object_arguments, AgentActivityAction, AgentMemoryContextEffect, AgentToolError,
    AgentToolHandler, ToolCallValidationError, ToolDescriptor, ToolEffect, ToolFuture,
    ToolInvocation, ToolResult,
};
use hanafubuki_agent_kernel::{AgentError, AgentGoal, AgentGoalOrigin, AgentGoalStatus};
use hanafubuki_agent_tool_contracts::{
    ToolContractError, ToolExecutionClass, ToolExposure, ToolName,
};
use serde_json::{json, Map, Value};

use super::runtime::{AgentGoalBudgets, GoalRuntime, ModelGoalStatus};

const GET_GOAL_TOOL_NAME: &str = "get_goal";
const CREATE_GOAL_TOOL_NAME: &str = "create_goal";
const UPDATE_GOAL_TOOL_NAME: &str = "update_goal";

#[derive(Clone, Copy)]
enum GoalToolKind {
    Get,
    Create,
    Update,
}

pub(super) struct GoalTool {
    kind: GoalToolKind,
    runtime: Arc<GoalRuntime>,
    descriptor: ToolDescriptor,
}

/// The three goal tools over `runtime`.
pub(super) fn goal_tools(runtime: Arc<GoalRuntime>) -> Result<Vec<GoalTool>, ToolContractError> {
    [
        GoalToolKind::Get,
        GoalToolKind::Create,
        GoalToolKind::Update,
    ]
    .into_iter()
    .map(|kind| {
        Ok(GoalTool {
            kind,
            runtime: runtime.clone(),
            descriptor: descriptor(kind)?,
        })
    })
    .collect()
}

fn descriptor(kind: GoalToolKind) -> Result<ToolDescriptor, ToolContractError> {
    let (name, description, input_schema) = match kind {
        GoalToolKind::Get => (
            GET_GOAL_TOOL_NAME,
            "Get the current goal for this session, including status, budgets, token and elapsed-time usage, and remaining token budget.".to_string(),
            json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        ),
        GoalToolKind::Create => (
            CREATE_GOAL_TOOL_NAME,
            format!(
                "Create a goal only when explicitly requested by the user or system/developer instructions; do not infer goals from ordinary tasks.\nSet token_budget only when an explicit token budget is requested. Fails if an unfinished goal exists; use {UPDATE_GOAL_TOOL_NAME} only for status."
            ),
            json!({
                "type": "object",
                "properties": {
                    "objective": {
                        "type": "string",
                        "description": "Required. The concrete objective to start pursuing. This starts a new active goal when no goal exists or replaces the current goal when it is complete."
                    },
                    "token_budget": {
                        "type": "integer",
                        "description": "Positive token budget for the new goal. Omit unless explicitly requested."
                    }
                },
                "required": ["objective"],
                "additionalProperties": false
            }),
        ),
        GoalToolKind::Update => (
            UPDATE_GOAL_TOOL_NAME,
            "Update the existing goal.\nSet status to `paused` only at the user's explicit request to pause this goal, never on your own initiative. Ask if unclear; a later resume revokes that request. Report the returned status and stop goal work. Budget limits take precedence over pausing.\nSet status to `complete` only when the objective has actually been achieved and no required work remains.\nSet status to `blocked` only when the same blocking condition has repeated for at least three consecutive goal turns, counting the original/user-triggered turn and any automatic continuations, and the agent cannot make meaningful progress without user input or an external-state change.\nIf the user resumes a goal that was previously marked `blocked`, treat the resumed run as a fresh blocked audit. If the same blocking condition then repeats for at least three consecutive resumed goal turns, set status to `blocked` again.\nOnce the blocked threshold is satisfied, do not keep reporting that you are still blocked while leaving the goal active; set status to `blocked`.\nDo not use `blocked` merely because the work is hard, slow, uncertain, incomplete, or would benefit from clarification.\nDo not mark a goal complete merely because its budget is nearly exhausted or because you are stopping work.\nYou cannot use this tool to resume or budget-limit a goal; those status changes are controlled by the user or system.\nWhen marking a budgeted goal achieved with status `complete`, report the final token usage from the tool result to the user.".to_string(),
            json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": ["complete", "blocked", "paused"],
                        "description": "Required. `paused` requires an explicit user request. Set to `complete` only when the objective is achieved and no required work remains. Set to `blocked` only after the same blocking condition has recurred for at least three consecutive goal turns and the agent is at an impasse. After a previously blocked goal is resumed, the resumed run starts a fresh blocked audit."
                    }
                },
                "required": ["status"],
                "additionalProperties": false
            }),
        ),
    };
    let writes = !matches!(kind, GoalToolKind::Get);
    Ok(ToolDescriptor {
        name: ToolName::parse(name)?,
        description,
        input_schema,
        exposure: ToolExposure::Direct,
        execution_class: if writes {
            ToolExecutionClass::Exclusive
        } else {
            ToolExecutionClass::ConcurrentRead
        },
        effect: if writes {
            ToolEffect::Mutating
        } else {
            ToolEffect::ReadOnly
        },
        action: AgentActivityAction::USE_TOOL,
        memory_effect: AgentMemoryContextEffect::None,
        timeout: None,
    })
}

impl AgentToolHandler for GoalTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn permission<'a>(
        &'a self,
        invocation: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
        Box::pin(async move {
            self.parse(&invocation.arguments)?;
            Ok(PermissionRequirement::NotRequired)
        })
    }

    fn invoke(
        &self,
        invocation: ToolInvocation,
        approval: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>> {
        Box::pin(async move {
            if approval.is_some() {
                return Err(AgentToolError::Failed(
                    "a goal tool received an unrelated approval".to_string(),
                ));
            }
            let session_id = invocation.context.session_id;
            let call = self.parse(&invocation.arguments)?;
            self.runtime
                .ensure_persistent(session_id)
                .await
                .map_err(tool_error)?;
            let (goal, report) = match call {
                GoalCall::Get => (self.runtime.goal(session_id).await, false),
                GoalCall::Create {
                    objective,
                    token_budget,
                } => (
                    self.runtime
                        .create(
                            session_id,
                            &objective,
                            AgentGoalBudgets {
                                token_budget,
                                time_budget_seconds: None,
                            },
                            AgentGoalOrigin::Model,
                        )
                        .await
                        .map(Some),
                    false,
                ),
                GoalCall::Update(status) => (
                    self.runtime
                        .update_by_model(session_id, status)
                        .await
                        .map(Some),
                    status == ModelGoalStatus::Complete,
                ),
            };
            Ok(ToolResult::success(goal_response(
                goal.map_err(tool_error)?.as_ref(),
                report,
            )))
        })
    }
}

enum GoalCall {
    Get,
    Create {
        objective: String,
        token_budget: Option<u64>,
    },
    Update(ModelGoalStatus),
}

impl GoalTool {
    fn parse(&self, arguments: &Value) -> Result<GoalCall, ToolCallValidationError> {
        let arguments = object_arguments(arguments)?;
        match self.kind {
            GoalToolKind::Get => {
                reject_unknown(arguments, &[])?;
                Ok(GoalCall::Get)
            }
            GoalToolKind::Create => {
                reject_unknown(arguments, &["objective", "token_budget"])?;
                let objective = arguments
                    .get("objective")
                    .and_then(Value::as_str)
                    .ok_or(ToolCallValidationError::MissingArgument("objective"))?;
                let token_budget = match arguments.get("token_budget") {
                    None | Some(Value::Null) => None,
                    Some(value) => Some(
                        value
                            .as_u64()
                            .filter(|budget| *budget > 0)
                            .ok_or(ToolCallValidationError::InvalidArgument("token_budget"))?,
                    ),
                };
                Ok(GoalCall::Create {
                    objective: objective.to_string(),
                    token_budget,
                })
            }
            GoalToolKind::Update => {
                reject_unknown(arguments, &["status"])?;
                let status = match arguments.get("status").and_then(Value::as_str) {
                    Some("complete") => ModelGoalStatus::Complete,
                    Some("blocked") => ModelGoalStatus::Blocked,
                    Some("paused") => ModelGoalStatus::Paused,
                    Some(_) => {
                        return Err(ToolCallValidationError::InvalidArgumentAt {
                            path: "status".to_string(),
                            reason: "update_goal can only mark the existing goal complete, blocked, or paused at the user's explicit request; resume and budget-limited status changes are controlled by the user or system".to_string(),
                        })
                    }
                    None => return Err(ToolCallValidationError::MissingArgument("status")),
                };
                Ok(GoalCall::Update(status))
            }
        }
    }
}

fn reject_unknown(
    arguments: &Map<String, Value>,
    accepted: &[&str],
) -> Result<(), ToolCallValidationError> {
    match arguments
        .keys()
        .find(|key| !accepted.contains(&key.as_str()))
    {
        Some(name) => Err(ToolCallValidationError::UnexpectedArgument {
            name: name.clone(),
            accepted: accepted.join(", "),
        }),
        None => Ok(()),
    }
}

/// What a goal tool answers: the goal, the tokens left of its budget, and,
/// when the model just completed a goal with usage to report, how to report
/// it.
pub(super) fn goal_response(goal: Option<&AgentGoal>, completion: bool) -> Value {
    let remaining_tokens = goal.and_then(|goal| {
        goal.token_budget
            .map(|budget| budget.saturating_sub(goal.tokens_used))
    });
    let completion_budget_report = goal
        .filter(|goal| {
            completion
                && goal.status == AgentGoalStatus::Complete
                && (goal.token_budget.is_some() || goal.time_used_seconds > 0)
        })
        .map(|_| {
            "Goal achieved. Report final usage from this tool result's structured goal fields. If `goal.tokenBudget` is present, include token usage from `goal.tokensUsed` and `goal.tokenBudget`. If `goal.timeUsedSeconds` is greater than 0, summarize elapsed time in a concise, human-friendly form appropriate to the response language."
        });
    json!({
        "goal": goal.map(goal_json),
        "remainingTokens": remaining_tokens,
        "completionBudgetReport": completion_budget_report,
    })
}

fn goal_json(goal: &AgentGoal) -> Value {
    let mut value = json!({
        "objective": goal.objective,
        "status": goal.status,
        "tokenBudget": goal.token_budget,
        "tokensUsed": goal.tokens_used,
        "timeUsedSeconds": goal.time_used_seconds,
        "createdAt": unix_seconds(goal.created_at),
        "updatedAt": unix_seconds(goal.updated_at),
    });
    if let (Some(budget), Some(object)) = (goal.time_budget_seconds, value.as_object_mut()) {
        object.insert("timeBudgetSeconds".to_string(), json!(budget));
    }
    value
}

fn unix_seconds(millis: u128) -> u64 {
    u64::try_from(millis / 1000).unwrap_or(u64::MAX)
}

/// A refusal reads to the model as the reason alone.
fn tool_error(error: AgentError) -> AgentToolError {
    match error {
        AgentError::InvalidInput(message) => AgentToolError::Failed(message),
        error => AgentToolError::Failed(error.to_string()),
    }
}
