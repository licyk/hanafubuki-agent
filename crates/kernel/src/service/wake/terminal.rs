//! A model's background terminals and the end each one reports (D11): an
//! execution a model's call started is tracked from its start; its first
//! terminal status, from a tool's sink, queues a `Terminal` wake event on
//! its session; the disposal a call reports withdraws it, since that call's
//! result told the model of the end. Nothing is queued for a `!command`'s
//! execution, for a `Cancelled` status (only the Agent's own stops produce
//! one), for the Agent's own reports (they name no call), with the policy
//! off, or on an archived session.
//!
//! Whether the call that started the execution is still running decides
//! nothing: a command can exit after its `exec_command` returned its
//! `session_id` but before the kernel committed that result, and only the
//! disposal says whether a result reported the end. The monitor reports a
//! terminal status before the execution can be disposed of, and a call
//! reports the disposal before it returns, so a step, which flushes the
//! progress before it drains the queue, and a turn's end, which flushes it
//! too, never see an end a call's result already reported.
//!
//! The tracking is this run's only: a process does not outlive it, and a
//! disposal withdraws only what this run queued (execution ids start again
//! in each run).

use std::collections::BTreeMap;

use super::*;
use crate::service::tool_execution::is_user_shell_call_id;
use crate::{
    AgentActivityOutputPreview, AgentActivitySnapshot, AgentExecutionProgressStatus,
    AgentTerminalWake, AgentToolProgressUpdate, AgentWakeFilter, AgentWakePolicy, AgentWakeSource,
};

/// The executions a model's calls started in this run, by id.
#[derive(Clone, Debug, Default)]
pub(in crate::service) struct BackgroundTerminals(BTreeMap<u64, TrackedTerminal>);

#[derive(Clone, Debug)]
struct TrackedTerminal {
    generation: u64,
    /// The starting call's presentation subject, as `/ps` shows it.
    command: String,
    /// Its terminal status was applied: it runs no more.
    ended: bool,
    /// Its end is queued, which a call's disposal withdraws.
    queued: bool,
}

impl BackgroundTerminals {
    pub(super) fn contains(&self, id: u64, generation: u64) -> bool {
        self.0
            .get(&id)
            .is_some_and(|terminal| terminal.generation == generation)
    }

    /// The tracked executions still running.
    pub(in crate::service) fn running(&self) -> usize {
        self.0.values().filter(|terminal| !terminal.ended).count()
    }
}

impl AgentService {
    /// The command an `ExecutionStarted` that a model's call reported runs:
    /// its call's presentation subject, read from the call in history
    /// before the update is staged (a presenter is plugin code, never run
    /// under the session's lock). `None` for any other update, a
    /// `!command`'s or the Agent's own report.
    pub(in crate::service) fn started_terminal_command(
        &self,
        cell: &SessionCell,
        call_id: Option<&str>,
        update: &AgentToolProgressUpdate,
    ) -> Option<String> {
        let call_id = call_id.filter(|call_id| !is_user_shell_call_id(call_id))?;
        if !matches!(update, AgentToolProgressUpdate::ExecutionStarted { .. }) {
            return None;
        }
        let call = cell.lock().ok().and_then(|record| {
            record
                .conversation
                .history
                .iter()
                .rev()
                .find_map(|item| match &item.content {
                    ConversationItem::ToolCall {
                        call_id: id,
                        name,
                        arguments,
                        ..
                    } if id == call_id => Some((name.clone(), arguments.clone())),
                    _ => None,
                })
        });
        Some(
            call.and_then(|(name, arguments)| {
                self.inner
                    .registries
                    .tool_presenters
                    .subject(&name, &arguments)
            })
            .unwrap_or_default(),
        )
    }

    /// How many of the session's background terminals still run: the
    /// executions its model's calls started that reported no end yet.
    pub fn background_terminals(&self, session_id: AgentSessionId) -> usize {
        self.session(session_id)
            .and_then(|cell| {
                cell.lock()
                    .map(|record| record.runtime.background_terminals.running())
            })
            .unwrap_or_default()
    }
}

/// Applies `update`, already applied to the session's activities
/// (`activity`, its execution activity now), to its tracked terminals:
/// tracks an execution `command` names, queues the end of a tracked one,
/// withdraws an end on a call's disposal. `from_call`: a tool's sink
/// reported it, not the Agent. Whether it queued an event.
pub(in crate::service) fn track_background_terminal(
    record: &mut SessionRecord,
    from_call: bool,
    update: &AgentToolProgressUpdate,
    activity: Option<&AgentActivitySnapshot>,
    command: Option<String>,
) -> bool {
    match *update {
        AgentToolProgressUpdate::ExecutionStarted {
            execution_id,
            generation,
            ..
        } => {
            if let Some(command) = command.filter(|_| activity.is_some()) {
                record.runtime.background_terminals.0.insert(
                    execution_id,
                    TrackedTerminal {
                        generation,
                        command,
                        ended: false,
                        queued: false,
                    },
                );
            }
            false
        }
        AgentToolProgressUpdate::ExecutionStatus {
            execution_id,
            generation,
            status,
            exit_code,
            ..
        } if status.is_terminal() => {
            let woken = record.runtime.wake_policy == AgentWakePolicy::On
                && record.status.lifecycle != AgentSessionLifecycle::Archived;
            let Some(terminal) = tracked(record, execution_id, generation) else {
                return false;
            };
            if std::mem::replace(&mut terminal.ended, true)
                || !from_call
                || status == AgentExecutionProgressStatus::Cancelled
                || !woken
            {
                return false;
            }
            // No activity: the execution was disposed of already.
            let Some(activity) = activity else {
                return false;
            };
            terminal.queued = true;
            let source = AgentWakeSource::Terminal {
                execution_id,
                command: terminal.command.clone(),
                event: ended(activity, status, exit_code),
            };
            record.runtime.wake_queue.push(source, now_millis_u64());
            log::debug!(
                "agent_diag stage=wake.terminal session_id={} execution_id={execution_id} outcome=queued",
                record.session_id
            );
            true
        }
        AgentToolProgressUpdate::ExecutionDisposed {
            execution_id,
            generation,
        } => {
            let Some(terminal) = tracked(record, execution_id, generation) else {
                return false;
            };
            let queued = terminal.queued;
            record.runtime.background_terminals.0.remove(&execution_id);
            // The Agent's own disposal (a stop, a restart's settling) told
            // the model nothing: the end stays queued.
            if queued && from_call {
                let withdrawn = record
                    .runtime
                    .wake_queue
                    .take(AgentWakeFilter::Terminal { execution_id });
                log::debug!(
                    "agent_diag stage=wake.terminal session_id={} execution_id={execution_id} outcome=withdrawn count={}",
                    record.session_id,
                    withdrawn.len()
                );
            }
            false
        }
        _ => false,
    }
}

fn tracked(
    record: &mut SessionRecord,
    execution_id: u64,
    generation: u64,
) -> Option<&mut TrackedTerminal> {
    record
        .runtime
        .background_terminals
        .0
        .get_mut(&execution_id)
        .filter(|terminal| terminal.generation == generation)
}

/// The end `activity` shows: how long it ran, and its last output, plain
/// text (the activity renders it so); the queue keeps its last
/// [`crate::MAXIMUM_WAKE_OUTPUT_BYTES`].
fn ended(
    activity: &AgentActivitySnapshot,
    status: AgentExecutionProgressStatus,
    exit_code: Option<i32>,
) -> AgentTerminalWake {
    let duration_millis = activity.updated_at.saturating_sub(activity.started_at);
    let (output, truncated) = activity
        .output
        .as_ref()
        .map(last_output)
        .unwrap_or_default();
    AgentTerminalWake::Ended {
        status,
        exit_code,
        duration_millis: u64::try_from(duration_millis).unwrap_or(u64::MAX),
        output,
        truncated,
    }
}

/// A preview's lines: its head and its tail, which follow each other unless
/// lines were left out between them; then the tail alone is the last
/// output.
fn last_output(preview: &AgentActivityOutputPreview) -> (String, bool) {
    let cut = preview.omitted_lines > 0 || preview.truncated;
    let lines = if preview.omitted_lines > 0 {
        preview.tail.clone()
    } else {
        [preview.head.as_slice(), preview.tail.as_slice()].concat()
    };
    (lines.join("\n"), cut)
}
