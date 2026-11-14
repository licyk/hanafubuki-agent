//! A `!command`: the command the person typed, through the planned
//! `exec_command` tool's handler as the person's own call (its approval is
//! the person's unless an approval rule asks again), in the turn's sandbox.
//! Its turn opens with its `UserShellCommand` item, which says it did not
//! finish (all that remains of it when it is interrupted or fails, or the
//! application stops) until the command's end writes what it did into it,
//! as a permission review's item is completed in place. The model reads it
//! from the next request on.
//!
//! No model is sampled. The model made no call, so neither a
//! `ToolCall`/`ToolOutput` pair nor the tool-lifecycle hooks of a model's
//! call are part of it; its events are a call's (`tool_started`, its
//! execution progress, `tool_execution_finished`, `tool_completed`), so an
//! interface shows it as a command.

use std::time::Duration;

use hanafubuki_agent_contracts::{output_text, structured_output, AgentToolDisplay};
use hanafubuki_agent_tool_contracts::ToolOutput;

use super::*;

/// The tool a `!command` runs through.
const EXEC_COMMAND: &str = "exec_command";

impl AgentService {
    pub(in crate::service) async fn run_user_shell_command(
        &self,
        identity: TurnIdentity,
        command: crate::AgentUserShellCommand,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        let (catalog_context, cancellation) = {
            let cell = self.session(identity.session_id)?;
            let record = cell.lock_after_persistence().await?;
            let turn = exact_turn_identity(&record, identity)?;
            (
                tool_catalog_context(&record, identity, 1)?,
                turn.cancellation.clone(),
            )
        };
        let plan = self
            .build_tool_plan(catalog_context, cancellation.clone())
            .await
            .map_err(|error| match error {
                crate::AgentToolCatalogError::Cancelled => AgentError::Cancelled,
                error => AgentError::Protocol(error.to_string()),
            })?;
        self.reserve_tool_invocations(identity, 1).await?;
        let maximum_in_flight = self.turn_budget_policy(identity)?.maximum_in_flight_tools;
        let runtime = crate::ToolCallRuntime::new(plan, maximum_in_flight)
            .map_err(|error| AgentError::Protocol(error.to_string()))?;
        let call = ModelToolCall {
            item_id: String::new(),
            call_id: user_shell_call_id(identity.turn_id),
            namespace: None,
            name: EXEC_COMMAND.to_string(),
            arguments: json!({ "cmd": command.command, "workdir": command.workdir }),
            argument_error: None,
        };
        let disposition = CallDisposition::of(runtime.catalog(), &call);
        let CallDisposition::Run(ref registration) = disposition else {
            return Err(AgentError::InvalidInput(
                "`!` commands run through the exec_command tool, which this session does not offer now"
                    .to_string(),
            ));
        };
        let presenters = &self.inner.registries.tool_presenters;
        self.set_phase_and_emit(
            identity,
            AgentTurnPhase::RunningTools,
            AgentEventKind::ToolStarted {
                call_id: call.call_id.clone(),
                name: disposition.tool_name(&call).to_string(),
                presentation: disposition.presentation(presenters, &call),
            },
            event_sink,
        )
        .await?;
        let started = Instant::now();
        let normalized = super::scheduler::normalized_call(&call, registration);
        let output = self
            .execute_handler_tool_call(
                identity,
                &normalized,
                registration,
                &runtime,
                cancellation,
                event_sink,
            )
            .await?;
        let (output, truncation) = crate::tool_output::canonicalize_agent_tool_output(
            output,
            self.inner.limits.maximum_tool_output_bytes,
        );
        let is_error = output.is_error();
        let item = user_shell_command_item(
            call.call_id.clone(),
            command,
            presenters.display(
                EXEC_COMMAND,
                &normalized.arguments,
                &output.output,
                &output.display_metadata,
            ),
            &output.output,
            truncation.is_some(),
            started.elapsed(),
        );
        self.emit(
            identity,
            AgentEventKind::ToolExecutionFinished {
                call_id: call.call_id.clone(),
                is_error,
            },
            event_sink,
        )
        .await?;
        self.complete_user_shell_command(identity, item).await?;
        self.emit(
            identity,
            AgentEventKind::ToolCompleted {
                call_id: call.call_id,
                is_error,
            },
            event_sink,
        )
        .await
    }

    /// Writes what the command did into the turn's item, which its
    /// admission committed.
    async fn complete_user_shell_command(
        &self,
        identity: TurnIdentity,
        item: ConversationItem,
    ) -> Result<(), AgentError> {
        let cell = self.session(identity.session_id)?;
        let staged = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let previous = record.rollback_state();
            let index = record
                .conversation
                .history
                .iter()
                .rposition(|committed| {
                    committed.turn_id == Some(identity.turn_id)
                        && matches!(committed.content, ConversationItem::UserShellCommand { .. })
                })
                .ok_or_else(|| {
                    AgentError::Persistence("the command's history item is missing".to_string())
                })?;
            let (previous_bytes, updated_bytes) = {
                let committed = Arc::make_mut(&mut mutable_history(record)[index]);
                let previous_bytes = committed.content.retained_bytes();
                committed.content = item;
                (previous_bytes, committed.content.retained_bytes())
            };
            let history_bytes = record
                .conversation
                .history_bytes
                .saturating_sub(previous_bytes)
                .saturating_add(updated_bytes);
            if history_bytes > self.inner.limits.maximum_history_bytes {
                record.restore_rollback_state(previous);
                return Err(AgentError::LimitExceeded {
                    resource: "history bytes",
                    limit: self.inner.limits.maximum_history_bytes,
                });
            }
            record.conversation.history_bytes = history_bytes;
            persist_history_item_snapshot(record, index)?;
            StagedPersistence::commit(&cell, record, previous)
        };
        self.persist_staged(staged).await
    }
}

/// The kernel's call id of the command of turn `turn_id`.
fn user_shell_call_id(turn_id: AgentTurnId) -> String {
    format!("user-shell-{turn_id}")
}

/// The item a `!command`'s admission commits: what remains of a command
/// that never finished.
pub(in crate::service) fn unfinished_user_shell_command(
    turn_id: AgentTurnId,
    command: &crate::AgentUserShellCommand,
) -> ConversationItem {
    ConversationItem::UserShellCommand {
        call_id: user_shell_call_id(turn_id),
        command: command.command.clone(),
        cwd: command.workdir.clone(),
        exit_code: None,
        duration_millis: 0,
        output: "The command did not finish.".to_string(),
        truncated: false,
        styled: None,
    }
}

/// What the command did, from the presenter's display of the tool's
/// output; a call that ran no command to its end (refused, stopped, failed
/// to start) has no exit code, and its error is the output.
fn user_shell_command_item(
    call_id: String,
    command: crate::AgentUserShellCommand,
    display: AgentToolDisplay,
    output: &ToolOutput,
    cut: bool,
    elapsed: Duration,
) -> ConversationItem {
    let elapsed_millis = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    match display {
        AgentToolDisplay::Command {
            cwd,
            output: text,
            exit_code,
            duration_millis,
            truncated,
            styled,
            ..
        } if !output.is_error => ConversationItem::UserShellCommand {
            call_id,
            command: command.command,
            cwd,
            exit_code,
            duration_millis: duration_millis.unwrap_or(elapsed_millis),
            output: text,
            truncated: truncated || cut,
            // A bounded output is no longer the text they style.
            styled: styled.filter(|_| !cut),
        },
        _ => ConversationItem::UserShellCommand {
            call_id,
            command: command.command,
            cwd: command.workdir,
            exit_code: None,
            duration_millis: elapsed_millis,
            output: error_text(output),
            truncated: cut,
            styled: None,
        },
    }
}

/// A tool error's message, else the output's text.
fn error_text(output: &ToolOutput) -> String {
    structured_output(output)
        .and_then(|value| value.get("error"))
        .and_then(|error| error.as_str())
        .map_or_else(|| output_text(output), str::to_string)
}

/// How a person sees `item` when it is the `UserShellCommand` of the call
/// `call_id`: a command and what it printed, as the execution tool's
/// presenter shows one.
pub(in crate::service) fn user_shell_command_display(
    item: &ConversationItem,
    call_id: &str,
) -> Option<crate::AgentToolCallDisplay> {
    let ConversationItem::UserShellCommand {
        call_id: id,
        command,
        cwd,
        exit_code,
        duration_millis,
        output,
        truncated,
        styled,
    } = item
    else {
        return None;
    };
    if id != call_id {
        return None;
    }
    let display = AgentToolDisplay::Command {
        command: command.clone(),
        cwd: cwd.clone(),
        output: output.clone(),
        exit_code: *exit_code,
        duration_millis: Some(*duration_millis),
        truncated: *truncated,
        styled: None,
    };
    let metadata = hanafubuki_agent_contracts::AgentToolDisplayMetadata {
        styled_output: styled.clone(),
        ..Default::default()
    };
    Some(crate::AgentToolCallDisplay {
        subject: Some(command.clone()),
        display: Some(display.with_metadata(&metadata)),
    })
}
