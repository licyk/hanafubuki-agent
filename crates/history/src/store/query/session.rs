use super::cursor::{decode_cursor, encode_cursor, validate_page_size, SessionCursor};
use crate::store::*;

impl FileAgentHistoryStore {
    pub(in crate::store) fn lookup_rollout_path(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Option<PathBuf>, AgentHistoryStoreError> {
        let _database = self.inner.database_gate.lock();
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT rollout_path FROM agent_sessions WHERE session_id = ?",
                params![to_i64(session_id.value(), "session id")?],
                |row| row.get::<_, String>(0).map(PathBuf::from),
            )
            .optional()
            .map_err(database_error)
    }

    /// Each candidate loads on its own, so one corrupt runtime projection or
    /// rollout fails only its own entry; reading the index fails the call.
    /// Every session whose projection is unavailable (`projection_lag`: one
    /// open could not admit, whatever its recovery state) is a failure too,
    /// so recovery lists each such session once.
    pub(in crate::store) fn load_recovery_candidates_inner(
        &self,
    ) -> Result<Vec<Result<AgentRecoveredSession, AgentRecoveryLoadFailure>>, AgentHistoryStoreError>
    {
        let _database = self.inner.database_gate.lock();
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT session_id, checkpoint_revision, runtime_json FROM agent_sessions WHERE lifecycle = 'available' AND recovery_needed = 1 ORDER BY session_id",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(database_error)?;
        let encoded = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?;
        drop(statement);
        drop(connection);
        drop(_database);
        let unavailable = self
            .inner
            .projection_lag
            .lock()
            .iter()
            .map(|(session_id, lag)| (*session_id, lag.reason().to_string()))
            .collect::<HashMap<_, _>>();
        let mut recovered = Vec::with_capacity(encoded.len() + unavailable.len());
        for (session_id, checkpoint_revision, runtime_json) in encoded {
            let session_id = AgentSessionId::new(from_i64(session_id, "session id")?);
            if unavailable.contains_key(&session_id) {
                continue;
            }
            recovered.push(
                self.load_recovery_candidate(session_id, checkpoint_revision, &runtime_json)
                    .map_err(|error| AgentRecoveryLoadFailure { session_id, error }),
            );
        }
        recovered.extend(unavailable.into_iter().map(|(session_id, reason)| {
            Err(AgentRecoveryLoadFailure {
                session_id,
                error: AgentHistoryStoreError::ProjectionUnavailable(reason),
            })
        }));
        Ok(recovered)
    }

    fn load_recovery_candidate(
        &self,
        session_id: AgentSessionId,
        checkpoint_revision: i64,
        runtime_json: &str,
    ) -> Result<AgentRecoveredSession, AgentHistoryStoreError> {
        let runtime: hanafubuki_agent_kernel::PersistedAgentRuntimeState =
            serde_json::from_str(runtime_json).map_err(json_error)?;
        if runtime.revision.value() != from_i64(checkpoint_revision, "checkpoint revision")? {
            return Err(AgentHistoryStoreError::CorruptHistory(
                "projected runtime does not match its checkpoint revision".to_string(),
            ));
        }
        let model_context = self.load_latest_model_context_inner(session_id)?;
        Ok(AgentRecoveredSession {
            runtime,
            model_context,
        })
    }

    pub(in crate::store) fn load_runtime_state_inner(
        &self,
        session_id: AgentSessionId,
    ) -> Result<hanafubuki_agent_kernel::PersistedAgentRuntimeState, AgentHistoryStoreError> {
        if let Some(reason) = self.inner.projection_lag.lock().get(&session_id) {
            return Err(AgentHistoryStoreError::ProjectionUnavailable(
                reason.reason().to_string(),
            ));
        }
        let _database = self.inner.database_gate.lock();
        let connection = self.connection()?;
        let (checkpoint_revision, encoded) = connection
            .query_row(
                "SELECT checkpoint_revision, runtime_json FROM agent_sessions WHERE session_id = ?",
                params![to_i64(session_id.value(), "session id")?],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(database_error)?
            .ok_or(AgentHistoryStoreError::NotFound { session_id })?;
        let runtime: hanafubuki_agent_kernel::PersistedAgentRuntimeState =
            serde_json::from_str(&encoded).map_err(json_error)?;
        if runtime.revision.value() != from_i64(checkpoint_revision, "checkpoint revision")? {
            return Err(AgentHistoryStoreError::CorruptHistory(
                "projected runtime does not match its checkpoint revision".to_string(),
            ));
        }
        Ok(runtime)
    }

    pub(in crate::store) fn load_head_revision_inner(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentRevision, AgentHistoryStoreError> {
        let _database = self.inner.database_gate.lock();
        let connection = self.connection()?;
        let revision = connection
            .query_row(
                "SELECT revision FROM agent_sessions WHERE session_id = ?",
                params![to_i64(session_id.value(), "session id")?],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(database_error)?
            .ok_or(AgentHistoryStoreError::NotFound { session_id })?;
        Ok(AgentRevision::new(from_i64(revision, "session revision")?))
    }

    pub(in crate::store) fn list_sessions_page_inner(
        &self,
        request: AgentSessionListRequest,
    ) -> Result<AgentSessionPage, AgentHistoryStoreError> {
        validate_page_size(request.page_size, AGENT_SESSION_PAGE_SIZE_MAX)?;
        let cursor = request
            .cursor
            .as_deref()
            .map(decode_cursor::<SessionCursor>)
            .transpose()?;
        if let Some(cursor) = &cursor {
            if cursor.version != 1
                || cursor.lifecycle != request.lifecycle
                || cursor.direction != request.direction
                || cursor.search != request.search
            {
                return Err(AgentHistoryStoreError::InvalidCursor(
                    "session cursor does not match the request".to_string(),
                ));
            }
        }
        let lifecycle = lifecycle_name(request.lifecycle);
        let search = request.search.as_ref().map(|value| format!("%{value}%"));
        let comparator = match request.direction {
            AgentHistorySortDirection::Ascending => ">",
            AgentHistorySortDirection::Descending => "<",
        };
        let order = match request.direction {
            AgentHistorySortDirection::Ascending => "ASC",
            AgentHistorySortDirection::Descending => "DESC",
        };
        let mut sql = String::from(
            "SELECT updated_at, session_id, summary_json FROM agent_sessions WHERE lifecycle = ?",
        );
        if search.is_some() {
            sql.push_str(" AND title LIKE ?");
        }
        if cursor.is_some() {
            sql.push_str(&format!(
                " AND (updated_at {comparator} ? OR (updated_at = ? AND session_id {comparator} ?))"
            ));
        }
        sql.push_str(&format!(
            " ORDER BY updated_at {order}, session_id {order} LIMIT ?"
        ));
        let _database = self.inner.database_gate.lock();
        let connection = self.connection()?;
        let mut statement = connection.prepare(&sql).map_err(database_error)?;
        let limit = to_i64(
            request.page_size.checked_add(1).ok_or_else(|| {
                AgentHistoryStoreError::InvalidCursor("page size overflow".to_string())
            })? as u64,
            "page size",
        )?;
        let mut rows = match (&search, &cursor) {
            (Some(search), Some(cursor)) => statement
                .query(params![
                    lifecycle,
                    search,
                    cursor.updated_at.to_string(),
                    cursor.updated_at.to_string(),
                    to_i64(cursor.session_id.value(), "session id")?,
                    limit,
                ])
                .map_err(database_error)?,
            (Some(search), None) => statement
                .query(params![lifecycle, search, limit])
                .map_err(database_error)?,
            (None, Some(cursor)) => statement
                .query(params![
                    lifecycle,
                    cursor.updated_at.to_string(),
                    cursor.updated_at.to_string(),
                    to_i64(cursor.session_id.value(), "session id")?,
                    limit,
                ])
                .map_err(database_error)?,
            (None, None) => statement
                .query(params![lifecycle, limit])
                .map_err(database_error)?,
        };
        // A row whose summary does not decode keeps its place in the paging
        // (the cursor may stop on it) but is left out of the page.
        let mut entries = Vec::new();
        while let Some(row) = rows.next().map_err(database_error)? {
            let updated_at: String = row.get(0).map_err(database_error)?;
            let session_id: i64 = row.get(1).map_err(database_error)?;
            let summary_json: String = row.get(2).map_err(database_error)?;
            entries.push((
                updated_at.parse::<u128>().map_err(|error| {
                    AgentHistoryStoreError::CorruptHistory(format!(
                        "invalid session timestamp: {error}"
                    ))
                })?,
                AgentSessionId::new(from_i64(session_id, "session id")?),
                serde_json::from_str::<AgentSessionSummary>(&summary_json),
            ));
        }
        drop(rows);
        drop(statement);
        drop(connection);
        drop(_database);
        for (_, session_id, summary) in &entries {
            if let Err(error) = summary {
                self.refuse_unlisted_session(*session_id, error);
            }
        }
        let has_more = entries.len() > request.page_size;
        entries.truncate(request.page_size);
        let next_cursor = if has_more {
            entries
                .last()
                .map(|(updated_at, session_id, _)| {
                    encode_cursor(&SessionCursor {
                        version: 1,
                        updated_at: *updated_at,
                        session_id: *session_id,
                        lifecycle: request.lifecycle,
                        direction: request.direction,
                        search: request.search.clone(),
                    })
                })
                .transpose()?
        } else {
            None
        };
        Ok(AgentSessionPage {
            sessions: entries
                .into_iter()
                .filter_map(|(_, _, summary)| summary.ok())
                .collect(),
            next_cursor,
        })
    }

    /// Refuses a session the list cannot show: its reads are refused like
    /// those of a session `open` could not admit, and its files are kept.
    /// Logged and told to the refusal sink when first refused. Rebuilding
    /// the index projects the summary again from the rollout.
    fn refuse_unlisted_session(&self, session_id: AgentSessionId, error: &serde_json::Error) {
        let reason = format!("the session summary in the history index does not decode: {error}");
        {
            let mut lag = self.inner.projection_lag.lock();
            let std::collections::hash_map::Entry::Vacant(entry) = lag.entry(session_id) else {
                return;
            };
            entry.insert(ProjectionLag::Refused(reason.clone()));
        }
        log::error!(
            "Agent session {session_id} quarantined: the session list leaves it out; its files are kept: {reason}"
        );
        self.inner
            .refusals
            .session_refused(hanafubuki_agent_kernel::AgentQuarantinedSession {
                session_id,
                reason,
            });
    }
}
