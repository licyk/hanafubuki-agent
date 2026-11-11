use crate::service::AgentApplicationService;
use crate::{AgentSessionModelSelectionRequest, AgentSessionReasoningEffortRequest};
use hanafubuki_agent_kernel::AgentSessionRuntimeSnapshot;

impl AgentApplicationService {
    pub async fn set_session_model(
        &self,
        request: AgentSessionModelSelectionRequest,
    ) -> Result<AgentSessionRuntimeSnapshot, String> {
        self.driver.select_model(request).await
    }

    pub async fn set_session_reasoning_effort(
        &self,
        request: AgentSessionReasoningEffortRequest,
    ) -> Result<AgentSessionRuntimeSnapshot, String> {
        self.activate_provider_session(request.session_id).await?;
        let service = self.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent session reasoning effort worker",
            move || {
                service
                    .core
                    .set_session_reasoning_effort(
                        request.session_id,
                        request.generation,
                        request.revision,
                        request.effort,
                    )
                    .map_err(|error| error.to_string())
            },
        )
        .await?
    }
}
