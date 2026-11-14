// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core/src/tools/handlers/multi_agents_spec.rs` (the MultiAgentV2
// tools' names, descriptions and schemas) and
// `codex-rs/core/src/tools/handlers/multi_agents_v2/` (their arguments and
// output) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: plain tool names rather than a namespace;
// `agent_type` names one of the Agent's spawnable roles; `provider` names
// one of the Agent's enabled Providers, and `model` and `reasoning_effort`
// are checked against what is known of them before the agent exists; what
// is not passed is the role's, else the parent's (the argument wins over
// the role, and a passed model keeps the role's or the parent's effort);
// the spawn returns the Provider, model and effort the agent runs with;
// `list_agents` reports each agent's role and the Provider, model and
// effort it runs on now; no task
// paths (an agent is named by its task name or its id among its parent's
// agents) and no `path_prefix`; `wait_agent` returns
// the final answers instead of a mailbox summary, and returns at once when
// no agent runs; `send_message` and `followup_task` say how the message was
// delivered, and a sub-agent's `send_message` to `parent` reaches the agent
// that spawned it; the descriptions say what this Agent does.

//! The six subagent tools: their specs, their arguments and what each
//! returns to the model.

use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_approval::{ApprovalAction, AuthorizedAction, PermissionRequirement};
use hanafubuki_agent_contracts::tools::arguments::{
    clamped_u64, nullable_bounded_string, required_text, valid_bounded_text, ArgumentNotes,
};
use hanafubuki_agent_contracts::{
    object_arguments, AgentActivityAction, AgentMemoryContextEffect, AgentToolError,
    AgentToolHandler, ToolCallValidationError, ToolDescriptor, ToolEffect, ToolFuture,
    ToolInvocation, ToolResult,
};
use hanafubuki_agent_foundation::agent_settings::MAX_MODEL_REASONING_EFFORT_BYTES;
use hanafubuki_agent_kernel::{AgentError, AgentSessionId, ModelReasoningEffort};
use hanafubuki_agent_tool_contracts::{
    ToolContractError, ToolExecutionClass, ToolExposure, ToolName,
};
use serde_json::{json, Map, Value};

use super::options::WaitBounds;
use super::service::{AgentRunStatus, Delivery, SpawnRequest, SubagentService, WaitOutcome};
use crate::roles::{AgentRoleCatalogReader, DEFAULT_ROLE};
use crate::session_driver::MAXIMUM_MODEL_ID_BYTES;

/// The longest message an agent is sent, as the kernel's steering bound.
const MAXIMUM_MESSAGE_BYTES: usize = 64 * 1024;
const MAXIMUM_TASK_NAME_CHARS: usize = 64;
const MAXIMUM_TARGET_CHARS: usize = 256;
const MAXIMUM_AGENT_TYPE_CHARS: usize = 256;
const MAXIMUM_PROVIDER_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SubagentToolKind {
    SpawnAgent,
    SendMessage,
    FollowupTask,
    WaitAgent,
    ListAgents,
    InterruptAgent,
}

impl SubagentToolKind {
    pub(super) const ALL: [Self; 6] = [
        Self::SpawnAgent,
        Self::SendMessage,
        Self::FollowupTask,
        Self::WaitAgent,
        Self::ListAgents,
        Self::InterruptAgent,
    ];

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::SpawnAgent => "spawn_agent",
            Self::SendMessage => "send_message",
            Self::FollowupTask => "followup_task",
            Self::WaitAgent => "wait_agent",
            Self::ListAgents => "list_agents",
            Self::InterruptAgent => "interrupt_agent",
        }
    }

    /// Waiting and listing read; the others start or change another
    /// conversation's work, so Plan mode leaves them out.
    const fn reads(self) -> bool {
        matches!(self, Self::WaitAgent | Self::ListAgents)
    }
}

pub(super) struct SubagentTool {
    kind: SubagentToolKind,
    service: Arc<SubagentService>,
    descriptor: ToolDescriptor,
}

impl SubagentTool {
    pub(super) fn new(
        kind: SubagentToolKind,
        service: Arc<SubagentService>,
    ) -> Result<Self, ToolContractError> {
        let waits = service.limits().waits;
        let (description, input_schema) = spec(kind, waits);
        Ok(Self {
            kind,
            service,
            descriptor: ToolDescriptor {
                name: ToolName::parse(kind.name())?,
                description,
                input_schema,
                exposure: ToolExposure::Direct,
                execution_class: if kind.reads() {
                    ToolExecutionClass::ConcurrentRead
                } else {
                    ToolExecutionClass::Exclusive
                },
                effect: if kind.reads() {
                    ToolEffect::ReadOnly
                } else {
                    ToolEffect::Mutating
                },
                action: AgentActivityAction::fixed(kind.name()),
                // An agent's final answer may carry what it read outside the
                // Agent.
                memory_effect: if kind == SubagentToolKind::WaitAgent {
                    AgentMemoryContextEffect::PollutesContribution
                } else {
                    AgentMemoryContextEffect::None
                },
                // A wait runs to its own bound, past the turn's tool timeout.
                timeout: (kind == SubagentToolKind::WaitAgent)
                    .then(|| waits.maximum + Duration::from_secs(30)),
            },
        })
    }
}

impl AgentToolHandler for SubagentTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    /// No approval: an agent's own tool calls are approved in its own
    /// conversation, under the permission mode it inherits.
    fn permission<'a>(
        &'a self,
        _invocation: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
        Box::pin(async { Ok(PermissionRequirement::NotRequired) })
    }

    fn invoke(
        &self,
        invocation: ToolInvocation,
        approval: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>> {
        Box::pin(async move {
            if approval.is_some() {
                return Err(AgentToolError::Failed(
                    "a subagent tool received an unrelated approval".to_string(),
                ));
            }
            if invocation.cancellation.is_cancelled() {
                return Err(AgentToolError::Cancelled);
            }
            let parent = invocation.context.session_id;
            let arguments = object_arguments(&invocation.arguments)?;
            let service = &self.service;
            match self.kind {
                SubagentToolKind::SpawnAgent => {
                    let request = spawn_arguments(arguments, &service.roles)?;
                    let task_name = request.task_name.clone();
                    let child = service.spawn(parent, request).await.map_err(tool_error)?;
                    Ok(json!({
                        "task_name": task_name,
                        "agent_id": child.id.to_string(),
                        "provider": child.provider_id,
                        "model": child.model,
                        "reasoning_effort": child.reasoning_effort,
                    })
                    .into())
                }
                SubagentToolKind::SendMessage => {
                    let (target, message) = message_arguments(arguments)?;
                    let delivery = service
                        .send_message(parent, &target, message)
                        .await
                        .map_err(tool_error)?;
                    Ok(delivered(delivery))
                }
                SubagentToolKind::FollowupTask => {
                    let (target, message) = message_arguments(arguments)?;
                    let delivery = service
                        .followup_task(parent, &target, message)
                        .await
                        .map_err(tool_error)?;
                    Ok(delivered(delivery))
                }
                SubagentToolKind::WaitAgent => {
                    let mut notes = ArgumentNotes::default();
                    let waits = service.limits().waits;
                    let timeout = Duration::from_millis(clamped_u64(
                        arguments,
                        "timeout_ms",
                        millis(waits.minimum),
                        millis(waits.maximum),
                        millis(waits.default),
                        &mut notes,
                    )?);
                    let outcome = service
                        .wait(parent, timeout, &invocation.cancellation)
                        .await
                        .map_err(tool_error)?;
                    Ok(waited(outcome).with_argument_notes(&notes))
                }
                SubagentToolKind::ListAgents => {
                    let agents = service.agents_on_models(parent).await.map_err(tool_error)?;
                    let agents = agents
                        .iter()
                        .map(|(agent, model)| {
                            let status = json!(list_status(&agent.status));
                            let mut listed = listed(&agent.name, agent.id, status);
                            listed["agent_type"] = json!(agent.role);
                            listed["provider"] = json!(model.provider_id);
                            listed["model"] = json!(model.model);
                            listed["reasoning_effort"] = json!(model.reasoning_effort);
                            listed
                        })
                        .collect::<Vec<_>>();
                    Ok(json!({ "agents": agents }).into())
                }
                SubagentToolKind::InterruptAgent => {
                    let target = target_argument(arguments)?;
                    let previous = service
                        .interrupt(parent, &target)
                        .await
                        .map_err(tool_error)?;
                    Ok(json!({ "previous_status": list_status(&previous) }).into())
                }
            }
        })
    }
}

fn spawn_arguments(
    arguments: &Map<String, Value>,
    roles: &AgentRoleCatalogReader,
) -> Result<SpawnRequest, AgentToolError> {
    let task_name = required_text(arguments, "task_name", MAXIMUM_TASK_NAME_CHARS, false)?;
    if !task_name.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
    }) {
        return Err(ToolCallValidationError::InvalidArgumentAt {
            path: "task_name".to_string(),
            reason: "use lowercase letters, digits, and underscores".to_string(),
        }
        .into());
    }
    let fork = match nullable_bounded_string(arguments, "fork_turns", 32)? {
        None => true,
        Some(turns) => {
            fork_all(turns.trim()).ok_or_else(|| ToolCallValidationError::InvalidArgumentAt {
                path: "fork_turns".to_string(),
                reason: "must be `none` or `all`".to_string(),
            })?
        }
    };
    let agent_type = nullable_bounded_string(arguments, "agent_type", MAXIMUM_AGENT_TYPE_CHARS)?;
    let agent_type = agent_type
        .as_deref()
        .map(str::trim)
        .filter(|agent_type| !agent_type.is_empty())
        .unwrap_or(DEFAULT_ROLE);
    let role = roles.spawn_role(agent_type).map_err(|spawnable| {
        ToolCallValidationError::InvalidArgumentAt {
            path: "agent_type".to_string(),
            reason: format!(
                "`{agent_type}` is not a role an agent can be spawned as; use one of {}",
                spawnable.join(", ")
            ),
        }
    })?;
    let reasoning_effort = optional_text(
        arguments,
        "reasoning_effort",
        MAX_MODEL_REASONING_EFFORT_BYTES,
    )?
    .map(|effort| {
        ModelReasoningEffort::parse(effort).map_err(|reason| {
            ToolCallValidationError::InvalidArgumentAt {
                path: "reasoning_effort".to_string(),
                reason,
            }
        })
    })
    .transpose()?;
    Ok(SpawnRequest {
        task_name,
        message: message_argument(arguments)?,
        fork,
        role,
        provider: optional_text(arguments, "provider", MAXIMUM_PROVIDER_BYTES)?,
        model: optional_text(arguments, "model", MAXIMUM_MODEL_ID_BYTES)?,
        reasoning_effort,
    })
}

/// A trimmed one-line string of at most `maximum_bytes`; missing, `null`
/// or blank is absent.
fn optional_text(
    arguments: &Map<String, Value>,
    name: &'static str,
    maximum_bytes: usize,
) -> Result<Option<String>, ToolCallValidationError> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.trim().is_empty() => Ok(None),
        Some(Value::String(text)) if valid_bounded_text(text.trim(), maximum_bytes, false) => {
            Ok(Some(text.trim().to_string()))
        }
        Some(_) => Err(ToolCallValidationError::InvalidArgumentAt {
            path: name.to_string(),
            reason: format!("must be a string of one line, at most {maximum_bytes} bytes"),
        }),
    }
}

/// Whether `fork_turns` asks for the parent's history: Codex also accepts a
/// turn count, which inherits all of it.
fn fork_all(turns: &str) -> Option<bool> {
    if turns.eq_ignore_ascii_case("none") {
        Some(false)
    } else if turns.eq_ignore_ascii_case("all") || turns.parse::<u64>().is_ok_and(|count| count > 0)
    {
        Some(true)
    } else {
        None
    }
}

fn message_arguments(arguments: &Map<String, Value>) -> Result<(String, String), AgentToolError> {
    Ok((target_argument(arguments)?, message_argument(arguments)?))
}

fn target_argument(arguments: &Map<String, Value>) -> Result<String, AgentToolError> {
    Ok(
        required_text(arguments, "target", MAXIMUM_TARGET_CHARS, false)?
            .trim()
            .to_string(),
    )
}

fn message_argument(arguments: &Map<String, Value>) -> Result<String, AgentToolError> {
    let message = required_text(arguments, "message", MAXIMUM_MESSAGE_BYTES, false)?;
    if message.trim().is_empty() {
        return Err(AgentToolError::Failed(
            "Empty message can't be sent to an agent".to_string(),
        ));
    }
    Ok(message)
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// A refusal reads as itself; anything else as the kernel says it.
fn tool_error(error: AgentError) -> AgentToolError {
    match error {
        AgentError::Cancelled => AgentToolError::Cancelled,
        AgentError::InvalidInput(message) => AgentToolError::Failed(message),
        error => AgentToolError::Failed(error.to_string()),
    }
}

fn delivered(delivery: Delivery) -> ToolResult {
    json!({
        "delivery": match delivery {
            Delivery::Steered => "delivered to its running turn",
            Delivery::Queued => "queued for its next turn",
            Delivery::Started => "started a new turn",
        }
    })
    .into()
}

fn waited(outcome: WaitOutcome) -> ToolResult {
    let (message, timed_out, agents) = match outcome {
        WaitOutcome::Updates(agents) => ("Wait completed.", false, agents),
        WaitOutcome::Idle => ("None of your agents is still working.", false, Vec::new()),
        WaitOutcome::TimedOut => ("Wait timed out.", true, Vec::new()),
    };
    let agents = agents
        .iter()
        .map(|agent| listed(&agent.name, agent.id, status(&agent.status)))
        .collect::<Vec<_>>();
    json!({ "message": message, "timed_out": timed_out, "agents": agents }).into()
}

fn listed(name: &str, id: AgentSessionId, status: Value) -> Value {
    json!({
        "agent_name": name,
        "agent_id": id.to_string(),
        "agent_status": status,
    })
}

/// Codex's status: a name, or an object carrying the final answer or the
/// error.
fn status(status: &AgentRunStatus) -> Value {
    match status {
        AgentRunStatus::Completed(answer) => json!({ "completed": answer }),
        AgentRunStatus::Errored(error) => json!({ "errored": error }),
        status => json!(list_status(status)),
    }
}

/// The status `list_agents` and `interrupt_agent` show: without the final
/// answer, which only `wait_agent` returns.
fn list_status(status: &AgentRunStatus) -> &'static str {
    match status {
        AgentRunStatus::PendingInit => "pending_init",
        AgentRunStatus::Running => "running",
        AgentRunStatus::Interrupted => "interrupted",
        AgentRunStatus::Completed(_) => "completed",
        AgentRunStatus::Errored(_) => "errored",
    }
}

const SPAWN_AGENT_DESCRIPTION: &str = r#"Spawns an agent to work on the specified task. The agent is a separate conversation that runs in the background with your permission mode, and with your model and your tools unless `model`, `provider`, `reasoning_effort` or its `agent_type` say otherwise; it cannot spawn agents of its own. Refer to it by its task name, or by the agent id this returns, in the other agent tools.
When it finishes, wait_agent returns its final answer, and the sub-agent guidance says whether it also reaches you by itself. You can send it messages and follow-up tasks meanwhile.

Note that passing `fork_turns="none"` will not pass any surrounding context to the spawned agent, which may cause the agent to lack the context it needs to complete its task, whereas `fork_turns="all"` will provide the agent with this conversation up to your current turn. Either way the message is the only part of the current turn the agent reads, so make it self-contained."#;

const WAIT_AGENT_DESCRIPTION: &str = "Wait until an agent you spawned finishes its turn. Returns the final status of every agent that finished and that you have not been told of yet: its final answer when it completed, or its error. Returns at once when such an update is pending or when none of your agents is still working, and with `timed_out` when no agent finishes before the deadline.";

/// The model-visible description and input schema of `kind`, with `waits`
/// the bounds `wait_agent` states. `spawn_agent`'s names no role: a spec is
/// fixed while the plugin runs, and the roles are listed by the guidance,
/// which reads them at each step.
fn spec(kind: SubagentToolKind, waits: WaitBounds) -> (String, Value) {
    let target = |description: &str| json!({ "type": "string", "description": description });
    let (description, properties, required): (&str, Value, &[&str]) = match kind {
        SubagentToolKind::SpawnAgent => (
            SPAWN_AGENT_DESCRIPTION,
            json!({
                "task_name": {
                    "type": "string",
                    "description": "Task name for the new agent. Use lowercase letters, digits, and underscores."
                },
                "message": {
                    "type": "string",
                    "description": "Initial plain-text task for the new agent."
                },
                "fork_turns": {
                    "type": "string",
                    "description": "Parent history to inherit. Defaults to `all`; use `none` to start without parent history. Only `all` and `none` are supported."
                },
                "agent_type": {
                    "type": "string",
                    "description": "Role of the new agent: one of the roles listed in the sub-agent guidance. Defaults to `default`, which keeps your tools and instructions. A role can only take tools away, and may set a model."
                },
                "model": {
                    "type": "string",
                    "description": "Model id for the new agent: one the sub-agent guidance lists, or another its Provider serves. Defaults to the role's model, else yours."
                },
                "reasoning_effort": {
                    "type": "string",
                    "description": "Reasoning effort for the new agent, one its model takes (the guidance lists them where known). Defaults to the role's, else yours."
                },
                "provider": {
                    "type": "string",
                    "description": "Provider of `model`: the id (or name) of an enabled Provider, as the sub-agent guidance lists them. Defaults to yours."
                }
            }),
            &["task_name", "message"],
        ),
        SubagentToolKind::SendMessage => (
            "Send a message to an existing agent, or, with `target` `parent`, to the agent that spawned you. An agent that is running reads it promptly, at its next message boundary; an idle sub-agent reads it with its next follow-up task, and an idle parent is resumed to read it. Does not give a sub-agent a new turn.",
            json!({
                "target": target("Task name or agent id of the agent to message (from spawn_agent), or `parent`."),
                "message": { "type": "string", "description": "Message text to send to the target agent." }
            }),
            &["target", "message"],
        ),
        SubagentToolKind::FollowupTask => (
            "Send a follow-up task to an existing agent and trigger a turn if it is idle. If the agent is already running, deliver the task promptly at message boundaries while sampling, or after the pending tool call completes.",
            json!({
                "target": target("Task name or agent id of the agent to send a follow-up task to (from spawn_agent)."),
                "message": { "type": "string", "description": "Message text to send to the target agent." }
            }),
            &["target", "message"],
        ),
        SubagentToolKind::WaitAgent => (
            WAIT_AGENT_DESCRIPTION,
            json!({
                "timeout_ms": {
                    "type": "integer",
                    "description": format!(
                        "Timeout in milliseconds. Defaults to {}, min {}, max {}.",
                        millis(waits.default),
                        millis(waits.minimum),
                        millis(waits.maximum)
                    )
                }
            }),
            &[],
        ),
        SubagentToolKind::ListAgents => (
            "List the agents you spawned in this conversation and the status of each.",
            json!({}),
            &[],
        ),
        SubagentToolKind::InterruptAgent => (
            "Interrupt an agent's current turn, if any, and return its previous status. The agent remains available for messages and follow-up tasks.",
            json!({
                "target": target("Task name or agent id of the agent to interrupt (from spawn_agent).")
            }),
            &["target"],
        ),
    };
    let mut schema = json!({
        "type": "object",
        "properties": properties,
        "additionalProperties": false
    });
    if !required.is_empty() {
        schema["required"] = json!(required);
    }
    (description.to_string(), schema)
}
