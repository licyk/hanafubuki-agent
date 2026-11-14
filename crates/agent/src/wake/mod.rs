//! Wake: a session that ended its turn while its background work runs is
//! resumed by itself when that work reports. A sub-agent's end or message
//! is queued on its parent's durable wake queue (the kernel's); the
//! dispatcher (`dispatcher.rs`), the observer of the core row `agent-wake`,
//! sees an event queued on an idle session, and a turn that ended with
//! events left, and wakes the session. A root is woken by its host: it is
//! told [`AgentBackgroundEvent::Woken`] and submits the woken turn
//! (`AgentApplicationService::submit_wake_turn_with_sink`), with a sink of
//! its choice, or leaves the events for the session's next turn. A person's
//! interrupt of a root's turn, or more than [`MAXIMUM_WOKEN_TURNS`] woken
//! turns since the person's last message, pauses its wakes until their next
//! message (`pause.rs`).

mod dispatcher;
mod pause;

use std::sync::Arc;

use hanafubuki_agent_kernel::AgentSessionId;

pub(crate) use self::dispatcher::wake_plugin;
pub(crate) use self::pause::Pauses;
pub use self::pause::{AgentWakePause, AgentWakeState, MAXIMUM_WOKEN_TURNS};

/// What the Agent tells its host about the sessions' background work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentBackgroundEvent {
    /// A root has wake events and no turn: `submit_wake_turn_with_sink`
    /// starts the turn that reads them. Ignoring it leaves them queued for
    /// the session's next turn.
    Woken { session_id: AgentSessionId },
}

/// Where the Agent tells its host about the sessions' background work,
/// installed at launch.
pub trait AgentBackgroundEventSink: Send + Sync {
    /// Bounded, non-blocking work, as `AgentEventSink::emit`.
    fn event(&self, event: AgentBackgroundEvent);
}

/// A host that is told nothing: the events wait for each session's next
/// turn.
pub struct NoopAgentBackgroundEventSink;

impl AgentBackgroundEventSink for NoopAgentBackgroundEventSink {
    fn event(&self, _event: AgentBackgroundEvent) {}
}

/// What the facade and the dispatcher share: the host's sink, and the
/// roots' pauses.
#[derive(Clone)]
pub(crate) struct Wakes {
    pub(crate) sink: Arc<dyn AgentBackgroundEventSink>,
    pub(crate) pauses: Pauses,
}

impl Wakes {
    pub(crate) fn new(sink: Arc<dyn AgentBackgroundEventSink>) -> Self {
        Self {
            sink,
            pauses: Pauses::default(),
        }
    }
}
