//! A session's hierarchical compression as a person reads it: the
//! terminal interface's `/compression`.

use hanafubuki_agent_kernel::{AgentCompressionStatus, AgentError, AgentSessionId};

use super::AgentApplicationService;
use crate::session_driver::hierarchical_compression;

impl AgentApplicationService {
    /// `session_id`'s hierarchical compression, tuned by
    /// `[compaction.hierarchical]` as its next turn would be: the status
    /// panel, the cache report, the counters and the content store's size.
    /// `None` when the session neither runs `hierarchical` nor kept a state
    /// from it. Nothing is committed. It blocks: it reads the settings and
    /// projects the session's history.
    pub fn compression_status(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Option<AgentCompressionStatus>, AgentError> {
        self.activate_session(session_id)?;
        let settings = self.config.settings().map_err(AgentError::Persistence)?;
        self.core.compression_status(
            session_id,
            &hierarchical_compression(&settings.compaction.hierarchical),
        )
    }
}
