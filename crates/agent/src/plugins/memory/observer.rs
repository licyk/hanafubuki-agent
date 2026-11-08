use hanafubuki_agent_contracts::{AgentMemoryContributionCause, AgentNotification, AgentObserver};

use crate::AgentMemoryApplication;

/// Signals the memory supervisor about finished turns, committed citations,
/// the user's contribution toggle, sessions that stop contributing and
/// archived or restored sessions, and
/// records a session's contribution state in the memory store.
///
/// Signals only enqueue. The contribution state is one bounded upsert in the
/// memory store, the same write the user's own toggle makes, so it reflects
/// the change before the committing call returns.
pub(super) struct MemoryObserver {
    pub(super) memory: AgentMemoryApplication,
}

impl AgentObserver for MemoryObserver {
    fn id(&self) -> &str {
        "memory"
    }

    fn observe(&self, notification: &AgentNotification) {
        match notification {
            // Memory is derived from committed history, so the scan waits
            // for the turn to end instead of competing with it for the
            // model. The session itself stays excluded while it is in use.
            AgentNotification::TurnFinished { session_id, .. } => {
                self.memory.signal_scan(Some(*session_id));
            }
            AgentNotification::SamplingCommitted {
                memory_citations, ..
            } => {
                if memory_citations.is_empty() {
                    return;
                }
                if let Err(error) = self.memory.record_citation_usage(memory_citations) {
                    log::warn!(
                        "Agent memory citation usage update was not queued after durable answer commit: {error}"
                    );
                }
            }
            AgentNotification::MemoryContributionChanged {
                session_id,
                state,
                cause,
                revision,
            } => {
                if let Err(error) =
                    self.memory
                        .sync_session_contribution(*session_id, *state, revision.value())
                {
                    log::warn!(
                        "Agent memory contribution of session {session_id} was not recorded in the memory store: {error}"
                    );
                }
                // A source that stops contributing must leave the published
                // memory soon, and the user's toggle asks for a scan either
                // way. A new or forked session that starts contributing is
                // picked up once it is idle, by the scans later turns signal:
                // a scan now would compete with its first turn for the model.
                if !state.mode.contributes() || *cause == AgentMemoryContributionCause::User {
                    self.memory.signal_scan(None);
                }
            }
            // Archived sessions are not memory sources; restoring one makes
            // it a source again.
            AgentNotification::SessionArchived { .. }
            | AgentNotification::SessionRestored { .. } => {
                self.memory.signal_scan(None);
            }
            // A turn's history is read once it has finished.
            AgentNotification::TurnAdmitted { .. } => {}
        }
    }
}
