use super::AgentApplicationService;
use crate::titles::provisional_agent_title;
use crate::{prepare_and_stage_images, AgentSteeringRequest, AgentTurnRequest};
use hanafubuki_agent_contracts::{ContextRequest, TurnAdmissionRequest, TurnExtensions};
use hanafubuki_agent_kernel::{
    AgentCompactionGeneration, AgentCompactionId, AgentCompactionOutcome, AgentError,
    AgentEventSink, AgentGeneration, AgentPlanImplementationRequest, AgentRevision, AgentSessionId,
    AgentSessionMode, AgentSteeringAccepted, AgentTurnAccepted, AgentTurnCancellationOutcome,
    AgentTurnId, AgentTurnOutcome, AgentUserInputAccepted, AgentUserInputAnswer,
    AgentUserInputRequestId, AgentWorldStateSnapshot, ApprovalRequestId, ApprovalUserDecision,
};
use std::sync::Arc;

impl AgentApplicationService {
    pub fn resolve_user_input(
        &self,
        session_id: AgentSessionId,
        turn_id: AgentTurnId,
        generation: AgentGeneration,
        request_id: AgentUserInputRequestId,
        answers: Vec<AgentUserInputAnswer>,
    ) -> Result<AgentUserInputAccepted, String> {
        self.core
            .resolve_user_input(session_id, turn_id, generation, request_id, answers)
            .map_err(|error| error.to_string())
    }

    pub async fn start_turn(
        &self,
        session_id: AgentSessionId,
        mut input: hanafubuki_agent_kernel::AgentTurnInput,
    ) -> Result<AgentTurnOutcome, AgentError> {
        let session = self
            .activate_provider_session(session_id)
            .await
            .map_err(AgentError::InvalidInput)?;
        let world_state = self
            .collect_world_state(
                session_id,
                session.mode,
                input.permission_mode,
                TurnExtensions::default(),
            )
            .await?;
        input.turn = self
            .collect_turn_extensions(session_id, session.mode)
            .await?;
        input.world_state = world_state;
        self.core.start_turn(session_id, input).await
    }

    pub async fn submit_turn_with_sink(
        &self,
        session_id: AgentSessionId,
        input: AgentTurnRequest,
        event_sink: Arc<dyn AgentEventSink>,
    ) -> Result<AgentTurnAccepted, String> {
        let session = self.activate_provider_session(session_id).await?;
        // Immediate feedback: the admission commits this title with the turn
        // if the session still has none, and the `titles` plugin may replace
        // it with a generated one.
        let provisional_title = session
            .title
            .is_none()
            .then(|| provisional_agent_title(&input.text))
            .flatten();
        let core = self.core.clone();
        let files = self.files.clone();
        let permission_mode = self.load_agent_settings().await?.permission_mode;
        let world_state = self
            .collect_world_state(session_id, session.mode, permission_mode, input.host)
            .await
            .map_err(|error| error.to_string())?;
        let turn = self
            .collect_turn_extensions(session_id, session.mode)
            .await
            .map_err(|error| error.to_string())?;
        // Durable admission writes the conversation repository. Keep only that
        // bounded synchronous commit off the async caller; the accepted turn
        // is then owned by hanafubuki-agent-kernel's managed Tokio task.
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent turn admission worker",
            move || {
                let images = match prepare_and_stage_images(&core, &files, session_id, input.images)
                {
                    Ok(images) => images,
                    Err(error) => {
                        if let Err(cleanup_error) =
                            core.cleanup_unreferenced_image_attachments(session_id)
                        {
                            log::debug!(
                                "agent turn image attachment cleanup failed session={session_id}: {cleanup_error}"
                            );
                        }
                        return Err(error);
                    }
                };
                let accepted = core.submit_turn_with_sink(
                    session_id,
                    hanafubuki_agent_kernel::AgentTurnInput {
                        text: input.text,
                        images,
                        skills: input.skills,
                        permission_mode,
                        turn,
                        world_state,
                        origin: hanafubuki_agent_kernel::AgentTurnOrigin::User,
                        provisional_title,
                    },
                    event_sink,
                );
                if accepted.is_err() {
                    if let Err(cleanup_error) =
                        core.cleanup_unreferenced_image_attachments(session_id)
                    {
                        log::debug!(
                            "agent turn image attachment cleanup failed session={session_id}: {cleanup_error}"
                        );
                    }
                }
                accepted.map_err(|error| error.to_string())
            },
        )
        .await?
    }

    pub async fn submit_plan_implementation_with_sink(
        &self,
        request: AgentPlanImplementationRequest,
        event_sink: Arc<dyn AgentEventSink>,
    ) -> Result<AgentTurnAccepted, String> {
        self.activate_provider_session(request.session_id).await?;
        let core = self.core.clone();
        let permission_mode = self.load_agent_settings().await?.permission_mode;
        // Implementing the plan returns the session to Default mode.
        let world_state = self
            .collect_world_state(
                request.session_id,
                AgentSessionMode::Default,
                permission_mode,
                TurnExtensions::default(),
            )
            .await
            .map_err(|error| error.to_string())?;
        let turn = self
            .collect_turn_extensions(request.session_id, AgentSessionMode::Default)
            .await
            .map_err(|error| error.to_string())?;
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent plan implementation admission worker",
            move || {
                core.submit_plan_implementation_with_context(
                    request,
                    permission_mode,
                    turn,
                    world_state,
                    event_sink,
                )
                .map_err(|error| error.to_string())
            },
        )
        .await?
    }

    pub async fn steer_turn_with_sink(
        &self,
        session_id: AgentSessionId,
        turn_id: AgentTurnId,
        generation: AgentGeneration,
        input: AgentSteeringRequest,
        event_sink: Arc<dyn AgentEventSink>,
    ) -> Result<AgentSteeringAccepted, String> {
        let core = self.core.clone();
        let steering_context = self.steering_context.clone();
        let files = self.files.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent steering admission worker",
            move || {
                let host_context = steering_context.steering_context(&input.host);
                let images = match prepare_and_stage_images(&core, &files, session_id, input.images)
                {
                    Ok(images) => images,
                    Err(error) => {
                        if let Err(cleanup_error) =
                            core.cleanup_unreferenced_image_attachments(session_id)
                        {
                            log::debug!(
                                "agent turn image attachment cleanup failed session={session_id}: {cleanup_error}"
                            );
                        }
                        return Err(error);
                    }
                };
                let accepted = core.steer_turn_with_sink(
                    session_id,
                    turn_id,
                    generation,
                    hanafubuki_agent_kernel::AgentSteeringSubmission {
                        text: input.text,
                        images,
                        host_context,
                    },
                    event_sink.as_ref(),
                );
                if accepted.is_err() {
                    if let Err(cleanup_error) =
                        core.cleanup_unreferenced_image_attachments(session_id)
                    {
                        log::debug!(
                            "agent turn image attachment cleanup failed session={session_id}: {cleanup_error}"
                        );
                    }
                }
                accepted.map_err(|error| error.to_string())
            },
        )
        .await?
    }

    pub async fn cancel_turn(
        &self,
        session_id: AgentSessionId,
        turn_id: AgentTurnId,
        generation: AgentGeneration,
    ) -> Result<AgentTurnCancellationOutcome, AgentError> {
        self.core
            .cancel_turn_confirmed(session_id, turn_id, generation)
            .await
    }

    pub async fn compact_session(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
    ) -> Result<AgentCompactionOutcome, AgentError> {
        self.activate_provider_session(session_id)
            .await
            .map_err(AgentError::InvalidInput)?;
        self.core
            .compact_session(session_id, generation, revision)
            .await
    }

    pub async fn compact_session_with_sink(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
        event_sink: Arc<dyn AgentEventSink>,
    ) -> Result<AgentCompactionOutcome, AgentError> {
        self.activate_provider_session(session_id)
            .await
            .map_err(AgentError::InvalidInput)?;
        self.core
            .compact_session_with_sink(session_id, generation, revision, event_sink)
            .await
    }

    pub fn cancel_compaction(
        &self,
        session_id: AgentSessionId,
        compaction_id: AgentCompactionId,
        generation: AgentCompactionGeneration,
    ) -> Result<(), AgentError> {
        self.core
            .cancel_compaction(session_id, compaction_id, generation)
    }

    pub fn resolve_approval(
        &self,
        session_id: AgentSessionId,
        turn_id: AgentTurnId,
        generation: AgentGeneration,
        request_id: ApprovalRequestId,
        decision: ApprovalUserDecision,
    ) -> Result<(), AgentError> {
        self.core
            .resolve_approval(session_id, turn_id, generation, request_id, decision)
    }
}

impl AgentApplicationService {
    /// The turn's world state, from the context contributors plugins
    /// registered. `host` is what the host submitted with the turn.
    async fn collect_world_state(
        &self,
        session_id: AgentSessionId,
        mode: AgentSessionMode,
        permission_mode: hanafubuki_agent_kernel::AgentPermissionMode,
        host: TurnExtensions,
    ) -> Result<AgentWorldStateSnapshot, AgentError> {
        let request = ContextRequest {
            session_id,
            mode,
            permission_mode,
            turn: host,
        };
        hanafubuki_agent_kernel::collect_world_state(&self.host.registries().context, &request)
            .await
    }

    /// The turn's extensions, from the turn admission contributors plugins
    /// registered (the `skills` plugin freezes the turn's Skill catalog).
    async fn collect_turn_extensions(
        &self,
        session_id: AgentSessionId,
        mode: AgentSessionMode,
    ) -> Result<TurnExtensions, AgentError> {
        hanafubuki_agent_kernel::collect_turn_extensions(
            &self.host.registries().turn_admission,
            &TurnAdmissionRequest { session_id, mode },
        )
        .await
    }
}
