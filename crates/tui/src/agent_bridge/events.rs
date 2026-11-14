//! The Agent's events of the session shown, onto what the chat widget
//! shows ([`TurnUpdate`]), and the sinks that bring them (and the Agent
//! slot's changes) into the app loop.

use hanafubuki_agent::{
    AgentCompactionStatus, AgentCompactionTrigger, AgentEvent, AgentEventKind, AgentEventSink,
    AgentStatusEventSink, AgentToolProgressUpdate, AgentTurnAccepted, AgentTurnTerminalReason,
};

use super::{AgentBridge, TurnEnd, TurnUpdate};
use crate::app_event::{AppEvent, AppEventSender};

impl AgentBridge {
    /// What the chat widget makes of one of the Agent's events, if anything.
    pub(crate) fn event(&mut self, event: AgentEvent) -> Option<TurnUpdate> {
        if Some(event.session_id) != self.session_id {
            return None;
        }
        self.denials.observe(event.session_id, &event.kind);
        self.adopt_followed_turn(&event.kind, event.turn_id);
        if let AgentEventKind::CompactionUpdated { compaction } = &event.kind {
            if compaction.status.is_terminal() {
                self.retry_missed_wake();
            }
        }
        // The turn's events can arrive before the reply that accepted it.
        if let (None, Some(turn_id), true) = (self.turn, event.turn_id, self.turn_running) {
            self.turn = Some(AgentTurnAccepted {
                session_id: event.session_id,
                turn_id,
                generation: event.generation,
            });
        }
        let update = match event.kind {
            AgentEventKind::TurnStarted { .. } => TurnUpdate::Started,
            AgentEventKind::ReasoningStarted { reasoning_id, .. } => {
                TurnUpdate::ReasoningStarted(reasoning_id)
            }
            AgentEventKind::ReasoningDelta {
                reasoning_id,
                delta,
            } => TurnUpdate::ReasoningDelta {
                reasoning_id,
                delta,
            },
            AgentEventKind::ReasoningCompleted {
                reasoning_id,
                duration_millis,
            } => TurnUpdate::ReasoningCompleted {
                reasoning_id,
                duration_millis,
            },
            AgentEventKind::ReasoningUpdated { reasoning } => TurnUpdate::Reasoning(reasoning),
            AgentEventKind::AssistantTextDelta { delta, .. } => TurnUpdate::MessageDelta(delta),
            AgentEventKind::AssistantMessageCompleted { .. } => TurnUpdate::MessageCompleted,
            AgentEventKind::ProposedPlanDelta { delta, .. } => TurnUpdate::PlanDelta(delta),
            AgentEventKind::ProposedPlanCompleted { .. } => TurnUpdate::PlanCompleted,
            AgentEventKind::ToolStarted {
                call_id,
                name,
                presentation,
            } => TurnUpdate::ToolStarted {
                call_id,
                name,
                subject: presentation.subject,
            },
            AgentEventKind::ToolCompleted { call_id, is_error } => {
                self.read_tool_display(event.session_id, call_id.clone());
                TurnUpdate::ToolCompleted { call_id, is_error }
            }
            // Only output names its call; a process the Agent reports itself
            // has none.
            AgentEventKind::ExecutionProgress {
                call_id: Some(call_id),
                update:
                    AgentToolProgressUpdate::ExecutionOutput {
                        text, truncated, ..
                    },
            } => TurnUpdate::ToolOutput {
                call_id,
                text,
                truncated,
            },
            AgentEventKind::ApprovalRequested { request } => TurnUpdate::ApprovalRequested(request),
            AgentEventKind::ApprovalResolved { request_id, .. } => {
                TurnUpdate::ApprovalResolved(request_id)
            }
            AgentEventKind::UserInputRequested { request } => {
                TurnUpdate::UserInputRequested(request)
            }
            AgentEventKind::UserInputResolved { resolution } => {
                TurnUpdate::UserInputResolved(resolution.request_id)
            }
            AgentEventKind::TaskPlanUpdated { plan } => {
                self.send_session_status();
                TurnUpdate::TaskPlan(plan)
            }
            AgentEventKind::SteeringConsumed {
                through_sequence, ..
            } => TurnUpdate::SteeringConsumed(through_sequence.value()),
            // Every install, `/compact`'s included, is shown from its event.
            AgentEventKind::CompactionUpdated { compaction }
                if compaction.status == AgentCompactionStatus::Completed =>
            {
                self.send_session_status();
                TurnUpdate::Compacted
            }
            // `/compact`'s failure is its reply; a cancelled one is no failure.
            AgentEventKind::CompactionUpdated { compaction }
                if compaction.status == AgentCompactionStatus::Failed
                    && compaction.trigger == AgentCompactionTrigger::Automatic =>
            {
                self.send_session_status();
                TurnUpdate::CompactionFailed(compaction.last_error.unwrap_or_default())
            }
            AgentEventKind::ContextCompressionFailed { message } => {
                TurnUpdate::CompressionFailed(message)
            }
            AgentEventKind::ModelContextWindowLearned { tokens, .. } => {
                self.send_session_status();
                TurnUpdate::ContextWindowLearned(tokens)
            }
            // The `compress` call's cell shows the fold, and the commit's
            // `HistoryCommitted` already refreshed the status line.
            AgentEventKind::ContextCompressed { .. } => return None,
            AgentEventKind::HistoryCommitted { .. }
            | AgentEventKind::SessionTitleUpdated { .. }
            | AgentEventKind::CompactionUpdated { .. } => return self.session_status(),
            AgentEventKind::SessionGoalUpdated { goal } => return self.goal_updated(goal),
            AgentEventKind::ModelRetryScheduled {
                current_retry,
                maximum_retries,
                reason,
                ..
            } => TurnUpdate::Retry(format!(
                "Reconnecting {current_retry}/{maximum_retries}: {reason}"
            )),
            AgentEventKind::TurnCompleted { .. } => self.ended(TurnEnd::Completed),
            AgentEventKind::TurnCancelled {
                reason: AgentTurnTerminalReason::UserInterrupted,
            } => self.ended(TurnEnd::Interrupted),
            AgentEventKind::TurnCancelled { reason } => self.ended(TurnEnd::Failed(format!(
                "the turn was cancelled ({reason:?})"
            ))),
            AgentEventKind::TurnFailed { message, .. } => self.ended(TurnEnd::Failed(message)),
            // A background terminal's end changes what a waiting session
            // waits on.
            AgentEventKind::ExecutionProgress {
                update: AgentToolProgressUpdate::ExecutionStatus { status, .. },
                ..
            } if status.is_terminal() && !self.turn_running => return self.session_status(),
            _ => return None,
        };
        Some(update)
    }
}

/// The Agent's events into the app loop: `emit` must not block.
pub(super) struct EventSink(pub(super) AppEventSender);

impl AgentEventSink for EventSink {
    fn emit(&self, event: AgentEvent) {
        self.0.send(AppEvent::Agent(Box::new(event)));
    }
}

pub(super) struct DiscardSink;

impl AgentEventSink for DiscardSink {
    fn emit(&self, _event: AgentEvent) {}
}

/// The Agent slot's changes into the app loop.
pub(crate) struct StatusSink(pub(crate) AppEventSender);

impl AgentStatusEventSink for StatusSink {
    fn status_changed(&self) {
        self.0.send(AppEvent::AgentStatusChanged);
    }
}
