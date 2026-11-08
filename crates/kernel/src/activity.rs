use std::collections::VecDeque;

use hanafubuki_agent_approval::{ApprovalDecision, ApprovalRequest, ApprovalRequestId};
use hanafubuki_agent_foundation::terminal_text::{complete_prefix_len, render_plain_text};

use crate::{
    AgentActivityAction, AgentActivityId, AgentActivityKind, AgentActivityOutputPreview,
    AgentActivityPhase, AgentActivityPresentation, AgentActivitySnapshot,
    AgentContinuationHookOutcome, AgentEventKind, AgentEventSequence, AgentExecutionActivityRef,
    AgentExecutionProgressStatus, AgentToolProgressUpdate,
};

const MAX_RETAINED_ACTIVITIES: usize = 64;
const MAX_RECENT_ACTIVITY_SNAPSHOTS: usize = 32;
const MAX_OUTPUT_HEAD_LINES: usize = 4;
const MAX_OUTPUT_TAIL_LINES: usize = 8;
const MAX_OUTPUT_LINE_BYTES: usize = 512;
/// Raw text of an unfinished line kept before it is collapsed to what it
/// displays, so a progress bar redrawn with `\r` cannot grow it without limit.
const MAX_PENDING_LINE_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
enum ActivitySource {
    Sampling,
    Tool(String),
    Approval(ApprovalRequestId),
    ContinuationHook {
        provider_id: String,
        hook_id: String,
    },
    Execution {
        execution_id: u64,
        generation: u64,
    },
    Restored,
}

#[derive(Clone, Debug)]
struct ActivityRecord {
    source: ActivitySource,
    snapshot: AgentActivitySnapshot,
    line: PendingOutputLine,
}

/// The execution output line still being written. Updates arrive wherever a
/// read stopped, so text after the last newline is kept raw (an escape sequence
/// or `\r` redraw may continue in the next update) and shown, rendered, as a
/// provisional last line that the next update replaces. Runtime-only: a
/// restored activity has no unfinished line.
#[derive(Clone, Debug, Default)]
struct PendingOutputLine {
    raw: String,
    shown: bool,
    /// The committed line showing the provisional one pushed out of the tail.
    evicted: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct AgentActivityTracker {
    activities: VecDeque<ActivityRecord>,
    next_activity_id: u64,
}

impl Default for AgentActivityTracker {
    fn default() -> Self {
        Self {
            activities: VecDeque::new(),
            next_activity_id: 1,
        }
    }
}

impl AgentActivityTracker {
    pub(crate) fn restore(snapshots: Vec<AgentActivitySnapshot>) -> Self {
        let mut tracker = Self::default();
        for snapshot in snapshots.into_iter().take(MAX_RETAINED_ACTIVITIES) {
            tracker.next_activity_id = tracker.next_activity_id.max(
                snapshot
                    .id
                    .value()
                    .checked_add(1)
                    .expect("agent activity id exhausted"),
            );
            let source = snapshot
                .execution
                .as_ref()
                .map(|execution| ActivitySource::Execution {
                    execution_id: execution.execution_id,
                    generation: execution.generation,
                })
                .unwrap_or(ActivitySource::Restored);
            tracker.activities.push_back(ActivityRecord {
                source,
                snapshot,
                line: PendingOutputLine::default(),
            });
        }
        tracker
    }

    pub(crate) fn persisted_snapshots(&self) -> Vec<AgentActivitySnapshot> {
        self.activities
            .iter()
            .map(|record| record.snapshot.clone())
            .collect()
    }

    pub(crate) fn observe(
        &mut self,
        _sequence: AgentEventSequence,
        kind: &AgentEventKind,
        now: u128,
    ) -> Option<AgentActivitySnapshot> {
        match kind {
            AgentEventKind::SamplingStarted { .. } => Some(self.start_sampling(now)),
            AgentEventKind::ResponseStarted { .. } => {
                self.update_sampling(AgentActivityAction::GENERATING_RESPONSE, None, now)
            }
            AgentEventKind::HistoryCommitted { .. } => self.finish(
                |source| matches!(source, ActivitySource::Sampling),
                AgentActivityPhase::Succeeded,
                now,
            ),
            AgentEventKind::ToolStarted {
                call_id,
                presentation,
                ..
            } => Some(self.start(
                ActivitySource::Tool(call_id.clone()),
                AgentActivityKind::ToolInvocation,
                AgentActivityPhase::Running,
                presentation.clone(),
                None,
                now,
            )),
            AgentEventKind::ToolExecutionFinished { call_id, is_error }
            | AgentEventKind::ToolCompleted { call_id, is_error } => self.finish(
                |source| matches!(source, ActivitySource::Tool(id) if id == call_id),
                outcome_phase(*is_error),
                now,
            ),
            AgentEventKind::UserInputRequested { request } => self.update(
                |source| matches!(source, ActivitySource::Tool(id) if id == &request.call_id),
                AgentActivityPhase::WaitingForUserInput,
                Some(AgentActivityAction::REQUEST_USER_INPUT),
                None,
                now,
            ),
            AgentEventKind::UserInputResolved { resolution: _ } => None,
            AgentEventKind::ApprovalRequested { request } => {
                Some(self.start_approval(request, now))
            }
            AgentEventKind::ApprovalResolved {
                request_id,
                resolution,
            } => self.finish(
                |source| matches!(source, ActivitySource::Approval(id) if id == request_id),
                approval_phase(resolution.decision),
                now,
            ),
            AgentEventKind::PermissionReviewStarted { .. }
            | AgentEventKind::PermissionReviewCompleted { .. } => None,
            AgentEventKind::ContinuationHookStarted {
                provider_id,
                hook_id,
            } => Some(self.start_continuation_hook(provider_id, hook_id, now)),
            AgentEventKind::ContinuationHookCompleted {
                provider_id,
                hook_id,
                outcome,
            } => self.finish_continuation_hook(provider_id, hook_id, outcome, now),
            AgentEventKind::ModelRetryScheduled { current_retry, .. } => self.update_sampling(
                AgentActivityAction::RETRYING_MODEL_REQUEST,
                Some(*current_retry),
                now,
            ),
            AgentEventKind::ExecutionProgress { update } => self.observe_execution(update, now),
            AgentEventKind::TurnCompleted { .. } => {
                self.finish_turn_activities(AgentActivityPhase::Succeeded, now);
                None
            }
            AgentEventKind::TurnCancelled { .. } => {
                self.finish_turn_activities(AgentActivityPhase::Cancelled, now);
                None
            }
            AgentEventKind::TurnFailed { .. } => {
                self.finish_turn_activities(AgentActivityPhase::Failed, now);
                None
            }
            AgentEventKind::TurnStarted
            | AgentEventKind::SessionTitleUpdated { .. }
            | AgentEventKind::ReasoningUpdated { .. }
            | AgentEventKind::ReasoningHeadlineUpdated { .. }
            | AgentEventKind::ProposedPlanStarted { .. }
            | AgentEventKind::ProposedPlanDelta { .. }
            | AgentEventKind::ProposedPlanCompleted { .. }
            | AgentEventKind::TaskPlanUpdated { .. }
            | AgentEventKind::AssistantMessageStarted { .. }
            | AgentEventKind::AssistantTextDelta { .. }
            | AgentEventKind::AssistantMessageCompleted { .. }
            | AgentEventKind::SteeringAccepted { .. }
            | AgentEventKind::SteeringConsumed { .. }
            | AgentEventKind::CompactionUpdated { .. } => None,
        }
    }

    fn start_sampling(&mut self, now: u128) -> AgentActivitySnapshot {
        self.start(
            ActivitySource::Sampling,
            AgentActivityKind::Sampling,
            AgentActivityPhase::Running,
            AgentActivityPresentation {
                action: AgentActivityAction::GENERATING_RESPONSE,
                subject: None,
            },
            None,
            now,
        )
    }

    fn update_sampling(
        &mut self,
        action: AgentActivityAction,
        attempt: Option<u32>,
        now: u128,
    ) -> Option<AgentActivitySnapshot> {
        self.update(
            |source| matches!(source, ActivitySource::Sampling),
            AgentActivityPhase::Running,
            Some(action),
            attempt,
            now,
        )
    }

    fn start_approval(&mut self, request: &ApprovalRequest, now: u128) -> AgentActivitySnapshot {
        self.start(
            ActivitySource::Approval(request.request_id),
            AgentActivityKind::Approval,
            AgentActivityPhase::WaitingForApproval,
            AgentActivityPresentation {
                action: AgentActivityAction::WAITING_FOR_APPROVAL,
                subject: Some(request.action.summary.clone()),
            },
            None,
            now,
        )
    }

    fn start_continuation_hook(
        &mut self,
        provider_id: &str,
        hook_id: &str,
        now: u128,
    ) -> AgentActivitySnapshot {
        self.start(
            ActivitySource::ContinuationHook {
                provider_id: provider_id.to_string(),
                hook_id: hook_id.to_string(),
            },
            AgentActivityKind::ContinuationHook,
            AgentActivityPhase::Running,
            AgentActivityPresentation {
                action: AgentActivityAction::CHECKING_CONTINUATION,
                subject: None,
            },
            None,
            now,
        )
    }

    fn finish_continuation_hook(
        &mut self,
        provider_id: &str,
        hook_id: &str,
        outcome: &AgentContinuationHookOutcome,
        now: u128,
    ) -> Option<AgentActivitySnapshot> {
        self.finish(
            |source| {
                matches!(
                    source,
                    ActivitySource::ContinuationHook {
                        provider_id: source_provider,
                        hook_id: source_hook,
                    } if source_provider == provider_id && source_hook == hook_id
                )
            },
            if matches!(
                outcome,
                AgentContinuationHookOutcome::Failed | AgentContinuationHookOutcome::IgnoredFailure
            ) {
                AgentActivityPhase::Failed
            } else {
                AgentActivityPhase::Succeeded
            },
            now,
        )
    }

    pub(crate) fn active_snapshots(&self) -> Vec<AgentActivitySnapshot> {
        self.activities
            .iter()
            .filter(|record| !record.snapshot.phase.is_terminal())
            .map(|record| record.snapshot.clone())
            .collect()
    }

    pub(crate) fn recent_snapshots(&self) -> Vec<AgentActivitySnapshot> {
        let mut recent = self
            .activities
            .iter()
            .rev()
            .filter(|record| record.snapshot.phase.is_terminal())
            .take(MAX_RECENT_ACTIVITY_SNAPSHOTS)
            .map(|record| record.snapshot.clone())
            .collect::<Vec<_>>();
        recent.reverse();
        recent
    }

    fn start(
        &mut self,
        source: ActivitySource,
        kind: AgentActivityKind,
        phase: AgentActivityPhase,
        presentation: AgentActivityPresentation,
        attempt: Option<u32>,
        now: u128,
    ) -> AgentActivitySnapshot {
        let snapshot = AgentActivitySnapshot {
            id: self.allocate_activity_id(),
            revision: 1,
            kind,
            phase,
            presentation,
            attempt,
            started_at: now,
            updated_at: now,
            finished_at: None,
            execution: None,
            output: None,
        };
        self.activities.push_back(ActivityRecord {
            source,
            snapshot: snapshot.clone(),
            line: PendingOutputLine::default(),
        });
        self.trim();
        snapshot
    }

    fn update(
        &mut self,
        matches: impl Fn(&ActivitySource) -> bool,
        phase: AgentActivityPhase,
        action: Option<AgentActivityAction>,
        attempt: Option<u32>,
        now: u128,
    ) -> Option<AgentActivitySnapshot> {
        let record = self
            .activities
            .iter_mut()
            .rev()
            .find(|record| !record.snapshot.phase.is_terminal() && matches(&record.source))?;
        record.snapshot.phase = phase;
        if let Some(action) = action {
            record.snapshot.presentation.action = action;
        }
        record.snapshot.attempt = attempt;
        record.snapshot.revision = record.snapshot.revision.saturating_add(1);
        record.snapshot.updated_at = now;
        Some(record.snapshot.clone())
    }

    fn finish(
        &mut self,
        matches: impl Fn(&ActivitySource) -> bool,
        phase: AgentActivityPhase,
        now: u128,
    ) -> Option<AgentActivitySnapshot> {
        let record = self
            .activities
            .iter_mut()
            .rev()
            .find(|record| !record.snapshot.phase.is_terminal() && matches(&record.source))?;
        record.snapshot.phase = phase;
        record.snapshot.revision = record.snapshot.revision.saturating_add(1);
        record.snapshot.updated_at = now;
        record.snapshot.finished_at = Some(now);
        Some(record.snapshot.clone())
    }

    fn finish_turn_activities(&mut self, phase: AgentActivityPhase, now: u128) {
        for record in &mut self.activities {
            if record.snapshot.phase.is_terminal()
                || matches!(record.source, ActivitySource::Execution { .. })
            {
                continue;
            }
            record.snapshot.phase = phase;
            record.snapshot.revision = record.snapshot.revision.saturating_add(1);
            record.snapshot.updated_at = now;
            record.snapshot.finished_at = Some(now);
        }
        self.trim();
    }

    fn observe_execution(
        &mut self,
        update: &AgentToolProgressUpdate,
        now: u128,
    ) -> Option<AgentActivitySnapshot> {
        let changed = match update {
            AgentToolProgressUpdate::ExecutionStarted {
                execution_id,
                generation,
                action,
                status,
            } => {
                if self.execution_index(*execution_id, *generation).is_some() {
                    return None;
                }
                self.start_execution(*execution_id, *generation, action.clone(), *status, now)
            }
            AgentToolProgressUpdate::ExecutionOutput {
                execution_id,
                generation,
                text,
                next_offset,
                truncated,
            } => {
                let record = self.execution_record(*execution_id, *generation)?;
                append_output(
                    record
                        .snapshot
                        .output
                        .get_or_insert_with(AgentActivityOutputPreview::default),
                    &mut record.line,
                    text,
                    *next_offset,
                    *truncated,
                );
                touch(&mut record.snapshot, now);
                record.snapshot.clone()
            }
            AgentToolProgressUpdate::ExecutionStatus {
                execution_id,
                generation,
                status,
                exit_code,
                cleanup_required,
            } => {
                let record = self.execution_record(*execution_id, *generation)?;
                if let Some(execution) = record.snapshot.execution.as_mut() {
                    execution.status = *status;
                    execution.exit_code = *exit_code;
                }
                if status.is_terminal() {
                    // No more output follows: the unfinished line is final.
                    record.line = PendingOutputLine::default();
                }
                if status.is_terminal() && *cleanup_required {
                    record.snapshot.phase = AgentActivityPhase::WaitingForCleanup;
                }
                touch(&mut record.snapshot, now);
                record.snapshot.clone()
            }
            AgentToolProgressUpdate::ExecutionDisposed {
                execution_id,
                generation,
            } => {
                let record = self.execution_record(*execution_id, *generation)?;
                record.snapshot.phase = execution_terminal_phase(&record.snapshot);
                record.snapshot.finished_at = Some(now);
                touch(&mut record.snapshot, now);
                record.snapshot.clone()
            }
            AgentToolProgressUpdate::ExecutionCleanupFailed {
                execution_id,
                generation,
            } => {
                let record = self.execution_record(*execution_id, *generation)?;
                record.snapshot.phase = AgentActivityPhase::WaitingForCleanup;
                touch(&mut record.snapshot, now);
                record.snapshot.clone()
            }
        };
        self.trim();
        Some(changed)
    }

    /// Record a new execution activity. The caller trims retained activities.
    fn start_execution(
        &mut self,
        execution_id: u64,
        generation: u64,
        action: AgentActivityAction,
        status: AgentExecutionProgressStatus,
        now: u128,
    ) -> AgentActivitySnapshot {
        let snapshot = AgentActivitySnapshot {
            id: self.allocate_activity_id(),
            revision: 1,
            kind: AgentActivityKind::Execution,
            phase: AgentActivityPhase::Running,
            presentation: AgentActivityPresentation {
                action,
                subject: None,
            },
            attempt: None,
            started_at: now,
            updated_at: now,
            finished_at: None,
            execution: Some(AgentExecutionActivityRef {
                execution_id,
                generation,
                status,
                exit_code: None,
            }),
            output: Some(AgentActivityOutputPreview::default()),
        };
        self.activities.push_back(ActivityRecord {
            source: ActivitySource::Execution {
                execution_id,
                generation,
            },
            snapshot: snapshot.clone(),
            line: PendingOutputLine::default(),
        });
        snapshot
    }

    fn execution_record(
        &mut self,
        execution_id: u64,
        generation: u64,
    ) -> Option<&mut ActivityRecord> {
        let index = self.execution_index(execution_id, generation)?;
        Some(&mut self.activities[index])
    }

    fn allocate_activity_id(&mut self) -> AgentActivityId {
        let id = AgentActivityId::new(self.next_activity_id);
        self.next_activity_id = self
            .next_activity_id
            .checked_add(1)
            .expect("agent activity id exhausted");
        id
    }

    fn execution_index(&self, execution_id: u64, generation: u64) -> Option<usize> {
        self.activities.iter().position(|record| {
            !record.snapshot.phase.is_terminal()
                && matches!(
                    record.source,
                    ActivitySource::Execution {
                        execution_id: current_id,
                        generation: current_generation,
                    } if current_id == execution_id && current_generation == generation
                )
        })
    }

    fn trim(&mut self) {
        while self.activities.len() > MAX_RETAINED_ACTIVITIES {
            let removable = self
                .activities
                .iter()
                .position(|record| record.snapshot.phase.is_terminal());
            let Some(index) = removable else {
                break;
            };
            self.activities.remove(index);
        }
    }
}

fn outcome_phase(is_error: bool) -> AgentActivityPhase {
    if is_error {
        AgentActivityPhase::Failed
    } else {
        AgentActivityPhase::Succeeded
    }
}

fn approval_phase(decision: ApprovalDecision) -> AgentActivityPhase {
    match decision {
        ApprovalDecision::AllowOnce | ApprovalDecision::AllowForSession => {
            AgentActivityPhase::Succeeded
        }
        ApprovalDecision::Aborted => AgentActivityPhase::Cancelled,
        ApprovalDecision::Deny | ApprovalDecision::TimedOut => AgentActivityPhase::Failed,
    }
}

fn touch(snapshot: &mut AgentActivitySnapshot, now: u128) {
    snapshot.revision = snapshot.revision.saturating_add(1);
    snapshot.updated_at = now;
}

/// Add an update's text as the plain text a terminal would show: only lines
/// ended by a newline are committed, and the line still being written is shown
/// provisionally (see [`PendingOutputLine`]).
fn append_output(
    output: &mut AgentActivityOutputPreview,
    line: &mut PendingOutputLine,
    text: &str,
    next_offset: u64,
    truncated: bool,
) {
    if std::mem::take(&mut line.shown) {
        if output.tail.pop().is_none() {
            output.head.pop();
        }
        if let Some(evicted) = line.evicted.take() {
            output.tail.insert(0, evicted);
            output.omitted_lines = output.omitted_lines.saturating_sub(1);
        }
    }
    if truncated && !line.raw.is_empty() {
        // Output was skipped after this line, so it can never be finished.
        let raw = std::mem::take(&mut line.raw);
        commit_output_line(output, &render_plain_text(raw.as_bytes()));
    }
    line.raw.push_str(text);
    if let Some(end) = line.raw.rfind('\n') {
        let complete: String = line.raw.drain(..=end).collect();
        for rendered in render_plain_text(complete.as_bytes()).lines() {
            commit_output_line(output, rendered);
        }
    }
    let complete = complete_prefix_len(&line.raw);
    if line.raw.len() > MAX_PENDING_LINE_BYTES {
        let collapsed = render_plain_text(line.raw[..complete].as_bytes());
        line.raw.replace_range(..complete, &collapsed);
    }
    let shown = render_plain_text(line.raw[..complete_prefix_len(&line.raw)].as_bytes());
    if !shown.is_empty() {
        line.evicted = commit_output_line(output, &shown);
        line.shown = true;
    }
    output.next_offset = next_offset;
    output.truncated |= truncated;
}

/// Append one line, returning the line it pushed out of the tail.
fn commit_output_line(output: &mut AgentActivityOutputPreview, line: &str) -> Option<String> {
    let line = truncate_utf8(line, MAX_OUTPUT_LINE_BYTES);
    if output.head.len() < MAX_OUTPUT_HEAD_LINES {
        output.head.push(line);
        return None;
    }
    let evicted = (output.tail.len() == MAX_OUTPUT_TAIL_LINES).then(|| {
        output.omitted_lines = output.omitted_lines.saturating_add(1);
        output.tail.remove(0)
    });
    output.tail.push(line);
    evicted
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn execution_terminal_phase(snapshot: &AgentActivitySnapshot) -> AgentActivityPhase {
    match snapshot.execution.as_ref() {
        Some(execution)
            if execution.status == AgentExecutionProgressStatus::Exited
                && execution.exit_code == Some(0) =>
        {
            AgentActivityPhase::Succeeded
        }
        Some(execution) if execution.status == AgentExecutionProgressStatus::Cancelled => {
            AgentActivityPhase::Cancelled
        }
        Some(execution) if execution.status.is_terminal() => AgentActivityPhase::Failed,
        _ => AgentActivityPhase::Interrupted,
    }
}

#[cfg(test)]
mod tests;
