use super::*;

impl AgentService {
    pub fn rename_session(
        &self,
        request: AgentSessionTitleRenameRequest,
        event_sink: &dyn AgentEventSink,
    ) -> Result<AgentSessionRuntimeSnapshot, AgentError> {
        let title = AgentSessionTitle::user(request.title).map_err(AgentError::InvalidInput)?;
        self.update_session_title(
            request.session_id,
            Some((request.generation, request.revision)),
            |_| true,
            title,
            |event| event_sink.emit(event),
        )
        // `update_session_title` returns `None` only when `may_update`
        // refuses, and this one always accepts.
        .map(|snapshot| snapshot.expect("manual title mutation always updates"))
    }

    /// Replaces the session's automatic title with `title` only while it is
    /// still the automatic title `expected`, so a rename the user made in the
    /// meantime wins. Returns `None` when it did not replace it. The event
    /// goes to the sink of the session's managed turn, if it has one.
    pub fn replace_automatic_session_title_if_expected(
        &self,
        session_id: AgentSessionId,
        expected: &str,
        title: String,
    ) -> Result<Option<AgentSessionRuntimeSnapshot>, AgentError> {
        let title = AgentSessionTitle::automatic(title).map_err(AgentError::InvalidInput)?;
        self.update_session_title(
            session_id,
            None,
            |current| {
                current.as_ref().is_some_and(|current| {
                    current.source == AgentSessionTitleSource::Automatic
                        && current.value == expected
                })
            },
            title,
            |event| self.emit_session_event(session_id, event),
        )
    }

    fn update_session_title(
        &self,
        session_id: AgentSessionId,
        expected_identity: Option<(AgentGeneration, AgentRevision)>,
        may_update: impl FnOnce(&Option<AgentSessionTitle>) -> bool,
        title: AgentSessionTitle,
        emit: impl FnOnce(AgentEvent),
    ) -> Result<Option<AgentSessionRuntimeSnapshot>, AgentError> {
        let cell = self.session(session_id)?;
        let mut guard = cell.lock_after_persistence_blocking()?;
        let record = &mut *guard;
        ensure_no_staged_commit(record)?;
        if let Some((generation, revision)) = expected_identity {
            if record.status.generation != generation {
                return Err(AgentError::StaleGeneration);
            }
            if record.status.revision != revision.value() {
                return Err(AgentError::StaleRevision);
            }
        }
        if !may_update(&record.settings.title) {
            return Ok(None);
        }
        if record.settings.title.as_ref() == Some(&title) {
            return Ok(Some(snapshot(session_id, record)));
        }
        let previous = record.rollback_state();
        record.settings.title = Some(title.clone());
        let event = record_session_event(
            record,
            AgentEventKind::SessionTitleUpdated { title },
            self.inner.limits,
        );
        let staged = StagedPersistence::commit(&cell, record, previous);
        drop(guard);
        self.persist_staged_blocking(staged)?;
        emit(event);
        self.snapshot(session_id).map(Some)
    }
}
