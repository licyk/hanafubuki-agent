//! Observers: plugins told what the kernel has durably committed.
//!
//! An observer is notified after the commit it describes is durable and its
//! UI events are emitted. It cannot change the outcome of anything: it has no
//! return value, and the kernel has already moved on. `observe` runs on the
//! thread that committed (often a turn's async task, or the history store's
//! writer for the terminal commit of a dropped turn), so it does bounded,
//! non-blocking work: it signals a worker or performs a small local update.
//!
//! The notifications are the ones a plugin consumes today; a new one is added
//! with its first consumer.

mod registry;

use crate::{
    AgentGeneration, AgentMemoryCitationSnapshot, AgentMemoryContributionCause,
    AgentMemoryContributionState, AgentRevision, AgentSessionId, AgentTurnId, AgentTurnOrigin,
};

pub use registry::ObserverRegistry;

/// What the kernel committed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentNotification {
    /// A turn was admitted: its start and its user message are committed,
    /// and its admission events are emitted. A failed admission is not
    /// notified. `origin` is who asked for the turn; `automatic_title` is
    /// the session's title after the admission when the Agent gave it (the
    /// provisional title the admission committed because the session had
    /// none, or one committed before); `None` when the session has a title
    /// the user gave it, or none.
    TurnAdmitted {
        session_id: AgentSessionId,
        turn_id: AgentTurnId,
        generation: AgentGeneration,
        origin: AgentTurnOrigin,
        automatic_title: Option<String>,
    },
    /// A turn ended (completed, cancelled or failed) and its terminal state
    /// is committed. Turns interrupted by a restart are settled during
    /// recovery, before any plugin runs, and are not notified.
    TurnFinished {
        session_id: AgentSessionId,
        turn_id: AgentTurnId,
        generation: AgentGeneration,
    },
    /// One sampling response was committed to the session's history.
    /// `memory_citations` are the validated citations of its answers.
    SamplingCommitted {
        session_id: AgentSessionId,
        turn_id: AgentTurnId,
        generation: AgentGeneration,
        memory_citations: Vec<AgentMemoryCitationSnapshot>,
    },
    /// The session's memory contribution changed for `cause`; `revision`
    /// is the session revision that holds it.
    MemoryContributionChanged {
        session_id: AgentSessionId,
        state: AgentMemoryContributionState,
        cause: AgentMemoryContributionCause,
        revision: AgentRevision,
    },
    /// The session was moved into the archive; `revision` is the session
    /// revision that holds it.
    SessionArchived {
        session_id: AgentSessionId,
        revision: AgentRevision,
    },
    /// The session was moved out of the archive; `revision` is the session
    /// revision that holds it.
    SessionRestored {
        session_id: AgentSessionId,
        revision: AgentRevision,
    },
    /// An event was queued on the session's wake queue (one dropped with
    /// the policy off is not notified).
    WakeEventQueued { session_id: AgentSessionId },
    /// The session's terminal lifecycle changed, or events left its wake
    /// queue: a turn delivered them (at its
    /// admission or at a step), or they were taken (`wait_agent`'s
    /// statuses, a follow-up's withdrawal).
    BackgroundWorkChanged { session_id: AgentSessionId },
}

/// Receives the kernel's notifications.
///
/// `observe` must not block or wait for the kernel: it may be called while
/// the kernel finishes the operation that produced the notification.
pub trait AgentObserver: Send + Sync + 'static {
    /// Unique within the registry.
    fn id(&self) -> &str;

    fn observe(&self, notification: &AgentNotification);
}
