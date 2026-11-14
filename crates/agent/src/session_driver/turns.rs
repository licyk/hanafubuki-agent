//! The shared preparation of regular and woken child turns.

use std::sync::Arc;

use hanafubuki_agent_contracts::TurnExtensions;
use hanafubuki_agent_kernel::{
    AgentError, AgentEventSink, AgentPermissionMode, AgentSessionId, AgentTurnInput,
    AgentTurnOrigin, AgentTurnOutcome, AgentWakePolicy,
};

use super::AgentSessionDriver;

impl AgentSessionDriver {
    pub(crate) async fn run_turn(
        &self,
        session_id: AgentSessionId,
        input: crate::AgentTurnInput,
        event_sink: Arc<dyn AgentEventSink>,
    ) -> Result<AgentTurnOutcome, AgentError> {
        let mut prepared = self
            .prepare_turn(session_id, Some(input.text), input.permission_mode)
            .await?;
        prepared.images = input.images;
        prepared.skills = input.skills;
        self.core
            .start_turn_with_sink(session_id, prepared, event_sink)
            .await
    }

    pub(crate) async fn run_wake_turn(
        &self,
        session_id: AgentSessionId,
        event_sink: Arc<dyn AgentEventSink>,
    ) -> Result<Option<AgentTurnOutcome>, AgentError> {
        let input = self.prepare_turn(session_id, None, None).await?;
        if self.core.snapshot(session_id)?.wake_policy == AgentWakePolicy::Off {
            return Ok(None);
        }
        self.core
            .start_wake_turn_with_sink(session_id, input, event_sink)
            .await
    }

    pub(crate) async fn prepare_turn(
        &self,
        session_id: AgentSessionId,
        message: Option<String>,
        permission_mode: Option<AgentPermissionMode>,
    ) -> Result<AgentTurnInput, AgentError> {
        let session = self
            .activate_provider_session(session_id)
            .await
            .map_err(AgentError::InvalidInput)?;
        let permission_mode = self
            .turn_permission_mode(session_id, permission_mode)
            .await
            .map_err(AgentError::InvalidInput)?;
        let world_state = self
            .collect_world_state(
                session_id,
                session.mode,
                permission_mode,
                message.clone(),
                TurnExtensions::default(),
            )
            .await?;
        let turn = self
            .collect_turn_extensions(session_id, session.mode)
            .await?;
        Ok(AgentTurnInput {
            text: message.unwrap_or_default(),
            images: Vec::new(),
            skills: Vec::new(),
            permission_mode,
            turn,
            world_state,
            origin: AgentTurnOrigin::Programmatic,
            provisional_title: None,
        })
    }
}
