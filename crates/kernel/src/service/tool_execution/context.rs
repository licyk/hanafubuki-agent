use super::*;

impl AgentService {
    /// The invocation context of one planned call, read from the session at
    /// the moment the call runs.
    pub(super) async fn planned_tool_context(
        &self,
        identity: TurnIdentity,
        call: &ModelToolCall,
        catalog_revision: crate::AgentToolCatalogRevision,
        registration: &crate::AgentPlannedTool,
        tool_plan: &crate::AgentToolPlan,
    ) -> Result<AgentToolInvocationContext, AgentError> {
        self.refresh_session_image_input_capability(identity.session_id)
            .await?;
        // The session's progress consumer runs on the turn's runtime.
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            AgentError::InvalidInput("Agent tool calls require an async runtime".to_string())
        })?;
        let cell = self.session(identity.session_id)?;
        let record = cell.lock()?;
        Ok(AgentToolInvocationContext {
            session_id: identity.session_id,
            turn_id: identity.turn_id,
            generation: identity.generation,
            mode: record.settings.config.mode,
            input_capabilities: record.settings.effective_input_capabilities,
            session_revision: crate::AgentRevision::new(record.status.revision),
            call_id: call.call_id.clone(),
            catalog_revision,
            provider_id: registration.provider_id.clone(),
            tool_id: registration.tool_id.clone(),
            tool_generation: registration.generation,
            tool_plan_leases: tool_plan.leases().to_vec(),
            approval_rules: tool_plan.approval_rules().cloned(),
            progress: Arc::new(super::super::progress::SessionToolProgressSink {
                service: self.clone(),
                session_id: identity.session_id,
                call_id: Some(call.call_id.clone()),
                runtime,
            }),
        })
    }

    /// The turn's extensions, frozen at its admission.
    pub(super) fn turn_extensions(
        &self,
        identity: TurnIdentity,
    ) -> Result<hanafubuki_agent_contracts::TurnExtensions, AgentError> {
        let cell = self.session(identity.session_id)?;
        let record = cell.lock()?;
        let turn = exact_turn(&record, identity.turn_id, identity.generation)?;
        Ok(turn.extensions.clone())
    }
}
