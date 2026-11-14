use super::*;

impl AgentService {
    /// Whether the kernel still admits operations; false throughout shutdown.
    pub fn is_accepting(&self) -> bool {
        self.inner.accepting.load(Ordering::Acquire)
    }

    /// Off the async runtime only: it waits, blocking, until `deadline`.
    pub fn shutdown(&self, deadline: Instant) -> Vec<AgentShutdownDiagnostic> {
        let mut diagnostics = self.stop_operations(deadline);
        diagnostics.extend(self.shutdown_history(deadline));
        diagnostics
    }

    /// Stops accepting work, cancels every running turn and compaction, the
    /// background ones included, and waits, blocking, until each session has
    /// settled and every managed turn and background compaction task has
    /// ended, or `deadline` passes. Off the async runtime only (the
    /// application's shutdown thread).
    pub fn stop_operations(&self, deadline: Instant) -> Vec<AgentShutdownDiagnostic> {
        self.inner.accepting.store(false, Ordering::Release);
        let mut cancelled_turns = Vec::new();
        for cell in self.inner.sessions.cells() {
            let Ok(mut record) = cell.lock() else {
                continue;
            };
            record.runtime.background_compaction.cancel();
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
        // Its usage is recorded once it ends, so it is waited for too.
        let background_running = !self.wait_for_background_tasks(None, deadline);
        if unsettled || managed_turn_running || background_running {
            vec![AgentShutdownDiagnostic {
                message: "shutdown deadline elapsed with an active Agent turn, managed worker, compaction, background compaction, or history commit".to_string(),
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
        if self.inner.history_store.is_ephemeral(session_id) {
            return Err(AgentError::InvalidInput(format!(
                "Agent session {session_id} is ephemeral: it is closed, never archived"
            )));
        }
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
        if next == AgentSessionLifecycle::Archived {
            record.runtime.background_compaction.cancel();
        }
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
        if next == AgentSessionLifecycle::Archived {
            self.await_cancelled_background_compaction(session_id);
        }
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
        let prepared = self.prepare_session_deletion(session_id, generation, revision, &[])?;
        self.complete_session_deletion_confirmed(prepared)
    }

    /// Prepares deleting `session_id` and, in the same deletion, the loaded
    /// `descendants` at their current revisions.
    pub fn prepare_session_deletion(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
        descendants: &[AgentSessionId],
    ) -> Result<PreparedAgentSessionDeletion, AgentError> {
        let session_ids = std::iter::once(session_id)
            .chain(descendants.iter().copied())
            .collect::<Vec<_>>();
        let cells = session_ids
            .iter()
            .map(|session_id| self.session(*session_id))
            .collect::<Result<Vec<_>, _>>()?;
        // A fork, or a kept file the index does not hold, that still needs
        // a session outside the deletion refuses it here, before the caller
        // lets go of anything else of them (their memory, their terminals).
        self.inner
            .history_store
            .check_sessions_deletable(&session_ids)
            .map_err(|error| AgentError::Persistence(error.to_string()))?;
        let mut prepared = PreparedAgentSessionDeletion {
            sessions: Vec::with_capacity(cells.len()),
        };
        for cell in &cells {
            let marked = cell
                .lock_after_persistence_blocking()
                .and_then(|mut record| {
                    if cell.session_id() == session_id {
                        validate_session_mutation(&record, generation, revision)?;
                    }
                    record.status.disposing = true;
                    record.runtime.background_compaction.cancel();
                    Ok(ConfirmedAgentSessionDeletion {
                        session_id: cell.session_id(),
                        generation: record.status.generation,
                        revision: AgentRevision::new(record.status.revision),
                    })
                });
            match marked {
                Ok(member) => prepared.sessions.push(member),
                Err(error) => {
                    self.cancel_prepared_session_deletion(prepared);
                    return Err(error);
                }
            }
        }
        Ok(prepared)
    }

    pub fn cancel_prepared_session_deletion(&self, prepared: PreparedAgentSessionDeletion) {
        for member in prepared.sessions {
            let Some(cell) = self.inner.sessions.find(member.session_id) else {
                continue;
            };
            let Ok(mut record) = cell.lock_after_persistence_blocking() else {
                continue;
            };
            if is_prepared(&record, &member) {
                record.status.disposing = false;
            }
        }
    }

    /// Deletes every session of `prepared` from the history store together,
    /// then lets go of each. Returns the first, the one the deletion was
    /// prepared for. Every member is checked first, so nothing is deleted
    /// unless all of them still stand as prepared (a disposing session
    /// admits no turn, so none can start meanwhile).
    pub fn complete_session_deletion_confirmed(
        &self,
        prepared: PreparedAgentSessionDeletion,
    ) -> Result<ConfirmedAgentSessionDeletion, AgentError> {
        let mut cells = Vec::with_capacity(prepared.sessions.len());
        for member in prepared.sessions.clone() {
            let cell = match self.session(member.session_id) {
                Ok(cell) => cell,
                Err(error) => {
                    self.cancel_prepared_session_deletion(prepared);
                    return Err(error);
                }
            };
            let ready = cell.lock().is_ok_and(|record| {
                is_prepared(&record, &member)
                    && !record
                        .runtime
                        .turn
                        .as_ref()
                        .is_some_and(|turn| !turn.phase.is_terminal())
            });
            if !ready {
                self.cancel_prepared_session_deletion(prepared);
                return Err(AgentError::SessionMutationNotConfirmed);
            }
            cells.push(cell);
        }
        let session_ids = prepared
            .sessions
            .iter()
            .map(|member| member.session_id)
            .collect::<Vec<_>>();
        for session_id in &session_ids {
            self.await_cancelled_background_compaction(*session_id);
        }
        if let Err(error) = self
            .inner
            .history_store
            .delete_sessions_confirmed(&session_ids)
        {
            self.cancel_prepared_session_deletion(prepared);
            return Err(AgentError::Persistence(error.to_string()));
        }
        for cell in cells {
            if let Ok(record) = cell.lock() {
                self.inner.sessions.remove(record);
            }
            self.inner.event_sinks.lock().remove(&cell.session_id());
        }
        Ok(prepared.sessions[0])
    }
}

fn is_prepared(record: &SessionRecord, member: &ConfirmedAgentSessionDeletion) -> bool {
    record.status.generation == member.generation
        && record.status.revision == member.revision.value()
        && record.status.disposing
}
