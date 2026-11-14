use super::*;

impl AgentService {
    pub fn recover_sessions(&self) -> Result<crate::AgentRecoveryReport, AgentError> {
        self.recover_sessions_with_model(Arc::new(UnavailableRecoveredModel))
    }

    /// `recover_sessions`, with `model` as the recovered sessions' model.
    #[cfg(any(test, feature = "test-support"))]
    pub fn recover_sessions_with_test_model(
        &self,
        model: Arc<dyn ModelPort>,
    ) -> Result<crate::AgentRecoveryReport, AgentError> {
        self.recover_sessions_with_model(model)
    }

    /// Loads every recoverable session, oldest id first, settling what the
    /// previous process left running. A session that fails to load,
    /// validate or settle is quarantined: logged, listed in the report and
    /// left untouched on disk, while the others still load. Only a failure
    /// to read the store's index fails recovery as a whole.
    fn recover_sessions_with_model(
        &self,
        model: Arc<dyn ModelPort>,
    ) -> Result<crate::AgentRecoveryReport, AgentError> {
        let mut persisted = self
            .inner
            .history_store
            .load_recovery_candidates()
            .map_err(history_store_error)?;
        persisted.sort_by_key(|candidate| match candidate {
            Ok(session) => session.runtime.id,
            Err(failure) => failure.session_id,
        });
        if !self.inner.sessions.is_empty() {
            return Err(AgentError::Persistence(
                "Agent recovery must run before sessions are created".to_string(),
            ));
        }
        let mut seen = HashSet::new();
        let mut report = crate::AgentRecoveryReport::default();
        for candidate in persisted {
            let (session_id, outcome) = match candidate {
                Ok(recovered) => {
                    let session_id = recovered.runtime.id;
                    let outcome = if seen.insert(session_id) {
                        self.recover_session(recovered, &model, &mut report)
                    } else {
                        Err(AgentError::Persistence(format!(
                            "duplicate Agent session {session_id}"
                        )))
                    };
                    (session_id, outcome)
                }
                Err(failure) => (failure.session_id, Err(history_store_error(failure.error))),
            };
            match outcome {
                Ok(()) => report.restored = report.restored.saturating_add(1),
                Err(error) => {
                    log::error!(
                        "Agent session {session_id} quarantined: recovery failed, its files are kept: {error}"
                    );
                    report.quarantined.push(crate::AgentQuarantinedSession {
                        session_id,
                        reason: error.to_string(),
                    });
                }
            }
        }
        Ok(report)
    }

    /// Validates one persisted session, settles it, registers it and commits
    /// the settlement. A session whose commit fails is not kept.
    fn recover_session(
        &self,
        mut recovered: crate::AgentRecoveredSession,
        model: &Arc<dyn ModelPort>,
        report: &mut crate::AgentRecoveryReport,
    ) -> Result<(), AgentError> {
        let contents = history_contents(&recovered.model_context);
        self.validate_persisted_session(
            &mut recovered.runtime,
            &recovered.model_context,
            &contents,
        )?;
        validate_recoverable_turn(&recovered.runtime)?;
        let interrupted = recovered
            .runtime
            .turn
            .as_ref()
            .is_some_and(|turn| !turn.phase.is_terminal());
        let executions = recovered
            .runtime
            .activities
            .iter()
            .filter(|activity| !activity.phase.is_terminal())
            .filter_map(|activity| {
                let execution = activity.execution.as_ref()?;
                let call_id = execution.initiating_call_id.as_deref()?;
                if crate::service::tool_execution::is_user_shell_call_id(call_id) {
                    return None;
                }
                (!execution.status.is_terminal()).then(|| crate::AgentInterruptedExecution {
                    session_id: recovered.runtime.id,
                    execution_id: execution.execution_id,
                    command: activity.presentation.subject.clone().unwrap_or_default(),
                })
            })
            .collect::<Vec<_>>();
        let mut record = self.persisted_record(
            recovered.runtime,
            recovered.model_context,
            &contents,
            model.clone(),
        );
        let repaired = repair_after_restart(&mut record)?;
        let cell = self.register_recovered(record)?;
        if repaired {
            if let Err(error) = self.persist_current_conversation(&cell, false) {
                self.remove_unpersisted_session(&cell);
                return Err(error);
            }
        }
        if interrupted {
            report.interrupted_turns.push(cell.session_id());
        }
        report.interrupted_executions.extend(executions);
        Ok(())
    }

    /// Adds a recovered session and moves the turn and compaction id
    /// counters past every id it used.
    fn register_recovered(&self, record: SessionRecord) -> Result<Arc<SessionCell>, AgentError> {
        let turn_ids = record.runtime.turn.iter().map(|turn| turn.id).chain(
            record
                .conversation
                .history
                .iter()
                .filter_map(|item| match &item.content {
                    ConversationItem::TurnStart { turn_id } => Some(*turn_id),
                    _ => None,
                }),
        );
        for turn_id in turn_ids {
            self.inner.next_turn_id.fetch_max(
                turn_id
                    .value()
                    .checked_add(1)
                    .expect("agent turn id exhausted"),
                Ordering::AcqRel,
            );
        }
        if let Some(compaction) = &record.runtime.compaction {
            self.inner.next_compaction_id.fetch_max(
                compaction
                    .snapshot
                    .id
                    .value()
                    .checked_add(1)
                    .expect("agent compaction id exhausted"),
                Ordering::AcqRel,
            );
        }
        self.inner.sessions.insert(record, &self.inner.accepting)
    }
}

/// What only a restart may find: a turn from another generation, steering
/// with an invalid host context, or an archived session with a running turn.
fn validate_recoverable_turn(session: &PersistedAgentRuntimeState) -> Result<(), AgentError> {
    let Some(turn) = session.turn.as_ref() else {
        return Ok(());
    };
    if turn.generation != session.generation {
        return Err(AgentError::Persistence(format!(
            "Agent session {} has a turn from a different generation",
            session.id
        )));
    }
    for input in &turn.pending_steering {
        if let Some(context) = input.host_context.as_ref() {
            validate_host_context(context)?;
        }
    }
    if session.lifecycle == AgentSessionLifecycle::Archived && !turn.phase.is_terminal() {
        return Err(AgentError::Persistence(format!(
            "archived Agent session {} has a non-terminal turn",
            session.id
        )));
    }
    Ok(())
}
