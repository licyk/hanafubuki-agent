use super::*;

impl AgentService {
    pub fn resolve_user_input(
        &self,
        session_id: AgentSessionId,
        turn_id: AgentTurnId,
        generation: AgentGeneration,
        request_id: AgentUserInputRequestId,
        answers: Vec<AgentUserInputAnswer>,
    ) -> Result<AgentUserInputAccepted, AgentError> {
        let request = {
            let cell = self.session(session_id)?;
            let guard = cell.lock()?;
            let record = &*guard;
            let turn = record
                .runtime
                .turn
                .as_ref()
                .ok_or(AgentError::UserInputNotFound)?;
            if turn.id != turn_id || turn.generation != generation || turn.phase.is_terminal() {
                return Err(AgentError::StaleUserInput);
            }
            let request = record
                .runtime
                .pending_user_input
                .clone()
                .ok_or(AgentError::UserInputNotFound)?;
            if request.request_id != request_id {
                return Err(AgentError::StaleUserInput);
            }
            validate_user_input_answers(&request, &answers)?;
            request
        };
        let mut routes = self.inner.pending_user_inputs.lock();
        let route = routes
            .get_mut(&request_id)
            .ok_or(AgentError::StaleUserInput)?;
        if route.identity
            != (TurnIdentity {
                session_id,
                turn_id,
                generation,
            })
            || route.call_id != request.call_id
            || route.request != request
        {
            return Err(AgentError::StaleUserInput);
        }
        let resolution = AgentUserInputResolution {
            request_id,
            answers,
            resolved_at: now_millis(),
        };
        route
            .sender
            .take()
            .ok_or(AgentError::StaleUserInput)?
            .send(resolution)
            .map_err(|_| AgentError::StaleUserInput)?;
        Ok(AgentUserInputAccepted {
            session_id,
            turn_id,
            generation,
            request_id,
        })
    }

    pub(in crate::service) async fn request_user_input(
        &self,
        identity: TurnIdentity,
        call_id: String,
        call: RequestUserInputTool,
        cancellation: CancellationToken,
        event_sink: &dyn AgentEventSink,
    ) -> Result<AgentToolOutput, AgentError> {
        let request_id = AgentUserInputRequestId::new(
            self.inner
                .next_user_input_request_id
                .fetch_add(1, Ordering::Relaxed),
        );
        let request = AgentUserInputRequest {
            request_id,
            call_id: call_id.clone(),
            questions: call.questions,
            requested_at: now_millis(),
        };
        let (sender, mut receiver) = tokio::sync::oneshot::channel();
        let (event, staged) = {
            let cell = self.session(identity.session_id)?;
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let previous = record.rollback_state();
            if record.runtime.pending_user_input.is_some() {
                return Ok(AgentToolOutput::model_visible_error(json!({
                    "error": "another user input request is already pending"
                })));
            }
            let turn = exact_turn_mut(record, identity)?;
            turn.phase = AgentTurnPhase::WaitingForUserInput;
            turn.updated_at = now_millis();
            record.runtime.pending_user_input = Some(request.clone());
            let event = record_event(
                record,
                identity,
                AgentEventKind::UserInputRequested {
                    request: request.clone(),
                },
                self.inner.limits,
            );
            (event, StagedPersistence::commit(&cell, record, previous))
        };
        self.persist_staged(staged).await?;
        self.inner.pending_user_inputs.lock().insert(
            request_id,
            PendingUserInputRoute {
                identity,
                call_id,
                request: request.clone(),
                sender: Some(sender),
            },
        );
        event_sink.emit(event);

        let resolution = loop {
            tokio::select! {
                result = &mut receiver => {
                    match result {
                        Ok(resolution) => break resolution,
                        Err(_) => {
                            self.abandon_user_input(identity, request_id).await;
                            return Err(AgentError::Cancelled);
                        }
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(50)) => {
                    if cancellation.is_cancelled() {
                        self.abandon_user_input(identity, request_id).await;
                        return Err(AgentError::Cancelled);
                    }
                }
            }
        };
        self.inner.pending_user_inputs.lock().remove(&request_id);
        let (event, staged) = {
            let cell = self.session(identity.session_id)?;
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let previous = record.rollback_state();
            if record.runtime.pending_user_input.as_ref() != Some(&request) {
                return Err(AgentError::StaleUserInput);
            }
            let turn = exact_turn_mut(record, identity)?;
            turn.phase = AgentTurnPhase::RunningTools;
            turn.updated_at = now_millis();
            record.runtime.pending_user_input = None;
            let event = record_event(
                record,
                identity,
                AgentEventKind::UserInputResolved {
                    resolution: resolution.clone(),
                },
                self.inner.limits,
            );
            (event, StagedPersistence::commit(&cell, record, previous))
        };
        if let Err(error) = self.persist_staged(staged).await {
            self.abandon_user_input(identity, request_id).await;
            return Err(error);
        }
        event_sink.emit(event);
        Ok(AgentToolOutput::success(
            json!({ "answers": resolution.answers }),
        ))
    }

    pub(super) async fn abandon_user_input(
        &self,
        identity: TurnIdentity,
        request_id: AgentUserInputRequestId,
    ) {
        self.inner.pending_user_inputs.lock().remove(&request_id);
        let Some(cell) = self.inner.sessions.find(identity.session_id) else {
            return;
        };
        let Ok(mut record) = cell.lock_after_persistence().await else {
            return;
        };
        if record.status.generation != identity.generation {
            return;
        }
        if record
            .runtime
            .pending_user_input
            .as_ref()
            .is_some_and(|request| request.request_id == request_id)
        {
            record.runtime.pending_user_input = None;
            record.status.updated_at = now_millis();
        }
    }

    pub(in crate::service) async fn abort_pending_user_inputs(&self, identity: TurnIdentity) {
        let request_ids = self
            .inner
            .pending_user_inputs
            .lock()
            .iter()
            .filter_map(|(request_id, route)| (route.identity == identity).then_some(*request_id))
            .collect::<Vec<_>>();
        for request_id in request_ids {
            self.inner.pending_user_inputs.lock().remove(&request_id);
            let Some(cell) = self.inner.sessions.find(identity.session_id) else {
                continue;
            };
            let Ok(mut record) = cell.lock_after_persistence().await else {
                continue;
            };
            if exact_turn(&record, identity.turn_id, identity.generation).is_err() {
                continue;
            }
            if record
                .runtime
                .pending_user_input
                .as_ref()
                .is_some_and(|request| request.request_id == request_id)
            {
                record.runtime.pending_user_input = None;
                record.status.updated_at = now_millis();
            }
        }
    }
}
