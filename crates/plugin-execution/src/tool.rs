use std::sync::Arc;

use std::time::Instant;

use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalOrigin, AuthorizedAction, PermissionRequirement,
};
use hanafubuki_agent_contracts::tools::arguments::ArgumentNotes;
use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentMemoryContextEffect, AgentStyledText, AgentToolDisplayMetadata,
    AgentToolError, AgentToolHandler, AgentToolProgressUpdate, BlockingService, ToolCallOrigin,
    ToolDescriptor, ToolEffect, ToolFuture, ToolInvocation, ToolResult,
};
use hanafubuki_agent_execution::{
    AgentExecutionError, AgentExecutionId, AgentExecutionScope, AgentExecutionService,
    UnifiedExecResult, WriteStdinRequest,
};
use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_sandbox::SandboxSettings;
use hanafubuki_agent_tool_contracts::{
    ToolContractError, ToolExecutionClass, ToolExposure, ToolName,
};
use serde_json::Value;

use crate::arguments::{parse, ExecCall, MAX_YIELD_TIME_MS};
use crate::monitor::{
    schedule_execution_cleanup_retry, start_execution_activity, ExecutionActivity,
};
use crate::specs::spec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExecToolKind {
    ExecCommand,
    WriteStdin,
    ResizeExecution,
}

impl ExecToolKind {
    pub(crate) const ALL: [Self; 3] = [Self::ExecCommand, Self::WriteStdin, Self::ResizeExecution];

    fn name(self) -> &'static str {
        match self {
            Self::ExecCommand => "exec_command",
            Self::WriteStdin => "write_stdin",
            Self::ResizeExecution => "resize_execution",
        }
    }
}

/// The services every execution tool shares: the executions, the workers
/// that stream their output, and the blocking boundary.
#[derive(Clone)]
pub(crate) struct ExecServices {
    pub(crate) executions: AgentExecutionService,
    pub(crate) workers: BackgroundWorkerGroup,
    pub(crate) blocking: Arc<BlockingService>,
}

/// One execution tool. `exec_command` needs approval of the exact prepared
/// command (the execution service re-checks it before spawning); input and
/// resize act only on executions this session already owns.
pub(crate) struct ExecTool {
    kind: ExecToolKind,
    services: ExecServices,
    descriptor: ToolDescriptor,
}

impl ExecTool {
    pub(crate) fn new(
        kind: ExecToolKind,
        services: ExecServices,
    ) -> Result<Self, ToolContractError> {
        let (description, input_schema) = spec(kind);
        Ok(Self {
            kind,
            services,
            descriptor: ToolDescriptor {
                name: ToolName::parse(kind.name())?,
                description: description.to_string(),
                input_schema,
                exposure: ToolExposure::Direct,
                execution_class: ToolExecutionClass::Exclusive,
                effect: ToolEffect::Mutating,
                action: AgentActivityAction::fixed(kind.name()),
                memory_effect: AgentMemoryContextEffect::None,
                timeout: None,
            },
        })
    }
}

impl AgentToolHandler for ExecTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn permission<'a>(
        &'a self,
        invocation: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
        Box::pin(async move {
            let (ExecCall::ExecCommand(request), _) = parse_call(self.kind, invocation)? else {
                return Ok(PermissionRequirement::NotRequired);
            };
            let executions = self.services.executions.clone();
            let prepared = self
                .services
                .blocking
                .run("agent exec_command prepare", move || {
                    executions.prepare_exec_command_action(&request)
                })
                .await?;
            Ok(match prepared {
                Ok(action) => PermissionRequirement::ReviewRequired(action),
                Err(error) => PermissionRequirement::Forbidden {
                    reason: error.to_string(),
                },
            })
        })
    }

    fn invoke(
        &self,
        invocation: ToolInvocation,
        approval: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>> {
        Box::pin(async move {
            let (call, notes) = parse_call(self.kind, &invocation)?;
            let services = self.services.clone();
            let activity = ExecutionActivity {
                session_id: invocation.context.session_id,
                progress: invocation.progress.clone(),
            };
            let cancellation = invocation.cancellation.clone();
            let result = self
                .services
                .blocking
                .run("agent execution tool", move || {
                    execute(&services, &activity, call, approval.as_ref(), &cancellation)
                })
                .await?;
            if invocation.cancellation.is_cancelled() {
                return Err(AgentToolError::Cancelled);
            }
            result
                .map(|(value, styled_output)| {
                    ToolResult::success(value)
                        .with_argument_notes(&notes)
                        .with_display_metadata(AgentToolDisplayMetadata {
                            styled_output,
                            ..Default::default()
                        })
                })
                .map_err(AgentToolError::Failed)
        })
    }
}

/// The call `invocation` makes, its command confined by the sandbox its
/// turn froze. A turn without one (restored after a restart, which keeps no
/// turn data) runs no command, rather than one unconfined. A command the
/// person typed is the person's (`ApprovalOrigin::User`).
fn parse_call(
    kind: ExecToolKind,
    invocation: &ToolInvocation,
) -> Result<(ExecCall, ArgumentNotes), AgentToolError> {
    let (mut call, notes) = parse(kind, &invocation.arguments)?;
    if let ExecCall::ExecCommand(request) = &mut call {
        if invocation.context.origin == ToolCallOrigin::User {
            request.origin = ApprovalOrigin::User;
        }
        request.sandbox = invocation
            .turn
            .get::<SandboxSettings>()
            .cloned()
            .ok_or_else(|| {
                AgentToolError::Failed(
                    "the turn has no sandbox settings, so it runs no command".to_string(),
                )
            })?;
    }
    Ok((call, notes))
}

/// What a call returns: the result the model reads, and the styles of the
/// output it shows, for a display only.
fn execute(
    services: &ExecServices,
    activity: &ExecutionActivity,
    call: ExecCall,
    approval: Option<&AuthorizedAction>,
    cancellation: &CancellationToken,
) -> Result<(Value, Option<AgentStyledText>), String> {
    let executions = &services.executions;
    let scope = AgentExecutionScope::for_agent_session(activity.session_id.value())
        .map_err(|error| error.to_string())?;
    let retain_cleanup = |execution_id: AgentExecutionId, announce: bool| {
        if let Ok(snapshot) = executions.snapshot(&scope, execution_id) {
            schedule_execution_cleanup_retry(
                executions,
                &services.workers,
                activity,
                scope.clone(),
                &snapshot,
                AgentActivityAction::fixed("exec_command"),
                announce,
            );
        }
    };
    match call {
        ExecCall::ExecCommand(request) => {
            let snapshot = executions
                .start_exec_command(scope.clone(), request.clone(), approval, cancellation)
                .map_err(|error| {
                    if let AgentExecutionError::CleanupRetained { execution_id, .. } = &error {
                        retain_cleanup(*execution_id, true);
                    }
                    error.to_string()
                })?;
            start_execution_activity(
                executions,
                &services.workers,
                activity,
                scope.clone(),
                &snapshot,
                AgentActivityAction::fixed("exec_command"),
            )
            .map_err(|error| error.to_string())?;
            let typed_by_user = request.origin == ApprovalOrigin::User;
            let waited = if typed_by_user {
                run_to_exit(
                    executions,
                    &scope,
                    snapshot.execution_id,
                    request.max_output_bytes,
                    cancellation,
                )
            } else {
                executions.wait_unified_execution(
                    &scope,
                    snapshot.execution_id,
                    request.yield_time_ms,
                    request.max_output_bytes,
                    cancellation,
                )
            };
            // A model's command lives on as a background terminal; the
            // person's was stopped and is gone.
            if typed_by_user && matches!(waited, Err(AgentExecutionError::Cancelled)) {
                activity
                    .progress
                    .report(AgentToolProgressUpdate::ExecutionDisposed {
                        execution_id: snapshot.execution_id.value(),
                        generation: snapshot.generation.value(),
                    });
            }
            let result = waited.map_err(|error| {
                if matches!(error, AgentExecutionError::CleanupRetained { .. }) {
                    retain_cleanup(snapshot.execution_id, false);
                }
                error.to_string()
            })?;
            if result.session_id.is_none() {
                activity
                    .progress
                    .report(AgentToolProgressUpdate::ExecutionDisposed {
                        execution_id: snapshot.execution_id.value(),
                        generation: snapshot.generation.value(),
                    });
            }
            model_result(result)
        }
        ExecCall::WriteStdin {
            session_id,
            chars,
            yield_time_ms,
            max_output_bytes,
        } => {
            let execution_id =
                AgentExecutionId::from_untrusted(session_id).map_err(|error| error.to_string())?;
            let generation = executions
                .snapshot(&scope, execution_id)
                .map_err(|error| error.to_string())?
                .generation;
            let result = executions
                .write_stdin(
                    &scope,
                    WriteStdinRequest {
                        session_id: execution_id,
                        chars,
                        yield_time_ms,
                        max_output_bytes,
                    },
                    cancellation,
                )
                .map_err(|error| {
                    if matches!(error, AgentExecutionError::CleanupRetained { .. }) {
                        retain_cleanup(execution_id, false);
                    }
                    error.to_string()
                })?;
            if result.session_id.is_none() {
                activity
                    .progress
                    .report(AgentToolProgressUpdate::ExecutionDisposed {
                        execution_id: execution_id.value(),
                        generation: generation.value(),
                    });
            }
            model_result(result)
        }
        ExecCall::Resize {
            session_id,
            cols,
            rows,
        } => {
            let execution_id =
                AgentExecutionId::from_untrusted(session_id).map_err(|error| error.to_string())?;
            let generation = executions
                .snapshot(&scope, execution_id)
                .map_err(|error| error.to_string())?
                .generation;
            let resized = serde_json::to_value(
                executions
                    .resize_interactive(&scope, execution_id, generation, cols, rows)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            Ok((resized, None))
        }
    }
}

/// `result` as the model reads it (its styles are not serialized), and its
/// styles beside it.
fn model_result(mut result: UnifiedExecResult) -> Result<(Value, Option<AgentStyledText>), String> {
    let styles = result.styles.take();
    let value = serde_json::to_value(result).map_err(|error| error.to_string())?;
    Ok((value, styles))
}

/// A command the person typed runs until it exits (Codex's user shell
/// command), within the call's timeout, rather than yielding to a
/// background terminal no one would poll. Stopped first (an interrupt, the
/// timeout), it is terminated and forgotten once its end is confirmed.
fn run_to_exit(
    executions: &AgentExecutionService,
    scope: &AgentExecutionScope,
    execution_id: AgentExecutionId,
    max_output_bytes: Option<usize>,
    cancellation: &CancellationToken,
) -> Result<UnifiedExecResult, AgentExecutionError> {
    let started = Instant::now();
    loop {
        match executions.wait(scope, execution_id, MAX_YIELD_TIME_MS, cancellation) {
            Ok(waited) if waited.completed => break,
            Ok(_) => {}
            Err(AgentExecutionError::Cancelled) => {
                executions.stop_confirmed(scope, execution_id)?;
                return Err(AgentExecutionError::Cancelled);
            }
            Err(error) => return Err(error),
        }
    }
    // It has ended: the read returns at once, with everything it printed.
    let mut result = executions.wait_unified_execution(
        scope,
        execution_id,
        1,
        max_output_bytes,
        &CancellationToken::default(),
    )?;
    result.wall_time_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(result)
}
