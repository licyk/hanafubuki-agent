use crate::store::lifecycle::deletion::delete_obligation_session_ids_for_retention;
use crate::store::projection::runtime_recovery_needed;
use crate::store::*;

impl FileAgentHistoryStore {
    pub fn retention_report(
        &self,
        policy: &AgentHistoryRetentionPolicy,
    ) -> Result<AgentHistoryRetentionReport, AgentHistoryStoreError> {
        let rows = {
            let _database = self.inner.database_gate.lock();
            let connection = self.connection()?;
            let mut statement = connection
                .prepare(
                    r#"
SELECT session_id, rollout_path, lifecycle, updated_at, recovery_needed, runtime_json,
       EXISTS(
           SELECT 1 FROM agent_session_lineage
           WHERE parent_session_id = agent_sessions.session_id
       )
FROM agent_sessions
ORDER BY CASE lifecycle WHEN 'archived' THEN 0 ELSE 1 END, updated_at ASC, session_id ASC
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
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                })
                .map_err(database_error)?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(database_error)?
        };
        let deleting = delete_obligation_session_ids_for_retention(self)?;
        let mut total_physical_bytes = self
            .inner
            .paths
            .sqlite_database
            .metadata()
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let mut archived_session_count = 0usize;
        let mut eligible = Vec::new();
        for (
            raw_id,
            raw_path,
            raw_lifecycle,
            raw_updated_at,
            recovery_needed,
            runtime_json,
            referenced,
        ) in &rows
        {
            let session_id = AgentSessionId::new(from_i64(*raw_id, "session id")?);
            let lifecycle = parse_lifecycle(raw_lifecycle)?;
            if lifecycle == AgentSessionLifecycle::Archived {
                archived_session_count = archived_session_count.saturating_add(1);
            }
            let path = PathBuf::from(raw_path);
            ensure_scoped_path(&path, &self.inner.paths.root)?;
            let physical_bytes = path
                .metadata()
                .map(|metadata| metadata.len())
                .unwrap_or(0)
                .saturating_add(attachment_physical_bytes(self, session_id)?);
            total_physical_bytes = total_physical_bytes.saturating_add(physical_bytes);
            let runtime: hanafubuki_agent_kernel::PersistedAgentRuntimeState =
                serde_json::from_str(runtime_json).map_err(json_error)?;
            if *recovery_needed == 0
                && *referenced == 0
                && !deleting.contains(&session_id)
                && !runtime_recovery_needed(&runtime)
            {
                eligible.push(AgentHistoryRetentionCandidate {
                    session_id,
                    lifecycle,
                    updated_at: raw_updated_at.parse::<u128>().map_err(|error| {
                        AgentHistoryStoreError::CorruptHistory(format!(
                            "invalid session update timestamp: {error}"
                        ))
                    })?,
                    physical_bytes,
                });
            }
        }
        let mut bytes_to_release = policy
            .maximum_total_bytes
            .map(|maximum| total_physical_bytes.saturating_sub(maximum))
            .unwrap_or(0);
        let mut archived_to_release = policy
            .maximum_archived_sessions
            .map(|maximum| archived_session_count.saturating_sub(maximum))
            .unwrap_or(0);
        let mut candidates = Vec::new();
        for candidate in eligible {
            if bytes_to_release == 0 && archived_to_release == 0 {
                break;
            }
            bytes_to_release = bytes_to_release.saturating_sub(candidate.physical_bytes);
            if candidate.lifecycle == AgentSessionLifecycle::Archived {
                archived_to_release = archived_to_release.saturating_sub(1);
            }
            candidates.push(candidate);
        }
        Ok(AgentHistoryRetentionReport {
            automatic_deletion_enabled: policy.automatic_deletion_enabled,
            total_physical_bytes,
            session_count: rows.len(),
            archived_session_count,
            candidates,
        })
    }
}

fn attachment_physical_bytes(
    store: &FileAgentHistoryStore,
    session_id: AgentSessionId,
) -> Result<u64, AgentHistoryStoreError> {
    let root = store
        .inner
        .paths
        .attachments
        .join(session_id.value().to_string());
    ensure_scoped_path(&root, &store.inner.paths.attachments)?;
    if !root.exists() {
        return Ok(0);
    }
    let mut total = 0u64;
    let mut stack = vec![root];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(directory).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let metadata = entry.metadata().map_err(io_error)?;
            if metadata.is_dir() {
                stack.push(entry.path());
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Ok(total)
}
