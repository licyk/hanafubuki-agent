use crate::store::*;
use chrono::Datelike;
use hanafubuki_agent_foundation::paths::PathKey;

impl FileAgentHistoryStore {
    pub(in crate::store) fn reconcile_session_locations(
        &self,
    ) -> Result<(), AgentHistoryStoreError> {
        let sessions = {
            let _database = self.inner.database_gate.lock();
            let connection = self.connection()?;
            let mut statement = connection
                .prepare(
                    r#"
SELECT session_id, rollout_path, lifecycle, runtime_json
FROM agent_sessions
WHERE lifecycle IN ('archived', 'available')
ORDER BY session_id
                    "#,
                )
                .map_err(database_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(database_error)?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(database_error)?
        };
        for (raw_id, raw_path, lifecycle, runtime_json) in sessions {
            let session_id = AgentSessionId::new(from_i64(raw_id, "session id")?);
            if let Err(error) =
                self.reconcile_session_location(session_id, raw_path, &lifecycle, &runtime_json)
            {
                log::error!(
                    "Agent session {session_id} quarantined: its history file is not where its lifecycle puts it, its files are kept: {error}"
                );
                self.inner
                    .projection_lag
                    .lock()
                    .insert(session_id, ProjectionLag::Refused(error.to_string()));
            }
        }
        Ok(())
    }

    /// Moves one session's rollout to the tree its lifecycle names. Two
    /// copies, a missing file or a path outside the store refuse the session
    /// and touch nothing.
    fn reconcile_session_location(
        &self,
        session_id: AgentSessionId,
        raw_path: String,
        lifecycle: &str,
        runtime_json: &str,
    ) -> Result<(), AgentHistoryStoreError> {
        let source = PathBuf::from(raw_path);
        ensure_scoped_path(&source, &self.inner.paths.root)?;
        let source_is_archived =
            PathKey::of(&source).starts_with(&PathKey::of(&self.inner.paths.archived_sessions));
        if (lifecycle == "archived") == source_is_archived {
            return Ok(());
        }
        let runtime: hanafubuki_agent_kernel::PersistedAgentRuntimeState =
            serde_json::from_str(runtime_json).map_err(json_error)?;
        let target = desired_session_path(self, &source, &runtime, lifecycle)?;
        if source == target {
            return Ok(());
        }
        if source.exists() && target.exists() {
            return Err(AgentHistoryStoreError::CorruptHistory(format!(
                "session {session_id} has both active and archived copies"
            )));
        }
        if source.exists() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(io_error)?;
            }
            fs::rename(&source, &target).map_err(io_error)?;
        } else if !target.exists() {
            return Err(AgentHistoryStoreError::CorruptHistory(format!(
                "session {session_id} history file is missing"
            )));
        }
        let _database = self.inner.database_gate.lock();
        let connection = self.connection()?;
        connection
            .execute(
                "UPDATE agent_sessions SET rollout_path = ? WHERE session_id = ?",
                params![
                    target.to_string_lossy(),
                    to_i64(session_id.value(), "session id")?
                ],
            )
            .map_err(database_error)?;
        Ok(())
    }

    pub(in crate::store) fn archive_session_inner(
        &self,
        session_id: AgentSessionId,
        expected_revision: AgentRevision,
    ) -> Result<(), AgentHistoryStoreError> {
        move_session_file(self, session_id, expected_revision, true)
    }

    pub(in crate::store) fn restore_session_inner(
        &self,
        session_id: AgentSessionId,
        expected_revision: AgentRevision,
    ) -> Result<(), AgentHistoryStoreError> {
        move_session_file(self, session_id, expected_revision, false)
    }
}

fn move_session_file(
    store: &FileAgentHistoryStore,
    session_id: AgentSessionId,
    expected_revision: AgentRevision,
    archive: bool,
) -> Result<(), AgentHistoryStoreError> {
    let gate = store.session_gate(session_id);
    let _guard = gate.lock();
    if store.inner.writers.lock().contains_key(&session_id) {
        store.shutdown_session_inner(session_id)?;
    }
    let _database = store.inner.database_gate.lock();
    let connection = store.connection()?;
    let (path, revision): (String, i64) = connection
        .query_row(
            "SELECT rollout_path, revision FROM agent_sessions WHERE session_id = ?",
            params![to_i64(session_id.value(), "session id")?],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(database_error)?
        .ok_or(AgentHistoryStoreError::NotFound { session_id })?;
    if from_i64(revision, "session revision")? != expected_revision.value() {
        return Err(AgentHistoryStoreError::StaleRevision);
    }
    let source = PathBuf::from(path);
    ensure_scoped_path(&source, &store.inner.paths.root)?;
    let runtime: hanafubuki_agent_kernel::PersistedAgentRuntimeState = serde_json::from_str(
        &connection
            .query_row(
                "SELECT runtime_json FROM agent_sessions WHERE session_id = ?",
                params![to_i64(session_id.value(), "session id")?],
                |row| row.get::<_, String>(0),
            )
            .map_err(database_error)?,
    )
    .map_err(json_error)?;
    let target = desired_session_path(
        store,
        &source,
        &runtime,
        if archive { "archived" } else { "available" },
    )?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    if source.exists() && target.exists() && source != target {
        return Err(AgentHistoryStoreError::CorruptHistory(format!(
            "session {session_id} has both lifecycle file locations"
        )));
    }
    if source.exists() && source != target {
        fs::rename(&source, &target).map_err(io_error)?;
    } else if !target.exists() {
        return Err(AgentHistoryStoreError::CorruptHistory(format!(
            "session {session_id} history file is missing"
        )));
    }
    connection
        .execute(
            "UPDATE agent_sessions SET rollout_path = ?, archived_at = ? WHERE session_id = ?",
            params![
                target.to_string_lossy(),
                archive.then(|| now_millis().to_string()),
                to_i64(session_id.value(), "session id")?,
            ],
        )
        .map_err(database_error)?;
    Ok(())
}

fn desired_session_path(
    store: &FileAgentHistoryStore,
    current: &Path,
    runtime: &hanafubuki_agent_kernel::PersistedAgentRuntimeState,
    lifecycle: &str,
) -> Result<PathBuf, AgentHistoryStoreError> {
    let file_name = current.file_name().ok_or_else(|| {
        AgentHistoryStoreError::CorruptHistory("session path has no file name".to_string())
    })?;
    match lifecycle {
        "archived" => Ok(store.inner.paths.archived_sessions.join(file_name)),
        "available" => {
            let timestamp = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(
                i64::try_from(runtime.created_at).unwrap_or(i64::MAX),
            )
            .ok_or_else(|| {
                AgentHistoryStoreError::CorruptHistory(
                    "session creation timestamp is invalid".to_string(),
                )
            })?;
            Ok(store
                .inner
                .paths
                .active_sessions
                .join(format!("{:04}", timestamp.year()))
                .join(format!("{:02}", timestamp.month()))
                .join(format!("{:02}", timestamp.day()))
                .join(file_name))
        }
        other => Err(AgentHistoryStoreError::CorruptHistory(format!(
            "unknown session lifecycle {other}"
        ))),
    }
}
