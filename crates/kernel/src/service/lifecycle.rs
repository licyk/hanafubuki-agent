use super::*;

impl AgentService {
    /// Off the async runtime only: it waits, blocking, until `deadline`.
    pub fn shutdown(&self, deadline: Instant) -> Vec<AgentShutdownDiagnostic> {
        let mut diagnostics = self.stop_operations(deadline);
        diagnostics.extend(self.shutdown_history(deadline));
        diagnostics
    }

    /// Stops accepting work, cancels every running turn and compaction and
    /// waits, blocking, until each session has settled and every managed turn
    /// task has ended, or `deadline` passes. Off the async runtime only (the
    /// application's shutdown thread).
    pub fn stop_operations(&self, deadline: Instant) -> Vec<AgentShutdownDiagnostic> {
        self.inner.accepting.store(false, Ordering::Release);
        let mut cancelled_turns = Vec::new();
        for cell in self.inner.sessions.cells() {
            let Ok(record) = cell.lock() else {
                continue;
            };
            if let Some(turn) = &record.runtime.turn {
                if !turn.phase.is_terminal() {
                    turn.cancellation.cancel();
                    cancelled_turns.push(TurnIdentity {
                        session_id: cell.session_id(),
                        turn_id: turn.id,
                        generation: turn.generation,
                    });
                }
            }
            if let Some(compaction) = &record.runtime.compaction {
                if !compaction.snapshot.status.is_terminal() {
                    compaction.cancellation.cancel();
                }
            }
        }
        for identity in cancelled_turns {
            self.abort_pending_approvals(identity);
        }
        let settled = |record: &SessionRecord| {
            Ok(
                (!has_active_operation(record) && record.status.persisting_revision.is_none())
                    .then_some(()),
            )
        };
        // A later session can only settle after the deadline if an earlier
        // one did not, so the waits share it. A cell removed meanwhile is
        // settled.
        let mut unsettled = false;
        for cell in self.inner.sessions.cells() {
            if let Ok(None) = cell.wait_until_blocking(deadline, settled) {
                unsettled = true;
            }
            // The progress queued until now, committed while history is open.
            if !cell.progress().flushed_blocking(deadline) {
                unsettled = true;
            }
        }
        let mut managed_turns = self.inner.managed_turns.lock();
        while !managed_turns.is_empty() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            self.inner
                .managed_turns_changed
                .wait_for(&mut managed_turns, remaining);
        }
        let managed_turn_running = !managed_turns.is_empty();
        drop(managed_turns);
        if unsettled || managed_turn_running {
            vec![AgentShutdownDiagnostic {
                message: "shutdown deadline elapsed with an active Agent turn, managed worker, compaction, or history commit".to_string(),
            }]
        } else {
            Vec::new()
        }
    }

    /// Closes every session's progress queue, stopping its consumer, and
    /// then the history store, whose writers get until `deadline` to stop.
    /// Every commit a turn saw acknowledged is already on disk; a writer
    /// still running at the deadline is reported, and only appends it had
    /// not acknowledged (never durable) are lost.
    pub fn shutdown_history(&self, deadline: Instant) -> Vec<AgentShutdownDiagnostic> {
        for cell in self.inner.sessions.cells() {
            cell.progress().close();
        }
        self.inner
            .history_store
            .shutdown(deadline)
            .into_iter()
            .map(|error| AgentShutdownDiagnostic {
                message: format!("Agent history shutdown failed: {error}"),
            })
            .collect()
    }

    pub fn archive_session(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
    ) -> Result<AgentSessionSummary, AgentError> {
        self.set_session_lifecycle(
            session_id,
            generation,
            revision,
            AgentSessionLifecycle::Available,
            AgentSessionLifecycle::Archived,
        )
    }

    pub fn restore_session(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
    ) -> Result<AgentSessionSummary, AgentError> {
        self.set_session_lifecycle(
            session_id,
            generation,
            revision,
            AgentSessionLifecycle::Archived,
            AgentSessionLifecycle::Available,
        )
    }

    fn set_session_lifecycle(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
        expected: AgentSessionLifecycle,
        next: AgentSessionLifecycle,
    ) -> Result<AgentSessionSummary, AgentError> {
        let cell = self.session(session_id)?;
        let mut guard = cell.lock_after_persistence_blocking()?;
        let record = &mut *guard;
        validate_session_mutation(record, generation, revision)?;
        if record.status.lifecycle != expected {
            return Err(if expected == AgentSessionLifecycle::Archived {
                AgentError::SessionNotArchived
            } else {
                AgentError::SessionArchived
            });
        }
        let previous = record.rollback_state();
        record.status.lifecycle = next;
        record.status.updated_at = now_millis();
        record.status.revision = record
            .status
            .revision
            .checked_add(1)
            .expect("agent snapshot revision exhausted");
        let committed_revision = record.status.revision;
        let staged = StagedPersistence::commit(&cell, record, previous);
        drop(guard);
        self.persist_staged_blocking(staged)?;
        let moved = match next {
            AgentSessionLifecycle::Archived => self
                .inner
                .history_store
                .archive_session(session_id, AgentRevision::new(committed_revision)),
            AgentSessionLifecycle::Available => self
                .inner
                .history_store
                .restore_session(session_id, AgentRevision::new(committed_revision)),
        };
        if let Err(error) = moved {
            return Err(AgentError::Persistence(error.to_string()));
        }
        // Both the lifecycle and the history move are durable, and no lock
        // is held.
        let revision = AgentRevision::new(committed_revision);
        self.inner.registries.observers.notify(&match next {
            AgentSessionLifecycle::Archived => AgentNotification::SessionArchived {
                session_id,
                revision,
            },
            AgentSessionLifecycle::Available => AgentNotification::SessionRestored {
                session_id,
                revision,
            },
        });
        let record = cell.lock()?;
        Ok(session_summary(session_id, &record))
    }

    pub fn delete_session_confirmed(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
    ) -> Result<ConfirmedAgentSessionDeletion, AgentError> {
        let prepared = self.prepare_session_deletion(session_id, generation, revision)?;
        self.complete_session_deletion_confirmed(prepared)
    }

    pub fn prepare_session_deletion(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
    ) -> Result<PreparedAgentSessionDeletion, AgentError> {
        let cell = self.session(session_id)?;
        // A fork, or a kept file the index does not hold, that still needs
        // the session refuses the deletion here, before the caller lets go of
        // anything else of it (its memory, its terminals).
        self.inner
            .history_store
            .check_sessions_deletable(&[session_id])
            .map_err(|error| AgentError::Persistence(error.to_string()))?;
        let mut record = cell.lock_after_persistence_blocking()?;
        validate_session_mutation(&record, generation, revision)?;
        record.status.disposing = true;
        Ok(PreparedAgentSessionDeletion {
            session_id,
            generation,
            revision,
        })
    }

    pub fn cancel_prepared_session_deletion(&self, prepared: PreparedAgentSessionDeletion) {
        let Some(cell) = self.inner.sessions.find(prepared.session_id) else {
            return;
        };
        let Ok(mut record) = cell.lock_after_persistence_blocking() else {
            return;
        };
        if record.status.generation == prepared.generation
            && record.status.revision == prepared.revision.value()
            && record.status.disposing
        {
            record.status.disposing = false;
        }
    }

    pub fn complete_session_deletion_confirmed(
        &self,
        prepared: PreparedAgentSessionDeletion,
    ) -> Result<ConfirmedAgentSessionDeletion, AgentError> {
        let session_id = prepared.session_id;
        let generation = prepared.generation;
        let revision = prepared.revision;
        if let Err(error) = self
            .inner
            .history_store
            .delete_sessions_confirmed(&[session_id])
        {
            if let Some(cell) = self.inner.sessions.find(session_id) {
                if let Ok(mut record) = cell.lock() {
                    if record.status.generation == generation
                        && record.status.revision == revision.value()
                    {
                        record.status.disposing = false;
                    }
                }
            }
            return Err(AgentError::Persistence(error.to_string()));
        }
        let cell = self.session(session_id)?;
        let record = cell.lock()?;
        if record.status.generation != generation
            || record.status.revision != revision.value()
            || !record.status.disposing
            || record
                .runtime
                .turn
                .as_ref()
                .is_some_and(|turn| !turn.phase.is_terminal())
        {
            return Err(AgentError::SessionMutationNotConfirmed);
        }
        self.inner.sessions.remove(record);
        self.inner.event_sinks.lock().remove(&session_id);
        Ok(ConfirmedAgentSessionDeletion {
            session_id,
            generation,
            revision,
        })
    }
}
