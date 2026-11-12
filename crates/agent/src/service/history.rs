use super::AgentApplicationService;
use hanafubuki_agent_kernel::{
    AgentError, AgentEventBatch, AgentEventSequence, AgentSessionId, AgentSessionListRequest,
    AgentSessionOpenResult, AgentSessionPage, AgentSessionRuntimeSnapshot, AgentToolCallDisplay,
    AgentTranscriptPage, AgentTranscriptPageRequest, AgentTurnPageRequest,
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

    /// How the call `call_id` of the session's history is shown, from its
    /// tool's presenter; `None` when the history has no such call. It reads
    /// the history: call it off an interface's thread.
    pub fn tool_display(
        &self,
        session_id: AgentSessionId,
        call_id: &str,
    ) -> Result<Option<AgentToolCallDisplay>, AgentError> {
        self.core.tool_display(session_id, call_id)
    }

    /// How a call, named as history records it (`namespace`, `name`), with
    /// the model's `arguments` and its committed `output` is shown, for a
    /// transcript read already.
    pub fn present_tool_call(
        &self,
        namespace: Option<&str>,
        name: &str,
        arguments: &serde_json::Value,
        output: Option<&hanafubuki_agent_tool_contracts::ToolOutput>,
    ) -> AgentToolCallDisplay {
        self.core
            .present_tool_call(namespace, name, arguments, output)
    }
}
