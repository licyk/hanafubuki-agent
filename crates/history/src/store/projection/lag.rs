use crate::store::*;

/// Why a session's projection cannot be served (`StoreInner::projection_lag`),
/// which decides what clears it.
pub(in crate::store) enum ProjectionLag {
    /// An append was durable but its projection failed. The session's writer
    /// clears it once it projects the rollout again: on a later append, or
    /// when it stops.
    Append(String),
    /// The index could not admit the session: `open` refused it, or a read
    /// found its row undecodable. Only a repair
    /// (`repair_session_projection`) or the next start judges it again.
    Refused(String),
}

impl ProjectionLag {
    pub(in crate::store) fn reason(&self) -> &str {
        match self {
            Self::Append(reason) | Self::Refused(reason) => reason,
        }
    }
}

impl FileAgentHistoryStore {
    /// Records that an append's projection failed, unless the session is
    /// refused, which stays the reason.
    pub(in crate::store) fn record_append_lag(&self, session_id: AgentSessionId, reason: String) {
        let mut lag = self.inner.projection_lag.lock();
        if !matches!(lag.get(&session_id), Some(ProjectionLag::Refused(_))) {
            lag.insert(session_id, ProjectionLag::Append(reason));
        }
    }

    /// The writer projected the whole rollout: an append's lag is over. A
    /// refusal is not the writer's to clear.
    pub(in crate::store) fn clear_append_lag(&self, session_id: AgentSessionId) {
        let mut lag = self.inner.projection_lag.lock();
        if matches!(lag.get(&session_id), Some(ProjectionLag::Append(_))) {
            lag.remove(&session_id);
        }
    }
}
