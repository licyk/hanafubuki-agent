use super::AgentApplicationService;
use crate::titles::provisional_agent_title;
use crate::{prepare_and_stage_images, AgentSteeringRequest, AgentTurnInput, AgentTurnRequest};
use hanafubuki_agent_config::AgentSandboxMode;
use hanafubuki_agent_contracts::TurnExtensions;
use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_kernel::{
    AgentCompactionGeneration, AgentCompactionId, AgentCompactionOutcome, AgentError,
    AgentEventSink, AgentGeneration, AgentPermissionMode, AgentPlanImplementationRequest,
    AgentRevision, AgentSessionId, AgentSessionMode, AgentSteeringAccepted, AgentTurnAccepted,
    AgentTurnCancellationOutcome, AgentTurnId, AgentTurnOutcome, AgentUserInputAccepted,
    AgentUserInputAnswer, AgentUserInputRequestId, AgentUserShellCommand, ApprovalRequestId,
    ApprovalUserDecision,
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

    /// Runs `input`'s turn and waits for its outcome. It runs in the
    /// input's permission mode, else the session's own, else the configured
    /// `permissionMode`.
    pub async fn start_turn(
        &self,
        session_id: AgentSessionId,
        input: AgentTurnInput,
    ) -> Result<AgentTurnOutcome, AgentError> {
        self.host.ensure_started().await;
        self.driver
            .run_turn(session_id, input, Arc::new(crate::session_driver::Discard))
            .await
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
        let permission_mode = self.driver.turn_permission_mode(session_id, None).await?;
        let world_state = self
            .driver
            .collect_world_state(
                session_id,
                session.mode,
                permission_mode,
                Some(input.text.clone()),
                input.host,
            )
            .await
            .map_err(|error| error.to_string())?;
        let turn = self
            .driver
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
        let permission_mode = self
            .driver
            .turn_permission_mode(request.session_id, None)
            .await?;
        // Implementing the plan returns the session to Default mode.
        let world_state = self
            .driver
            .collect_world_state(
                request.session_id,
                AgentSessionMode::Default,
                permission_mode,
                None,
                TurnExtensions::default(),
            )
            .await
            .map_err(|error| error.to_string())?;
        let turn = self
            .driver
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

    /// Runs `command`, which the person typed after `!`, as a turn of its
    /// own that samples no model: through the `exec_command` tool, in the
    /// session's sandbox and permission mode, approved by the person who
    /// typed it (an approval rule, such as `git clone`'s, still asks). It
    /// runs in the working directory, until it exits or the turn is
    /// cancelled, and the model reads what it did from its next request on.
    pub async fn submit_user_shell_command_with_sink(
        &self,
        session_id: AgentSessionId,
        command: String,
        event_sink: Arc<dyn AgentEventSink>,
    ) -> Result<AgentTurnAccepted, String> {
        let command = command.trim().to_string();
        if command.is_empty() {
            return Err("a `!` command needs a command after the `!`".to_string());
        }
        let session = self.activate_provider_session(session_id).await?;
        let permission_mode = self.driver.turn_permission_mode(session_id, None).await?;
        let turn = self
            .driver
            .collect_turn_extensions(session_id, session.mode)
            .await
            .map_err(|error| error.to_string())?;
        let workdir = self
            .executions
            .working_directory()
            .and_then(|directory| ResolvedPath::resolve(directory).ok())
            .map(|directory| directory.native_text().to_string());
        let core = self.core.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent shell command admission worker",
            move || {
                core.submit_user_shell_command_with_sink(
                    session_id,
                    AgentUserShellCommand { command, workdir },
                    permission_mode,
                    turn,
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

    /// Cancels the turn, as the person asked: a root's wakes pause until
    /// the person's next message (its background work goes on).
    pub async fn cancel_turn(
        &self,
        session_id: AgentSessionId,
        turn_id: AgentTurnId,
        generation: AgentGeneration,
    ) -> Result<AgentTurnCancellationOutcome, AgentError> {
        // Before the cancel, so the turn's end wakes nothing.
        if self
            .core
            .snapshot(session_id)
            .is_ok_and(|session| session.parent_session_id.is_none())
        {
            self.wakes.pauses.interrupted(session_id);
            self.wakes.feed.hint(session_id);
        }
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

    /// The permission mode the user chose for the session
    /// ([`Self::set_session_permission_mode`]); `None` while the configured
    /// `permissionMode` applies.
    pub fn session_permission_mode(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Option<AgentPermissionMode>, AgentError> {
        self.activate_session(session_id)?;
        self.core.session_permission_mode(session_id)
    }

    /// Sets the session's permission mode, in place of the configured
    /// `permissionMode`, until the session ends; it is not persisted. A
    /// running turn keeps its mode; the next turn takes this one.
    pub fn set_session_permission_mode(
        &self,
        session_id: AgentSessionId,
        mode: AgentPermissionMode,
    ) -> Result<(), AgentError> {
        self.activate_session(session_id)?;
        self.core.set_session_permission_mode(session_id, mode)
    }

    /// The sandbox mode the user chose for the session
    /// ([`Self::set_session_sandbox_mode`]); `None` while the configured
    /// `sandboxMode` (or the platform's default) applies.
    pub fn session_sandbox_mode(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Option<AgentSandboxMode>, AgentError> {
        self.activate_session(session_id)?;
        self.core.session_sandbox_mode(session_id)
    }

    /// Sets the sandbox the session's commands run in, in place of the
    /// configured `sandboxMode`, until the session ends; it is not
    /// persisted. Independent of the permission mode: a command a
    /// full-access permission mode runs without asking is still sandboxed.
    /// A running turn keeps its sandbox; the next turn takes this one.
    pub fn set_session_sandbox_mode(
        &self,
        session_id: AgentSessionId,
        mode: AgentSandboxMode,
    ) -> Result<(), AgentError> {
        self.activate_session(session_id)?;
        self.core.set_session_sandbox_mode(session_id, mode)
    }

    /// The sandbox this machine gives the commands that ask for one, as the
    /// configured `[windows] sandbox` has it, or why it cannot.
    pub fn sandbox_status(&self) -> Result<hanafubuki_agent_sandbox::SandboxStatus, AgentError> {
        let settings = self.config.settings().map_err(AgentError::Persistence)?;
        Ok(self
            .executions
            .sandbox()
            .with_windows_sandbox(settings.windows.sandbox)
            .status())
    }

    /// Records that the user approves the action automatic permission review
    /// `review_id` denied (one of the session's ten latest denials): a
    /// context note the model and the next review read. It starts no turn,
    /// and a retry of the action is reviewed again. Refused while a turn or
    /// a compaction runs.
    pub fn approve_denied_action(
        &self,
        session_id: AgentSessionId,
        review_id: &str,
    ) -> Result<(), AgentError> {
        self.activate_session(session_id)?;
        self.core.approve_denied_action(session_id, review_id)
    }
}
