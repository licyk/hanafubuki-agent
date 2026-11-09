//! A session's children: its side conversation, and what archiving or
//! deleting a session does to the children it has.

use std::collections::VecDeque;

use super::AgentApplicationService;
use crate::side_conversation::side_conversation_context;
use hanafubuki_agent_kernel::{
    AgentChildSessionBase, AgentChildSessionRequest, AgentError, AgentSessionId,
    AgentSessionLifecycle, AgentSessionListRequest, AgentSessionOrigin, AgentSessionParentFilter,
    AgentSessionPersistence, AgentSessionRuntimeSnapshot, AgentSessionSummary,
    AGENT_SESSION_PAGE_SIZE_MAX,
};

impl AgentApplicationService {
    /// Opens the side conversation of `parent`: an ephemeral fork of its
    /// history at its last turn boundary (before the turn it runs, if one
    /// does), with the side policy and the boundary its model reads first.
    /// A parent has at most one: opening it again returns the open one.
    /// Turns run on it through [`Self::submit_turn_with_sink`]; it is never
    /// listed and is gone once closed or when the Agent stops.
    pub fn open_side_conversation(
        &self,
        parent: AgentSessionId,
    ) -> Result<AgentSessionRuntimeSnapshot, AgentError> {
        self.activate_session(parent)?;
        let _opening = self.side_conversations.lock();
        if let Some(side) = self
            .core
            .ephemeral_children(parent, Some(AgentSessionOrigin::Side))
            .first()
        {
            return self.core.snapshot(*side);
        }
        self.driver.create_child(AgentChildSessionRequest {
            parent_session_id: parent,
            origin: AgentSessionOrigin::Side,
            persistence: AgentSessionPersistence::Ephemeral,
            base: AgentChildSessionBase::ParentHead,
            context: side_conversation_context(),
        })
    }

    /// Closes the side conversation `side`: interrupts its running turn,
    /// stops its background terminals and drops it.
    pub async fn close_side_conversation(&self, side: AgentSessionId) -> Result<(), AgentError> {
        let snapshot = self.core.snapshot(side)?;
        if snapshot.origin != Some(AgentSessionOrigin::Side) || !snapshot.ephemeral {
            return Err(AgentError::InvalidInput(format!(
                "Agent session {side} is not a side conversation"
            )));
        }
        self.driver.close(side).await
    }

    /// Lets go of the live ephemeral children of `session_id`, whatever
    /// made them.
    pub(super) fn release_ephemeral_children(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentError> {
        for child in self.core.ephemeral_children(session_id, None) {
            self.driver.release(child)?;
        }
        Ok(())
    }

    /// The persisted descendants of `session_id`, archived or not, each
    /// after its parent.
    pub(super) fn persisted_descendants(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Vec<AgentSessionSummary>, AgentError> {
        let mut descendants = Vec::new();
        let mut parents = VecDeque::from([session_id]);
        while let Some(parent) = parents.pop_front() {
            for lifecycle in [
                AgentSessionLifecycle::Available,
                AgentSessionLifecycle::Archived,
            ] {
                let mut cursor = None;
                loop {
                    let page = self.core.list_sessions_page(AgentSessionListRequest {
                        page_size: AGENT_SESSION_PAGE_SIZE_MAX,
                        cursor,
                        lifecycle,
                        parent: AgentSessionParentFilter::ChildrenOf(parent),
                        ..AgentSessionListRequest::default()
                    })?;
                    parents.extend(page.sessions.iter().map(|child| child.session_id));
                    descendants.extend(page.sessions);
                    let Some(next) = page.next_cursor else {
                        break;
                    };
                    cursor = Some(next);
                }
            }
        }
        Ok(descendants)
    }

    /// Archives one descendant at its current revision.
    pub(super) fn archive_one(&self, session_id: AgentSessionId) -> Result<(), AgentError> {
        self.activate_session(session_id)?;
        self.quiesce_background_terminals(session_id)?;
        let current = self.core.snapshot(session_id)?;
        self.core
            .archive_session(session_id, current.generation, current.revision)
            .map(|_| ())
    }
}
