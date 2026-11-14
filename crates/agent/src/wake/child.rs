//! What a sub-agent's turn tells the host ([`AgentBackgroundEvent::Child`]):
//! the turn's start, the approvals and questions it asks (and their
//! answers), and its end. The `subagents` plugin gives each turn it runs
//! this sink in place of none. Streamed output never crosses: the host
//! follows a sub-agent's transcript, when it shows one, from its journal.

use std::sync::Arc;

use hanafubuki_agent_kernel::{AgentEvent, AgentEventKind, AgentEventSink, AgentSessionId};

use super::{AgentBackgroundEvent, AgentBackgroundEventSink, Wakes};

impl Wakes {
    /// The sink of a turn of a sub-agent in `root`'s tree.
    pub(crate) fn child_events(&self, root: AgentSessionId) -> Arc<dyn AgentEventSink> {
        Arc::new(ChildEvents {
            root,
            sink: self.sink.clone(),
        })
    }
}

struct ChildEvents {
    root: AgentSessionId,
    sink: Arc<dyn AgentBackgroundEventSink>,
}

impl AgentEventSink for ChildEvents {
    fn emit(&self, event: AgentEvent) {
        if !told(&event.kind) {
            return;
        }
        self.sink.event(AgentBackgroundEvent::Child {
            root: self.root,
            session_id: event.session_id,
            event: Box::new(event),
        });
    }
}

fn told(kind: &AgentEventKind) -> bool {
    kind.is_terminal()
        || matches!(
            kind,
            AgentEventKind::TurnStarted { .. }
                | AgentEventKind::ApprovalRequested { .. }
                | AgentEventKind::ApprovalResolved { .. }
                | AgentEventKind::UserInputRequested { .. }
                | AgentEventKind::UserInputResolved { .. }
        )
}
