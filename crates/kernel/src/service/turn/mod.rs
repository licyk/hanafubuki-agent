use super::*;

mod admission;
mod cancellation;
mod continuation;
mod loop_driver;
mod state;
mod terminal;
mod worker;

fn terminal_reason_for_error(error: &AgentError) -> AgentTurnTerminalReason {
    match error {
        AgentError::ModelRequestRetriesExhausted { .. } => {
            AgentTurnTerminalReason::ModelRequestRetriesExhausted
        }
        AgentError::ModelStreamRetriesExhausted { .. } => {
            AgentTurnTerminalReason::ModelStreamRetriesExhausted
        }
        AgentError::Protocol(_) => AgentTurnTerminalReason::ModelProtocol,
        AgentError::LimitExceeded { .. }
        | AgentError::ContextBudgetExceeded { .. }
        | AgentError::CompactionDidNotReduceContext { .. } => {
            AgentTurnTerminalReason::BudgetExceeded
        }
        AgentError::TurnDeadlineExceeded { .. } => AgentTurnTerminalReason::DeadlineExceeded,
        AgentError::ContextWindowExceededAfterCompaction { .. } => {
            AgentTurnTerminalReason::ContextWindowExceeded
        }
        AgentError::Model(_)
        | AgentError::ModelStreamInterrupted(_)
        | AgentError::ProviderContextWindowExceeded { .. }
        | AgentError::UnsupportedImageInput { .. }
        | AgentError::UnsupportedReasoning { .. } => AgentTurnTerminalReason::ModelError,
        _ => AgentTurnTerminalReason::InternalFailure,
    }
}

impl Drop for TurnExecutionGuard {
    fn drop(&mut self) {
        if self.armed {
            self.service.abort_pending_approvals(self.identity);
            self.service
                .finish_cancelled_on_drop(self.identity, self.event_sink.clone());
        }
    }
}

struct ManagedTurnTaskGuard {
    service: AgentService,
    identity: TurnIdentity,
}

impl Drop for ManagedTurnTaskGuard {
    fn drop(&mut self) {
        self.service
            .inner
            .managed_turns
            .lock()
            .remove(&self.identity);
        self.service.inner.managed_turns_changed.notify_all();
    }
}
