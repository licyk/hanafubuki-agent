use crate::store::*;
use hanafubuki_agent_foundation::persistence::quarantine_file;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::store) struct DeleteObligation {
    pub(in crate::store) version: u32,
    pub(in crate::store) operation_id: String,
    pub(in crate::store) session_ids: Vec<AgentSessionId>,
    pub(in crate::store) source_paths: Vec<PathBuf>,
    #[serde(default)]
    pub(in crate::store) attachment_paths: Vec<PathBuf>,
}

impl FileAgentHistoryStore {
    pub(in crate::store) fn recover_delete_obligations(
        &self,
    ) -> Result<(), AgentHistoryStoreError> {
        let mut obligations = Vec::new();
        for entry in fs::read_dir(&self.inner.paths.trash).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            if !entry.file_type().map_err(io_error)?.is_dir() {
                continue;
            }
            let manifest = entry.path().join("obligation.json");
            if manifest.exists() {
                match read_delete_obligation(&manifest) {
                    Ok(obligation) => obligations.push((manifest, obligation)),
                    Err(error) => {
                        log::warn!(
                            "quarantining incompatible Agent history delete obligation path={} error={error}",
                            manifest.display()
                        );
                        quarantine_file(&manifest).map_err(AgentHistoryStoreError::Io)?;
                    }
                }
            } else if fs::read_dir(entry.path())
                .map_err(io_error)?
                .next()
                .is_none()
            {
                fs::remove_dir(entry.path()).map_err(io_error)?;
            }
        }
        obligations.sort_by(|left, right| left.1.operation_id.cmp(&right.1.operation_id));
        for (manifest, obligation) in obligations {
            if let Err(error) = self.complete_delete_obligation(&obligation) {
                log::warn!(
                    "Agent history delete obligation remains pending path={} error={error}",
                    manifest.display()
                );
            }
        }
        Ok(())
    }

    pub(in crate::store) fn find_delete_obligation(
        &self,
        session_ids: &[AgentSessionId],
    ) -> Result<Option<DeleteObligation>, AgentHistoryStoreError> {
        for entry in fs::read_dir(&self.inner.paths.trash).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let manifest = entry.path().join("obligation.json");
            if !manifest.exists() {
                continue;
            }
            let obligation = read_delete_obligation(&manifest)?;
            if obligation.session_ids == session_ids {
                return Ok(Some(obligation));
            }
        }
        Ok(None)
    }

    pub(in crate::store) fn delete_obligation_session_ids(
        &self,
    ) -> Result<HashSet<AgentSessionId>, AgentHistoryStoreError> {
        let mut session_ids = HashSet::new();
        for entry in fs::read_dir(&self.inner.paths.trash).map_err(io_error)? {
            let manifest = entry.map_err(io_error)?.path().join("obligation.json");
            if manifest.exists() {
                session_ids.extend(read_delete_obligation(&manifest)?.session_ids);
            }
        }
        Ok(session_ids)
    }

    pub(in crate::store) fn complete_delete_obligation(
        &self,
        obligation: &DeleteObligation,
    ) -> Result<(), AgentHistoryStoreError> {
        if obligation.version != 1
            || obligation.session_ids.is_empty()
            || obligation.session_ids.len() != obligation.source_paths.len()
            || obligation.session_ids.len() != obligation.attachment_paths.len()
        {
            return Err(AgentHistoryStoreError::CorruptHistory(
                "invalid Agent history delete obligation".to_string(),
            ));
        }
        let trash_dir = self.inner.paths.trash.join(&obligation.operation_id);
        ensure_scoped_path(&trash_dir, &self.inner.paths.trash)?;
        let mut trash_paths = Vec::with_capacity(obligation.source_paths.len());
        for source in obligation
            .source_paths
            .iter()
            .chain(&obligation.attachment_paths)
        {
            ensure_scoped_path(source, &self.inner.paths.root)?;
            let file_name = source.file_name().ok_or_else(|| {
                AgentHistoryStoreError::CorruptHistory("session path has no file name".to_string())
            })?;
            let target = trash_dir.join(file_name);
            if source.exists() && target.exists() {
                return Err(AgentHistoryStoreError::CorruptHistory(format!(
                    "delete obligation has both source and trash copies for {}",
                    source.display()
                )));
            }
            if source.exists() {
                fs::rename(source, &target).map_err(io_error)?;
            }
            trash_paths.push(target);
        }

        {
            let _database = self.inner.database_gate.lock();
            let mut connection = self.connection()?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(database_error)?;
            delete_projection_rows(&transaction, &obligation.session_ids)?;
            transaction.commit().map_err(database_error)?;
        }

        for path in &trash_paths {
            if path.exists() {
                if path.is_dir() {
                    fs::remove_dir_all(path).map_err(io_error)?;
                } else {
                    fs::remove_file(path).map_err(io_error)?;
                }
            }
        }
        {
            let _database = self.inner.database_gate.lock();
            let connection = self.connection()?;
            for (session_id, source) in obligation.session_ids.iter().zip(&obligation.source_paths)
            {
                let remaining: i64 = connection
                    .query_row(
                        "SELECT COUNT(*) FROM agent_sessions WHERE session_id = ?",
                        params![to_i64(session_id.value(), "session id")?],
                        |row| row.get(0),
                    )
                    .map_err(database_error)?;
                let trash_path = trash_dir.join(source.file_name().ok_or_else(|| {
                    AgentHistoryStoreError::CorruptHistory(
                        "session path has no file name".to_string(),
                    )
                })?);
                if remaining != 0 || source.exists() || trash_path.exists() {
                    return Err(AgentHistoryStoreError::ProjectionUnavailable(format!(
                        "delete operation {} is not yet confirmed",
                        obligation.operation_id
                    )));
                }
            }
            for source in &obligation.attachment_paths {
                let file_name = source.file_name().ok_or_else(|| {
                    AgentHistoryStoreError::CorruptHistory(
                        "attachment path has no file name".to_string(),
                    )
                })?;
                if source.exists() || trash_dir.join(file_name).exists() {
                    return Err(AgentHistoryStoreError::ProjectionUnavailable(format!(
                        "delete operation {} still retains attachments",
                        obligation.operation_id
                    )));
                }
            }
        }
        let manifest = trash_dir.join("obligation.json");
        if manifest.exists() {
            fs::remove_file(&manifest).map_err(io_error)?;
        }
        if trash_dir.exists() {
            fs::remove_dir(&trash_dir).map_err(io_error)?;
        }
        Ok(())
    }

    pub(in crate::store) fn delete_sessions_confirmed_inner(
        &self,
        session_ids: &[AgentSessionId],
    ) -> Result<(), AgentHistoryStoreError> {
        if session_ids.is_empty() {
            return Ok(());
        }
        let deletion_set = session_ids.iter().copied().collect::<HashSet<_>>();
        let mut ordered = deletion_set.iter().copied().collect::<Vec<_>>();
        ordered.sort_unstable();
        let gates = ordered
            .iter()
            .map(|session_id| self.session_gate(*session_id))
            .collect::<Vec<_>>();
        let _guards = gates.iter().map(|gate| gate.lock()).collect::<Vec<_>>();
        for session_id in &ordered {
            if self.inner.writers.lock().contains_key(session_id) {
                self.shutdown_session(*session_id)?;
            }
        }
        if let Some(obligation) = self.find_delete_obligation(&ordered)? {
            return self.complete_delete_obligation(&obligation);
        }

        // An obligation already written completes above; a new one is
        // written only once nothing outside the deletion needs it.
        self.check_sessions_deletable_inner(&ordered)?;
        let source_paths = {
            let _database = self.inner.database_gate.lock();
            let connection = self.connection()?;
            ordered
                .iter()
                .map(|session_id| {
                    connection
                        .query_row(
                            "SELECT rollout_path FROM agent_sessions WHERE session_id = ?",
                            params![to_i64(session_id.value(), "session id")?],
                            |row| row.get::<_, String>(0).map(PathBuf::from),
                        )
                        .optional()
                        .map_err(database_error)?
                        .ok_or(AgentHistoryStoreError::NotFound {
                            session_id: *session_id,
                        })
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        for path in &source_paths {
            ensure_scoped_path(path, &self.inner.paths.root)?;
        }
        let operation_id = unique_delete_operation_id(&self.inner.paths.trash)?;
        let obligation = DeleteObligation {
            version: 1,
            operation_id,
            session_ids: ordered.clone(),
            source_paths,
            attachment_paths: ordered
                .iter()
                .map(|session_id| {
                    self.inner
                        .paths
                        .attachments
                        .join(session_id.value().to_string())
                })
                .collect(),
        };
        write_delete_obligation(&self.inner.paths.trash, &obligation)?;
        self.complete_delete_obligation(&obligation)
    }
}

fn unique_delete_operation_id(trash_root: &Path) -> Result<String, AgentHistoryStoreError> {
    for suffix in 0..1_000_u16 {
        let operation_id = format!("delete-{}-{suffix}", now_millis());
        if !trash_root.join(&operation_id).exists() {
            return Ok(operation_id);
        }
    }
    Err(AgentHistoryStoreError::Io(
        "could not allocate a unique Agent history delete operation".to_string(),
    ))
}

pub(in crate::store) fn write_delete_obligation(
    trash_root: &Path,
    obligation: &DeleteObligation,
) -> Result<(), AgentHistoryStoreError> {
    let directory = trash_root.join(&obligation.operation_id);
    ensure_scoped_path(&directory, trash_root)?;
    fs::create_dir(&directory).map_err(io_error)?;
    let temporary = directory.join("obligation.tmp");
    let manifest = directory.join("obligation.json");
    let encoded = serde_json::to_vec(obligation).map_err(json_error)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(io_error)?;
    file.write_all(&encoded).map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    publish_file(&temporary, &manifest).map_err(io_error)?;
    sync_directory(&directory)?;
    sync_directory(trash_root)
}

fn read_delete_obligation(path: &Path) -> Result<DeleteObligation, AgentHistoryStoreError> {
    let encoded = fs::read(path).map_err(io_error)?;
    let mut value: serde_json::Value = serde_json::from_slice(&encoded).map_err(json_error)?;
    let object = value.as_object_mut().ok_or_else(|| {
        AgentHistoryStoreError::CorruptHistory(
            "Agent history delete obligation root is not an object".to_string(),
        )
    })?;
    let original_len = object.len();
    object.retain(|key, _| {
        matches!(
            key.as_str(),
            "version" | "operationId" | "sessionIds" | "sourcePaths" | "attachmentPaths"
        )
    });
    let changed = object.len() != original_len;
    let obligation: DeleteObligation = serde_json::from_value(value).map_err(json_error)?;
    if changed {
        let replacement = serde_json::to_vec(&obligation).map_err(json_error)?;
        let temporary = path.with_extension("json.repaired");
        fs::write(&temporary, replacement).map_err(io_error)?;
        publish_file(&temporary, path).map_err(io_error)?;
    }
    Ok(obligation)
}

fn delete_projection_rows(
    transaction: &rusqlite::Transaction<'_>,
    session_ids: &[AgentSessionId],
) -> Result<(), AgentHistoryStoreError> {
    for session_id in session_ids {
        let id = to_i64(session_id.value(), "session id")?;
        for table in [
            "agent_history_items",
            "agent_turns",
            "agent_history_records",
            "agent_history_projection_state",
            "agent_projection_repairs",
        ] {
            transaction
                .execute(
                    &format!("DELETE FROM {table} WHERE session_id = ?"),
                    params![id],
                )
                .map_err(database_error)?;
        }
        transaction
            .execute(
                "DELETE FROM agent_session_lineage WHERE child_session_id = ?",
                params![id],
            )
            .map_err(database_error)?;
        transaction
            .execute(
                "DELETE FROM agent_sessions WHERE session_id = ?",
                params![id],
            )
            .map_err(database_error)?;
    }
    Ok(())
}

pub(in crate::store) fn delete_obligation_session_ids_for_retention(
    store: &FileAgentHistoryStore,
) -> Result<HashSet<AgentSessionId>, AgentHistoryStoreError> {
    store.delete_obligation_session_ids()
}
