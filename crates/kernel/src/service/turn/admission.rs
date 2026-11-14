use super::state::AdmittedTurn;
use super::*;

impl AgentService {
    pub async fn start_turn(
        &self,
        session_id: AgentSessionId,
        input: impl Into<crate::AgentTurnInput>,
    ) -> Result<AgentTurnOutcome, AgentError> {
        self.start_turn_with_sink(session_id, input, Arc::new(NoopAgentEventSink))
            .await
    }

    pub async fn start_turn_with_sink(
        &self,
        session_id: AgentSessionId,
        input: impl Into<crate::AgentTurnInput>,
        event_sink: Arc<dyn AgentEventSink>,
    ) -> Result<AgentTurnOutcome, AgentError> {
        let turn_started = Instant::now();
        log::info!("agent_diag stage=turn.start.begin session_id={session_id}");
        if !self.inner.accepting.load(Ordering::Acquire) {
            log::info!(
                "agent_diag stage=turn.start.end session_id={session_id} outcome=rejected_shutting_down elapsed_ms={}",
                turn_started.elapsed().as_millis()
            );
            return Err(AgentError::ShuttingDown);
        }
        let admission_started = Instant::now();
        log::info!("agent_diag stage=turn.admission.begin session_id={session_id}");
        let identity = match self.admit_turn(session_id, input.into(), &event_sink).await {
            Ok(identity) => identity,
            Err(error) => {
                log::info!(
                    "agent_diag stage=turn.admission.end session_id={session_id} outcome=error elapsed_ms={}",
                    admission_started.elapsed().as_millis()
                );
                return Err(error);
            }
        };
        log::info!(
            "agent_diag stage=turn.admission.end session_id={} turn_id={} generation={} outcome=ok elapsed_ms={}",
            identity.session_id,
            identity.turn_id,
            identity.generation,
            admission_started.elapsed().as_millis()
        );
        self.execute_admitted_turn(identity, event_sink)
            .await?
            .ok_or_else(|| AgentError::Protocol("the turn ended without an answer".to_string()))
    }

    /// Durably admits a turn, schedules the complete loop on the current async
    /// runtime, and returns without retaining the caller's IPC future.
    pub fn submit_turn_with_sink(
        &self,
        session_id: AgentSessionId,
        input: impl Into<crate::AgentTurnInput>,
        event_sink: Arc<dyn AgentEventSink>,
    ) -> Result<AgentTurnAccepted, AgentError> {
        if !self.inner.accepting.load(Ordering::Acquire) {
            return Err(AgentError::ShuttingDown);
        }
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            AgentError::InvalidInput("Agent turn submission requires an async runtime".to_string())
        })?;
        let admitted = self.begin_turn(session_id, input.into(), event_sink.as_ref())?;
        self.spawn_managed_turn(admitted, event_sink, runtime)
    }

    /// Admits a turn that runs `command`, which the person typed after `!`,
    /// through the planned `exec_command` tool as the person's own call, and
    /// schedules it as `submit_turn_with_sink` does. It samples no model:
    /// the next turn's request carries what the command did.
    pub fn submit_user_shell_command_with_sink(
        &self,
        session_id: AgentSessionId,
        command: crate::AgentUserShellCommand,
        permission_mode: hanafubuki_agent_approval::AgentPermissionMode,
        turn: hanafubuki_agent_contracts::TurnExtensions,
        event_sink: Arc<dyn AgentEventSink>,
    ) -> Result<AgentTurnAccepted, AgentError> {
        if !self.inner.accepting.load(Ordering::Acquire) {
            return Err(AgentError::ShuttingDown);
        }
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            AgentError::InvalidInput("Agent turn submission requires an async runtime".to_string())
        })?;
        let mut input = crate::AgentTurnInput::from(command.command.clone());
        input.permission_mode = permission_mode;
        input.turn = turn;
        let admitted = self.begin_turn_with_admission(
            session_id,
            input,
            TurnAdmission::UserShellCommand(command),
            event_sink.as_ref(),
        )?;
        self.spawn_managed_turn(admitted, event_sink, runtime)
    }

    /// Atomically switches an idle Plan session to Default mode, commits the
    /// fixed implementation instruction, and schedules the resulting turn.
    pub fn submit_plan_implementation_with_sink(
        &self,
        request: crate::AgentPlanImplementationRequest,
        event_sink: Arc<dyn AgentEventSink>,
    ) -> Result<AgentTurnAccepted, AgentError> {
        self.submit_plan_implementation_with_permission_mode(
            request,
            hanafubuki_agent_approval::AgentPermissionMode::Manual,
            event_sink,
        )
    }

    pub fn submit_plan_implementation_with_permission_mode(
        &self,
        request: crate::AgentPlanImplementationRequest,
        permission_mode: hanafubuki_agent_approval::AgentPermissionMode,
        event_sink: Arc<dyn AgentEventSink>,
    ) -> Result<AgentTurnAccepted, AgentError> {
        self.submit_plan_implementation_with_context(
            request,
            permission_mode,
            hanafubuki_agent_contracts::TurnExtensions::default(),
            AgentWorldStateSnapshot::default(),
            event_sink,
        )
    }

    pub fn submit_plan_implementation_with_context(
        &self,
        request: crate::AgentPlanImplementationRequest,
        permission_mode: hanafubuki_agent_approval::AgentPermissionMode,
        turn: hanafubuki_agent_contracts::TurnExtensions,
        world_state: AgentWorldStateSnapshot,
        event_sink: Arc<dyn AgentEventSink>,
    ) -> Result<AgentTurnAccepted, AgentError> {
        if !self.inner.accepting.load(Ordering::Acquire) {
            return Err(AgentError::ShuttingDown);
        }
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            AgentError::InvalidInput("Agent turn submission requires an async runtime".to_string())
        })?;
        let mut input = crate::AgentTurnInput::from(PLAN_IMPLEMENTATION_MESSAGE);
        input.permission_mode = permission_mode;
        input.turn = turn;
        input.world_state = world_state;
        let admitted = self.begin_turn_with_admission(
            request.session_id,
            input,
            TurnAdmission::ImplementPlan(request),
            event_sink.as_ref(),
        )?;
        self.spawn_managed_turn(admitted, event_sink, runtime)
    }

    /// Retains the session's sink, tells the observers about the admission
    /// and only then spawns the turn's task, so nothing the turn commits is
    /// told before its admission and an observer can already reach the
    /// session's sink.
    pub(in crate::service) fn spawn_managed_turn(
        &self,
        admitted: AdmittedTurn,
        event_sink: Arc<dyn AgentEventSink>,
        runtime: tokio::runtime::Handle,
    ) -> Result<AgentTurnAccepted, AgentError> {
        let identity = admitted.identity;
        self.inner
            .event_sinks
            .lock()
            .insert(identity.session_id, event_sink.clone());
        self.inner.managed_turns.lock().insert(identity, None);
        self.notify_turn_admitted(admitted);
        let service = self.clone();
        let managed = ManagedTurnTaskGuard {
            service: service.clone(),
            identity,
        };
        let task = runtime.spawn(async move {
            let _managed = managed;
            // The turn catches its own panics; one that escapes it (from its
            // terminal commit) is logged here instead of ending the task
            // silently.
            match hanafubuki_agent_foundation::panic::catch_panic_async(
                service.execute_admitted_turn(identity, event_sink),
            )
            .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => log::error!("managed agent turn failed: {error}"),
                Err(message) => {
                    log::error!(
                        "managed agent turn panicked session_id={} turn_id={} generation={}: {message}",
                        identity.session_id,
                        identity.turn_id,
                        identity.generation
                    );
                    service.inner.kernel_faults.kernel_fault("turn", &message);
                }
            }
        });
        let abort = task.abort_handle();
        if let Some(slot) = self.inner.managed_turns.lock().get_mut(&identity) {
            *slot = Some(abort);
        }
        Ok(AgentTurnAccepted {
            session_id: identity.session_id,
            turn_id: identity.turn_id,
            generation: identity.generation,
        })
    }

    pub fn steer_turn_with_sink(
        &self,
        session_id: AgentSessionId,
        turn_id: AgentTurnId,
        generation: AgentGeneration,
        submission: impl Into<crate::AgentSteeringSubmission>,
        event_sink: &dyn AgentEventSink,
    ) -> Result<AgentSteeringAccepted, AgentError> {
        const MAXIMUM_STEERING_INPUT_BYTES: usize = 64 * 1024;
        let submission = submission.into();
        let user_message = submission.text;
        let images = submission.images;
        if user_message.trim().is_empty() && images.is_empty() {
            return Err(AgentError::InvalidInput(
                "steering message must contain text or an image".to_string(),
            ));
        }
        enforce_size(
            "steering input bytes",
            user_message.len(),
            MAXIMUM_STEERING_INPUT_BYTES,
        )?;
        if !self.inner.accepting.load(Ordering::Acquire) {
            return Err(AgentError::ShuttingDown);
        }
        self.refresh_session_image_input_capability_blocking(session_id)?;
        let identity = TurnIdentity {
            session_id,
            turn_id,
            generation,
        };
        let cell = self.session(session_id)?;
        let mut guard = cell.lock_after_persistence_blocking()?;
        let record = &mut *guard;
        if !images.is_empty() && !record.settings.effective_input_capabilities.image_input {
            return Err(AgentError::InvalidInput(
                "the configured model does not support image input".to_string(),
            ));
        }
        ensure_no_staged_commit(record)?;
        let previous = record.rollback_state();
        if record.status.lifecycle == AgentSessionLifecycle::Archived {
            return Err(AgentError::SessionArchived);
        }
        if record.status.disposing
            || record
                .runtime
                .compaction
                .as_ref()
                .is_some_and(|compaction| !compaction.snapshot.status.is_terminal())
        {
            return Err(AgentError::SessionMutationNotConfirmed);
        }
        let policy = record.settings.budget_policy;
        let now = now_millis();
        let turn = exact_turn_mut(record, identity)?;
        if turn.phase.is_terminal() || turn.cancellation.is_cancelled() {
            return Err(AgentError::TurnNotRunning);
        }
        if matches!(turn.kind, TurnKind::UserShellCommand(_)) {
            return Err(AgentError::InvalidInput(
                "a `!` command's turn reads no messages; send it once the command ends".to_string(),
            ));
        }
        let sequence = AgentSteeringSequence::new(turn.next_steering_sequence);
        let next_sequence =
            turn.next_steering_sequence
                .checked_add(1)
                .ok_or(AgentError::UsageOverflow {
                    field: "steering sequence",
                })?;
        let mut ledger = turn.budget_ledger;
        ledger
            .admit_pending_input_items(1, policy)
            .map_err(map_budget_error)?;
        turn.budget_ledger = ledger;
        turn.next_steering_sequence = next_sequence;
        turn.pending_steering.push_back(AgentSteeringInput {
            sequence,
            text: user_message,
            images,
            host_context: submission.host_context,
            accepted_at: now,
        });
        turn.updated_at = now;
        record.status.updated_at = now;
        record.status.revision = record
            .status
            .revision
            .checked_add(1)
            .expect("agent snapshot revision exhausted");
        let event = record_event(
            record,
            identity,
            AgentEventKind::SteeringAccepted { sequence },
            self.inner.limits,
        );
        let revision = AgentRevision::new(record.status.revision);
        let staged = StagedPersistence::commit(&cell, record, previous);
        drop(guard);
        self.persist_staged_blocking(staged)?;
        event_sink.emit(event);
        Ok(AgentSteeringAccepted {
            session_id,
            turn_id,
            generation,
            sequence,
            revision,
        })
    }
}
