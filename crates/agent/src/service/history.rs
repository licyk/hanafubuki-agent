use super::AgentApplicationService;
use hanafubuki_agent_kernel::{
    AgentError, AgentEventBatch, AgentEventSequence, AgentSessionId, AgentSessionListRequest,
    AgentSessionOpenResult, AgentSessionPage, AgentSessionRuntimeSnapshot, AgentTranscriptPage,
    AgentTranscriptPageRequest, AgentTurnPageRequest,
};

impl AgentApplicationService {
    pub fn snapshot(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentSessionRuntimeSnapshot, AgentError> {
        self.core.snapshot(session_id)
    }

    pub fn events_since(
        &self,
        session_id: AgentSessionId,
        after: Option<AgentEventSequence>,
    ) -> Result<AgentEventBatch, AgentError> {
        self.core.events_since(session_id, after)
    }

    pub fn list_sessions_page(
        &self,
        request: AgentSessionListRequest,
    ) -> Result<AgentSessionPage, AgentError> {
        self.core.list_sessions_page(request)
    }

    pub fn open_session(
        &self,
        session_id: AgentSessionId,
        page_size: usize,
    ) -> Result<AgentSessionOpenResult, AgentError> {
        self.core.open_session(session_id, page_size)
    }

    pub(super) fn activate_session(&self, session_id: AgentSessionId) -> Result<(), AgentError> {
        self.core.activate_session(session_id)
    }

    /// Prepares `session_id` for a model operation. Every turn, plan
    /// implementation and compaction passes through here, so the plugin host
    /// is started before the kernel plans tools from its registries.
    pub(super) async fn activate_provider_session(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentSessionRuntimeSnapshot, String> {
        self.host.ensure_started().await;
        self.driver.activate_provider_session(session_id).await
    }

    pub fn read_transcript_page(
        &self,
        request: AgentTranscriptPageRequest,
    ) -> Result<AgentTranscriptPage, AgentError> {
        self.core.read_transcript_page(request)
    }

    pub fn read_turn_page(
        &self,
        request: AgentTurnPageRequest,
    ) -> Result<AgentTranscriptPage, AgentError> {
        self.core.read_turn_page(request)
    }
}
