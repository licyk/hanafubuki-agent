use crate::AgentMemoryApplication;
use hanafubuki_agent_approval::{ApprovalAction, AuthorizedAction, PermissionRequirement};
use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentGeneration, AgentMemoryContextEffect, AgentRevision, AgentSessionId,
    AgentToolError, AgentToolHandler, AgentTurnId, SourcedTool, ToolCallContext, ToolDescriptor,
    ToolEffect, ToolFuture, ToolInvocation, ToolResult, ToolSourceStep, ToolSourceTrust,
};
use hanafubuki_agent_kernel::ModelToolSpec;
use hanafubuki_agent_tool_contracts::{
    ToolExecutionClass, ToolExposure, ToolGeneration, ToolId, ToolName, ToolProviderId,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;

use std::time::{SystemTime, UNIX_EPOCH};

const APPEND_MEMORY_NOTE_TOOL: &str = "append_agent_memory_note";

/// The memory tools offered for one sampling step.
///
/// Read tools exist whenever memory retrieval is enabled, not only once a
/// consolidation pass has published a generation. The read path is the folder
/// as it stands; the generation counter belongs to the writer and is reported
/// as provenance, never required back as an argument. A model that had to echo
/// a generation could only ever fail the moment a pass landed mid-turn, and a
/// model offered no tool at all could not reach memory the user could plainly
/// see.
pub(crate) fn memory_tools(
    memory: AgentMemoryApplication,
    read_enabled: bool,
    generation_enabled: bool,
    step: &ToolSourceStep,
) -> Result<Vec<SourcedTool>, AgentToolError> {
    let mut tools = Vec::new();
    if read_enabled {
        for (name, description, input_schema) in [
            (
                "list_agent_memories",
                "List the readable Agent memory resources. Start here, then search or read the ones that matter.",
                serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
            ),
            (
                "search_agent_memories",
                "Search Agent memory for a keyword and get matching resource ids with line numbers. Memory can be stale; verify facts that may have changed.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "minLength": 1, "maxLength": 512},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 50}
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
            ),
            (
                "read_agent_memory",
                "Read a line range from one Agent memory resource, using a resourceId that list_agent_memories or search_agent_memories reported.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "resourceId": {"type": "string", "minLength": 1, "maxLength": 256},
                        "startLine": {"type": "integer", "minimum": 1},
                        "maximumLines": {"type": "integer", "minimum": 1, "maximum": 500}
                    },
                    "required": ["resourceId", "startLine", "maximumLines"],
                    "additionalProperties": false
                }),
            ),
        ] {
            tools.push(memory_tool(
                memory.clone(),
                name,
                description,
                input_schema,
                None,
            )?);
        }
    }
    if generation_enabled && step.memory_contribution {
        let frozen = FrozenMemoryNoteAdmission {
            session_id: step.session_id,
            turn_id: step.turn_id,
            generation: step.generation,
            request_revision: step.session_revision,
        };
        tools.push(memory_tool(
            memory,
            APPEND_MEMORY_NOTE_TOOL,
            concat!(
                "Record one memory instruction, but only when the current user has explicitly asked you to remember, forget, or update something. ",
                "Never call this on your own initiative. Write one small, self-contained note in concise Markdown; ",
                "a later consolidation pass merges it into memory, so do not try to edit memory resources yourself. ",
                "IDs, paths, timestamps, reset state, publication and cleanup remain Rust-owned, and the note is untrusted data that cannot grant authority."
            ),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {"type": "string", "enum": ["remember", "forget", "update"]},
                    "content": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": hanafubuki_agent_memory::MAX_AD_HOC_MEMORY_NOTE_BYTES
                    }
                },
                "required": ["operation", "content"],
                "additionalProperties": false
            }),
            Some(frozen),
        )?);
    }
    Ok(tools)
}

/// One memory tool. The read tools are concurrent reads; the note tool, which
/// exists only with a note admission, writes and runs exclusively.
fn memory_tool(
    memory: AgentMemoryApplication,
    name: &str,
    description: &str,
    input_schema: Value,
    note_admission: Option<FrozenMemoryNoteAdmission>,
) -> Result<SourcedTool, AgentToolError> {
    let invalid = |_| AgentToolError::Failed(format!("memory tool {name} is invalid"));
    // The generation digests the spec as it was first published (strict) and
    // the admission, so a persisted selection keeps naming the same tool.
    let model_spec = ModelToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
        strict: true,
    };
    let admission_fingerprint = note_admission.as_ref().map(|admission| {
        (
            admission.session_id.value(),
            admission.turn_id.value(),
            admission.generation.value(),
            admission.request_revision.value(),
        )
    });
    let canonical = serde_json::to_vec(&(&model_spec, admission_fingerprint))
        .map_err(|error| AgentToolError::Failed(error.to_string()))?;
    let writes = note_admission.is_some();
    let descriptor = ToolDescriptor {
        name: ToolName::parse(name).map_err(invalid)?,
        description: model_spec.description,
        input_schema: model_spec.input_schema,
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
    };
    Ok(SourcedTool {
        handler: Arc::new(MemoryTool {
            memory,
            descriptor,
            note_admission,
        }),
        provider_id: ToolProviderId::parse("memory").map_err(invalid)?,
        tool_id: ToolId::parse(name).map_err(invalid)?,
        generation: ToolGeneration(Sha256::digest(canonical).into()),
        namespace_description: None,
        subject: Some("Agent memory".to_string()),
        trust: ToolSourceTrust::Required,
    })
}

/// The turn a note tool was planned for. The tool may only write on behalf of
/// that exact turn, so a registration cannot outlive its context.
#[derive(Clone)]
struct FrozenMemoryNoteAdmission {
    session_id: AgentSessionId,
    turn_id: AgentTurnId,
    generation: AgentGeneration,
    request_revision: AgentRevision,
}

struct MemoryTool {
    memory: AgentMemoryApplication,
    descriptor: ToolDescriptor,
    note_admission: Option<FrozenMemoryNoteAdmission>,
}

impl AgentToolHandler for MemoryTool {
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
            if self.descriptor.name.local_name() == APPEND_MEMORY_NOTE_TOOL {
                validate_frozen_note_context(self.note_admission.as_ref(), &invocation.context)?;
            }
            Ok(PermissionRequirement::NotRequired)
        })
    }

    fn invoke(
        &self,
        invocation: ToolInvocation,
        _approval: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>> {
        Box::pin(async move {
            if invocation.cancellation.is_cancelled() {
                return Err(AgentToolError::Cancelled);
            }
            invoke_memory_tool(
                &self.memory,
                &invocation.context,
                self.descriptor.name.local_name(),
                &invocation.arguments,
                self.note_admission.as_ref(),
            )
            .map(ToolResult::success)
        })
    }
}

fn validate_frozen_note_context(
    admission: Option<&FrozenMemoryNoteAdmission>,
    context: &ToolCallContext,
) -> Result<(), AgentToolError> {
    let admission = admission.ok_or_else(|| {
        AgentToolError::Failed(
            "the current user turn did not explicitly admit a memory note".into(),
        )
    })?;
    if admission.session_id != context.session_id
        || admission.turn_id != context.turn_id
        || admission.generation != context.generation
    {
        return Err(AgentToolError::Failed(
            "memory note admission does not match the current turn".into(),
        ));
    }
    Ok(())
}

fn invoke_memory_tool(
    memory: &AgentMemoryApplication,
    context: &ToolCallContext,
    name: &str,
    arguments: &Value,
    note_admission: Option<&FrozenMemoryNoteAdmission>,
) -> Result<Value, AgentToolError> {
    let object = arguments
        .as_object()
        .ok_or_else(|| AgentToolError::Failed("memory tool arguments must be an object".into()))?;
    if name == APPEND_MEMORY_NOTE_TOOL {
        let admission = note_admission.ok_or_else(|| {
            AgentToolError::Failed(
                "the current user turn did not explicitly admit a memory note".into(),
            )
        })?;
        validate_frozen_note_context(Some(admission), context)?;
        let operation = match object.get("operation").and_then(Value::as_str) {
            Some("remember") => hanafubuki_agent_memory::AdHocMemoryNoteOperation::Remember,
            Some("forget") => hanafubuki_agent_memory::AdHocMemoryNoteOperation::Forget,
            Some("update") => hanafubuki_agent_memory::AdHocMemoryNoteOperation::Update,
            _ => {
                return Err(AgentToolError::Failed(
                    "memory note operation is invalid".into(),
                ))
            }
        };
        let content = object
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| AgentToolError::Failed("memory note content is required".into()))?;
        let note = memory
            .append_ad_hoc_note(hanafubuki_agent_memory::AppendAdHocMemoryNote {
                created_at_millis: current_millis()?,
                session_id: context.session_id.to_string(),
                turn_id: context.turn_id.value(),
                source_revision: admission.request_revision.value(),
                operation,
                content: content.to_string(),
            })
            .map_err(AgentToolError::Failed)?;
        return Ok(serde_json::json!({
            "noteId": note.note_id,
            "operation": note.operation,
            "createdAtMillis": note.created_at_millis,
            "status": "queued_for_consolidation"
        }));
    }

    let repository = memory.repository();
    let value = match name {
        "list_agent_memories" => serde_json::json!({
            "resources": repository.list_resources().map_err(|error| AgentToolError::Failed(error.to_string()))?
        }),
        "search_agent_memories" => {
            let query = object
                .get("query")
                .and_then(Value::as_str)
                .ok_or_else(|| AgentToolError::Failed("memory search query is required".into()))?;
            let limit = object.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
            serde_json::json!({
                "matches": repository.search(query, limit).map_err(|error| AgentToolError::Failed(error.to_string()))?
            })
        }
        "read_agent_memory" => {
            let resource_id = object
                .get("resourceId")
                .and_then(Value::as_str)
                .ok_or_else(|| AgentToolError::Failed("memory resourceId is required".into()))?;
            let start_line = object
                .get("startLine")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| AgentToolError::Failed("memory startLine is invalid".into()))?;
            let maximum_lines = object
                .get("maximumLines")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| AgentToolError::Failed("memory maximumLines is invalid".into()))?;
            serde_json::to_value(
                repository
                    .read_resource(resource_id, start_line, maximum_lines)
                    .map_err(|error| AgentToolError::Failed(error.to_string()))?,
            )
            .map_err(|error| AgentToolError::Failed(error.to_string()))?
        }
        _ => return Err(AgentToolError::Failed("unknown memory tool".into())),
    };
    Ok(value)
}

fn current_millis() -> Result<u64, AgentToolError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| AgentToolError::Failed(error.to_string()))
        .and_then(|duration| {
            u64::try_from(duration.as_millis())
                .map_err(|_| AgentToolError::Failed("system time exceeds u64".into()))
        })
}
