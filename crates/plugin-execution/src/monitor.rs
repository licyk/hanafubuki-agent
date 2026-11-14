use std::sync::Arc;

use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentExecutionProgressStatus, AgentSessionId, AgentToolProgressSink,
    AgentToolProgressUpdate,
};
use hanafubuki_agent_execution::{
    AgentExecutionError, AgentExecutionGeneration, AgentExecutionId, AgentExecutionOutput,
    AgentExecutionOutputChunk, AgentExecutionOutputStream, AgentExecutionScope,
    AgentExecutionService, AgentExecutionSnapshot, AgentExecutionStatus, TerminalScreen,
};
use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_foundation::terminal_text::{
    aligned_start, resume_point, styled_terminal_text, ALIGN_SEARCH_BYTES,
};
use std::time::Duration;

/// The activity an execution reports into: its session (for worker names)
/// and the turn's progress sink.
#[derive(Clone)]
pub(crate) struct ExecutionActivity {
    pub(crate) session_id: AgentSessionId,
    pub(crate) progress: Arc<dyn AgentToolProgressSink>,
}

/// Output one progress update carries. A monitor further behind than this
/// skips to the newest output and reports the gap as truncated, since the
/// live preview shows only the newest lines.
const MONITOR_READ_BYTES: usize = 8 * 1024;
const MONITOR_POLL: Duration = Duration::from_millis(100);

pub(crate) fn start_execution_activity(
    executions: &AgentExecutionService,
    workers: &BackgroundWorkerGroup,
    activity: &ExecutionActivity,
    scope: AgentExecutionScope,
    snapshot: &AgentExecutionSnapshot,
    action: AgentActivityAction,
) -> Result<(), AgentExecutionError> {
    activity
        .progress
        .report(AgentToolProgressUpdate::ExecutionStarted {
            execution_id: snapshot.execution_id.value(),
            generation: snapshot.generation.value(),
            action,
            status: execution_progress_status(snapshot.status),
        });
    report_execution_status(activity, snapshot);

    let monitor_executions = executions.clone();
    let monitor_scope = scope.clone();
    let progress = activity.progress.clone();
    let execution_id = snapshot.execution_id;
    let generation = snapshot.generation;
    // Attached before the monitor runs, so an execution that finishes at once
    // still waits for the monitor to report its output before disposal.
    executions.attach_observer(&scope, execution_id, generation)?;
    let spawn = workers.spawn(
        format!(
            "agent-execution-monitor-{}-{}-{}",
            activity.session_id,
            execution_id.value(),
            generation.value()
        ),
        move |cancellation| {
            monitor_execution(
                &monitor_executions,
                &monitor_scope,
                execution_id,
                generation,
                progress.as_ref(),
                cancellation,
            );
        },
    );
    if let Err(error) = spawn {
        executions.release_observer(&scope, execution_id, generation);
        let cleanup = (|| {
            let snapshot = executions.snapshot(&scope, execution_id)?;
            if !snapshot.status.is_terminal() {
                executions.terminate(&scope, execution_id)?;
            }
            executions.dispose(&scope, execution_id)
        })();
        match cleanup {
            Ok(_) | Err(AgentExecutionError::NotFound) => {
                activity
                    .progress
                    .report(AgentToolProgressUpdate::ExecutionDisposed {
                        execution_id: execution_id.value(),
                        generation: generation.value(),
                    });
                return Err(AgentExecutionError::Process(error));
            }
            Err(cleanup_error) => {
                activity
                    .progress
                    .report(AgentToolProgressUpdate::ExecutionCleanupFailed {
                        execution_id: execution_id.value(),
                        generation: generation.value(),
                    });
                return Err(AgentExecutionError::CleanupRetained {
                    execution_id,
                    message: format!("{error}; cleanup failed: {cleanup_error}"),
                });
            }
        }
    }
    Ok(())
}

pub(crate) fn schedule_execution_cleanup_retry(
    executions: &AgentExecutionService,
    workers: &BackgroundWorkerGroup,
    activity: &ExecutionActivity,
    scope: AgentExecutionScope,
    snapshot: &AgentExecutionSnapshot,
    action: AgentActivityAction,
    announce_execution: bool,
) {
    if announce_execution {
        if let Err(error) = start_execution_activity(
            executions,
            workers,
            activity,
            scope.clone(),
            snapshot,
            action,
        ) {
            log::warn!(
                "agent_diag stage=unified_exec.monitor execution_id={} outcome=error cleanup_obligation={}",
                snapshot.execution_id,
                matches!(error, AgentExecutionError::CleanupRetained { .. })
            );
        }
    } else {
        activity
            .progress
            .report(AgentToolProgressUpdate::ExecutionCleanupFailed {
                execution_id: snapshot.execution_id.value(),
                generation: snapshot.generation.value(),
            });
    }

    let executions = executions.clone();
    let progress = activity.progress.clone();
    let execution_id = snapshot.execution_id;
    let generation = snapshot.generation;
    let worker_name = format!(
        "agent-execution-cleanup-{}-{}-{}",
        activity.session_id,
        execution_id.value(),
        generation.value()
    );
    if workers
        .spawn(worker_name, move |cancellation| loop {
            if cancellation.wait_timeout(Duration::from_secs(1)) {
                return;
            }
            match executions.dispose(&scope, execution_id) {
                Ok(_) | Err(AgentExecutionError::NotFound) => {
                    progress.report(AgentToolProgressUpdate::ExecutionDisposed {
                        execution_id: execution_id.value(),
                        generation: generation.value(),
                    });
                    log::info!(
                        "agent_diag stage=unified_exec.cleanup_retry execution_id={} cleanup_obligation=false outcome=disposed",
                        execution_id
                    );
                    return;
                }
                Err(_) => {
                    progress.report(AgentToolProgressUpdate::ExecutionCleanupFailed {
                        execution_id: execution_id.value(),
                        generation: generation.value(),
                    });
                    log::warn!(
                        "agent_diag stage=unified_exec.cleanup_retry execution_id={} cleanup_obligation=true outcome=retained",
                        execution_id
                    );
                }
            }
        })
        .is_err()
    {
        log::warn!(
            "agent_diag stage=unified_exec.cleanup_retry execution_id={} cleanup_obligation=true outcome=worker_rejected",
            execution_id
        );
    }
}

/// Stream an execution's output and status into its activity until it ends.
///
/// The monitor is the execution's observer: it holds the execution's disposal
/// until it has reported the output produced before the terminal status, and
/// that status, so the preview never loses the end of a command.
pub(crate) fn monitor_execution(
    executions: &AgentExecutionService,
    scope: &AgentExecutionScope,
    execution_id: AgentExecutionId,
    generation: AgentExecutionGeneration,
    progress: &dyn AgentToolProgressSink,
    cancellation: CancellationToken,
) {
    struct Release<'a> {
        executions: &'a AgentExecutionService,
        scope: &'a AgentExecutionScope,
        execution_id: AgentExecutionId,
        generation: AgentExecutionGeneration,
    }
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            self.executions
                .release_observer(self.scope, self.execution_id, self.generation);
        }
    }
    let _release = Release {
        executions,
        scope,
        execution_id,
        generation,
    };

    let mut offset = 0;
    let mut last_status = None;
    // A PTY's output as its screen shows it, from the last skip on.
    let mut screen: Option<TerminalScreen> = None;
    while !cancellation.is_cancelled() {
        if executions
            .ensure_generation(scope, execution_id, generation)
            .is_err()
        {
            return;
        }
        // The status is read first and one read reaches its offset (the skip
        // bounds what is left), so a terminal status follows all its output.
        let Ok(snapshot) = executions.snapshot(scope, execution_id) else {
            return;
        };
        let skip_to = snapshot
            .output_offset
            .saturating_sub(MONITOR_READ_BYTES as u64);
        let skipped = skip_to > offset;
        if snapshot.output_offset > offset {
            if let Ok(mut output) = executions.read_output(
                scope,
                execution_id,
                Some(offset.max(skip_to)),
                Some(MONITOR_READ_BYTES),
            ) {
                if skipped {
                    output.truncated = true;
                    skip_to_aligned_start(&mut output);
                    screen = None;
                }
                if screen.is_none() {
                    screen = TerminalScreen::for_output(&output);
                }
                if let Some(screen) = screen.as_mut() {
                    render_screen(screen, &mut output);
                }
                report_execution_output_to(progress, generation, &output);
                offset = output.next_offset;
            }
        }
        let current = (
            snapshot.status,
            snapshot.exit_code,
            snapshot.cleanup_required,
        );
        if last_status != Some(current) {
            report_execution_status_to(progress, &snapshot);
            last_status = Some(current);
        }
        if snapshot.status.is_terminal() {
            return;
        }
        if cancellation.wait_timeout(MONITOR_POLL) {
            return;
        }
    }
}

/// Replace a PTY's `output` with what its `screen` shows of it, as terminal
/// text the preview renders: each styled run of a row between its SGR
/// sequence and a reset, so no style outlives its row; a row the previous
/// update left unfinished is cleared and written again.
fn render_screen(screen: &mut TerminalScreen, output: &mut AgentExecutionOutput) {
    screen.push_output(output);
    let taken = screen.take();
    let rows = styled_terminal_text(&taken.text, &taken.styles);
    let text = if taken.rewrites_last_line {
        format!("\r\x1b[2K{rows}")
    } else {
        rows
    };
    output.chunks = vec![AgentExecutionOutputChunk {
        stream: AgentExecutionOutputStream::Merged,
        text,
    }];
}

/// Drop what precedes the first place `output`, read from a skipped-to
/// offset, renders from (see [`resume_point`]).
fn skip_to_aligned_start(output: &mut AgentExecutionOutput) {
    let mut skip = {
        let start = output
            .chunks
            .iter()
            .flat_map(|chunk| chunk.text.as_bytes())
            .take(ALIGN_SEARCH_BYTES)
            .copied()
            .collect::<Vec<_>>();
        resume_point(&start)
    };
    for chunk in &mut output.chunks {
        let drop = skip.min(chunk.text.len());
        chunk.text.drain(..drop);
        skip -= drop;
    }
    output.chunks.retain(|chunk| !chunk.text.is_empty());
}

pub(crate) fn report_execution_output_to(
    progress: &dyn AgentToolProgressSink,
    generation: AgentExecutionGeneration,
    output: &AgentExecutionOutput,
) {
    let combined = output
        .chunks
        .iter()
        .map(|chunk| chunk.text.as_str())
        .collect::<String>();
    if combined.is_empty() && !output.truncated {
        return;
    }
    let text = bound_execution_output(&combined);
    progress.report(AgentToolProgressUpdate::ExecutionOutput {
        execution_id: output.execution_id.value(),
        generation: generation.value(),
        truncated: output.truncated || text.len() < combined.len(),
        text,
        next_offset: output.next_offset,
    });
}

pub(crate) fn report_execution_status(
    activity: &ExecutionActivity,
    snapshot: &AgentExecutionSnapshot,
) {
    report_execution_status_to(activity.progress.as_ref(), snapshot);
}

pub(crate) fn report_execution_status_to(
    progress: &dyn AgentToolProgressSink,
    snapshot: &AgentExecutionSnapshot,
) {
    progress.report(AgentToolProgressUpdate::ExecutionStatus {
        execution_id: snapshot.execution_id.value(),
        generation: snapshot.generation.value(),
        status: execution_progress_status(snapshot.status),
        exit_code: snapshot.exit_code,
        cleanup_required: snapshot.cleanup_required,
    });
}

pub(crate) fn execution_progress_status(
    status: AgentExecutionStatus,
) -> AgentExecutionProgressStatus {
    match status {
        AgentExecutionStatus::Starting => AgentExecutionProgressStatus::Starting,
        AgentExecutionStatus::Running => AgentExecutionProgressStatus::Running,
        AgentExecutionStatus::Exited => AgentExecutionProgressStatus::Exited,
        AgentExecutionStatus::Failed => AgentExecutionProgressStatus::Failed,
        AgentExecutionStatus::Cancelled => AgentExecutionProgressStatus::Cancelled,
        AgentExecutionStatus::TimedOut => AgentExecutionProgressStatus::TimedOut,
    }
}

/// Keep the newest `MAX_BYTES` of an update: the text ends at `next_offset`,
/// so its tail is what the preview continues from.
pub(crate) fn bound_execution_output(value: &str) -> String {
    const MAX_BYTES: usize = 8 * 1024;
    if value.len() <= MAX_BYTES {
        return value.to_owned();
    }
    let start = aligned_start(value.as_bytes(), value.len() - MAX_BYTES);
    value[start..].to_owned()
}

#[cfg(test)]
mod tests;
