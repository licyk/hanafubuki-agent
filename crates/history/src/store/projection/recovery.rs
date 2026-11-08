use crate::store::jsonl::{
    collect_jsonl_files, read_complete_records, repair_partial_tail, session_id_from_rollout_path,
};
use crate::store::*;
use hanafubuki_agent_foundation::persistence::quarantine_file;

impl FileAgentHistoryStore {
    /// Rebuilds the index from every rollout under the active and archived
    /// trees. A rollout that cannot be projected costs only its own session:
    /// one of another format is moved aside with its attachments
    /// ([`Self::quarantine_rollout`]); any other failure leaves the file in
    /// place and records a pending repair, so every later start retries and
    /// reports it. Either way the session's id stays reserved and the session
    /// is refused (`projection_lag`) and listed by recovery.
    pub(in crate::store) fn catch_up_discovered_sessions(
        &self,
    ) -> Result<(), AgentHistoryStoreError> {
        let mut paths = Vec::new();
        collect_jsonl_files(&self.inner.paths.active_sessions, &mut paths)?;
        collect_jsonl_files(&self.inner.paths.archived_sessions, &mut paths)?;
        paths.sort();
        for path in paths {
            let session_id = session_id_from_rollout_path(&path);
            let result = (|| {
                repair_partial_tail(&path)?;
                let complete = read_complete_records(&path, 0)?;
                let Some(first) = complete.records.first() else {
                    return Ok(());
                };
                self.materialize_path(first.record.session_id, &path)
            })();
            let error = match result {
                Ok(()) => continue,
                Err(error) => error,
            };
            let incompatible = matches!(error, AgentHistoryStoreError::IncompatibleFormat { .. });
            let reason = if incompatible {
                self.quarantine_rollout(&path, session_id, &error)
            } else {
                log::error!(
                    "Agent history rebuild skipped corrupt rollout {}: {error}",
                    path.display()
                );
                error.to_string()
            };
            let Some(session_id) = session_id else {
                continue;
            };
            let recorded = self.reserve_session_id(session_id).and_then(|()| {
                if incompatible {
                    Ok(())
                } else {
                    self.mark_projection_pending(session_id, &path, AgentRevision::new(0))
                }
            });
            if let Err(error) = recorded {
                log::warn!(
                    "Agent history could not record the refused session {session_id}: {error}"
                );
            }
            self.inner
                .projection_lag
                .lock()
                .insert(session_id, ProjectionLag::Refused(reason));
        }
        Ok(())
    }

    /// Moves a rollout of another format, and the session's attachment
    /// directory, into a `.recovery` directory beside each
    /// (`quarantine_file`: `<name>.<pid>.<n>.invalid`, which no scan reads),
    /// and returns the reason the session is refused. Nothing is deleted; a
    /// move that fails leaves the file where it is.
    fn quarantine_rollout(
        &self,
        path: &Path,
        session_id: Option<AgentSessionId>,
        error: &AgentHistoryStoreError,
    ) -> String {
        let rollout = match quarantine_file(path) {
            Ok(target) => {
                if let Some(parent) = path.parent() {
                    if let Err(error) = sync_directory(parent) {
                        log::warn!("Agent history could not sync {}: {error}", parent.display());
                    }
                }
                target.map_or_else(
                    || path.display().to_string(),
                    |target| target.display().to_string(),
                )
            }
            Err(move_error) => {
                log::error!(
                    "Agent history could not move aside the rollout {}: {move_error}",
                    path.display()
                );
                return format!("{error}; the rollout stays at {}", path.display());
            }
        };
        log::warn!(
            "Agent history quarantined a rollout it cannot read path={} quarantined={rollout} error={error}",
            path.display()
        );
        let mut reason = format!("{error}; the rollout was moved to {rollout}");
        if let Some(session_id) = session_id {
            let attachments = self
                .inner
                .paths
                .attachments
                .join(session_id.value().to_string());
            match quarantine_file(&attachments) {
                Ok(Some(target)) => {
                    log::warn!(
                        "Agent history quarantined the attachments of session {session_id} path={} quarantined={}",
                        attachments.display(),
                        target.display()
                    );
                    reason.push_str(&format!(", its attachments to {}", target.display()));
                }
                Ok(None) => {}
                Err(move_error) => log::error!(
                    "Agent history could not move aside the attachments of session {session_id}: {move_error}"
                ),
            }
        }
        reason
    }

    /// Keeps `session_id` from being allocated again, so a session the index
    /// could not take never shares its id, rollout name or attachments with a
    /// new one.
    fn reserve_session_id(&self, session_id: AgentSessionId) -> Result<(), AgentHistoryStoreError> {
        let following = session_id.value().checked_add(1).ok_or_else(|| {
            AgentHistoryStoreError::CorruptHistory("session id exhausted".to_string())
        })?;
        let _database = self.inner.database_gate.lock();
        self.connection()?
            .execute(
                "UPDATE agent_store_state SET value = MAX(value, ?) WHERE key = 'next_session_id'",
                params![to_i64(following, "next session id")?],
            )
            .map_err(database_error)?;
        Ok(())
    }

    pub(in crate::store) fn mark_projection_pending(
        &self,
        session_id: AgentSessionId,
        path: &Path,
        revision: AgentRevision,
    ) -> Result<(), AgentHistoryStoreError> {
        let _database = self.inner.database_gate.lock();
        let connection = self.connection()?;
        connection
            .execute(
                r#"
INSERT INTO agent_projection_repairs(session_id, rollout_path, requested_revision)
VALUES (?, ?, ?)
ON CONFLICT(session_id) DO UPDATE SET
    rollout_path = excluded.rollout_path,
    requested_revision = MAX(requested_revision, excluded.requested_revision)
                "#,
                params![
                    to_i64(session_id.value(), "session id")?,
                    path.to_string_lossy(),
                    to_i64(revision.value(), "session revision")?,
                ],
            )
            .map_err(database_error)?;
        Ok(())
    }

    pub(in crate::store) fn clear_projection_pending(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError> {
        let _database = self.inner.database_gate.lock();
        let connection = self.connection()?;
        connection
            .execute(
                "DELETE FROM agent_projection_repairs WHERE session_id = ?",
                params![to_i64(session_id.value(), "session id")?],
            )
            .map_err(database_error)?;
        Ok(())
    }

    pub(in crate::store) fn catch_up_pending_repairs(&self) -> Result<(), AgentHistoryStoreError> {
        let repairs = {
            let _database = self.inner.database_gate.lock();
            let connection = self.connection()?;
            let mut statement = connection
                .prepare(
                    "SELECT session_id, rollout_path FROM agent_projection_repairs ORDER BY session_id",
                )
                .map_err(database_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(database_error)?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(database_error)?
        };
        for (raw_session_id, raw_path) in repairs {
            let session_id = AgentSessionId::new(from_i64(raw_session_id, "session id")?);
            let path = PathBuf::from(raw_path);
            let result = (|| {
                ensure_scoped_path(&path, &self.inner.paths.root)?;
                if !path.exists() {
                    return self.clear_projection_pending(session_id);
                }
                repair_partial_tail(&path)?;
                if fs::metadata(&path).map_err(io_error)?.len() == 0 {
                    self.clear_projection_pending(session_id)?;
                    return Ok(());
                }
                self.materialize_path(session_id, &path)?;
                self.clear_projection_pending(session_id)
            })();
            if let Err(error) = result {
                log::error!(
                    "Agent history startup repair remains pending for session {session_id}: {error}"
                );
                self.inner
                    .projection_lag
                    .lock()
                    .insert(session_id, ProjectionLag::Refused(error.to_string()));
            }
        }
        Ok(())
    }

    pub(in crate::store) fn repair_session_projection_inner(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError> {
        let path = match self.lookup_rollout_path(session_id)? {
            Some(path) => Some(path),
            None => {
                let _database = self.inner.database_gate.lock();
                let connection = self.connection()?;
                connection
                    .query_row(
                        "SELECT rollout_path FROM agent_projection_repairs WHERE session_id = ?",
                        params![to_i64(session_id.value(), "session id")?],
                        |row| row.get::<_, String>(0).map(PathBuf::from),
                    )
                    .optional()
                    .map_err(database_error)?
            }
        }
        .ok_or(AgentHistoryStoreError::NotFound { session_id })?;
        self.materialize_path(session_id, &path)?;
        self.validate_session_attachments(session_id)?;
        self.clear_projection_pending(session_id)?;
        self.inner.projection_lag.lock().remove(&session_id);
        Ok(())
    }
}
