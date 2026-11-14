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
//!
//! The host also reads what a root's tree holds open
//! ([`AgentBackgroundTasks`], `tasks.rs`) and is told when that changes
//! ([`AgentBackgroundEvent::Changed`], `feed.rs`), and what a sub-agent's
//! turn asks of a person ([`AgentBackgroundEvent::Child`], `child.rs`).

mod child;
mod dispatcher;
mod feed;
mod pause;
mod tasks;

use std::sync::Arc;

use hanafubuki_agent_kernel::{AgentEvent, AgentSessionId};

pub(crate) use self::dispatcher::wake_plugin;
pub(crate) use self::feed::Feed;
pub(crate) use self::pause::Pauses;
pub use self::pause::{AgentWakePause, AgentWakeState, MAXIMUM_WOKEN_TURNS};
pub(crate) use self::tasks::root_of;
pub use self::tasks::{AgentBackgroundStatus, AgentBackgroundTasks, AgentBackgroundTerminal};
use crate::plugins::SubagentRuns;

/// What the Agent tells its host about the sessions' background work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentBackgroundEvent {
    /// A root has wake events and no turn: `submit_wake_turn_with_sink`
    /// starts the turn that reads them. Ignoring it leaves them queued for
    /// the session's next turn.
    Woken { session_id: AgentSessionId },
    /// What `background_tasks(root)` answers changed: an agent of the tree
    /// started, ended, or changed its title or model; an event was queued
    /// on `root` or delivered; its wakes paused or resumed. Read it again:
    /// the event is a hint, the snapshot is the truth.
    Changed { root: AgentSessionId },
    /// An event of the turn a sub-agent of `root`'s tree runs: its start,
    /// an approval or a question it asks or that was answered, and its end
    /// (completed, cancelled or failed). Nothing else of the turn is told:
    /// its journal has the rest.
    Child {
        root: AgentSessionId,
        session_id: AgentSessionId,
        event: Box<AgentEvent>,
    },
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

/// What the facade, the dispatcher and the `subagents` plugin share: the
/// host's sink, the roots' pauses, the feed's hints, and the sub-agents
/// this instance runs (which outlive the plugin's restarts).
#[derive(Clone)]
pub(crate) struct Wakes {
    pub(crate) sink: Arc<dyn AgentBackgroundEventSink>,
    pub(crate) pauses: Pauses,
    pub(crate) feed: Arc<Feed>,
    pub(crate) runs: SubagentRuns,
}

impl Wakes {
    pub(crate) fn new(sink: Arc<dyn AgentBackgroundEventSink>) -> Self {
        let feed = Arc::new(Feed::default());
        Self {
            sink,
            pauses: Pauses::default(),
            runs: SubagentRuns::new(feed.clone()),
            feed,
        }
    }
}
