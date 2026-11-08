use super::AgentApplicationService;
use crate::model_api_protocol;
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
    /// is started before the kernel plans tools from its registries. The
    /// kernel calls load history and wait for the session's staged commit,
    /// so they run on the blocking boundary.
    pub(super) async fn activate_provider_session(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentSessionRuntimeSnapshot, String> {
        self.host.ensure_started().await;
        let core = self.core.clone();
        let snapshot = hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent session activation worker",
            move || {
                core.activate_session(session_id)?;
                core.snapshot(session_id)
            },
        )
        .await?
        .map_err(|error| error.to_string())?;
        if snapshot.model.is_empty() {
            return Err(
                "No model is selected for this conversation; use /model or choose one in Agent settings"
                    .to_string(),
            );
        }
        let resolved = self.resolve_provider(&snapshot.provider_id).await?;
        let protocol = model_api_protocol(resolved.definition.protocol);
        if protocol != snapshot.provider_protocol {
            return Err(format!(
                "Agent Provider {} protocol changed; select a model explicitly before continuing",
                resolved.definition.display_name
            ));
        }
        let core = self.core.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent session activation worker",
            move || {
                core.activate_model_session(
                    protocol,
                    &resolved.definition.base_url,
                    resolved.credential,
                    resolved.headers,
                    hanafubuki_agent_kernel::ModelRetryPolicy::default(),
                    resolved.policy.compaction_capability,
                    session_id,
                )?;
                core.snapshot(session_id)
            },
        )
        .await?
        .map_err(|error| error.to_string())
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
