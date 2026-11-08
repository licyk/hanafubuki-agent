//! What keeps a session from being deleted: a session that still needs its
//! rollout prefix or its attachments. A fork the index holds is in
//! `agent_session_lineage`; a session file the index does not hold (a
//! rollout moved into `.recovery`, one refused at open and left in place
//! with a pending repair) is read for the sessions it forks from and the
//! digests it names (`kept_files`). A file that cannot be read keeps every
//! session: what it needs is unknown.

use crate::store::kept_files::{quarantined_session_id, read_kept_file};
use crate::store::*;
use hanafubuki_agent_kernel::AgentAttachmentId;

impl FileAgentHistoryStore {
    /// Refuses deleting `session_ids` together while a session outside them
    /// needs one of them.
    pub(in crate::store) fn check_sessions_deletable_inner(
        &self,
        session_ids: &[AgentSessionId],
    ) -> Result<(), AgentHistoryStoreError> {
        let deletion_set = session_ids.iter().copied().collect::<HashSet<_>>();
        let mut ordered = deletion_set.iter().copied().collect::<Vec<_>>();
        ordered.sort_unstable();
        let repairs = {
            let _database = self.inner.database_gate.lock();
            let connection = self.connection()?;
            let mut children = connection
                .prepare(
                    "SELECT child_session_id FROM agent_session_lineage WHERE parent_session_id = ?",
                )
                .map_err(database_error)?;
            for session_id in &ordered {
                let rows = children
                    .query_map(params![to_i64(session_id.value(), "session id")?], |row| {
                        row.get::<_, i64>(0)
                    })
                    .map_err(database_error)?;
                for child in rows {
                    let child = from_i64(child.map_err(database_error)?, "child session id")?;
                    if !deletion_set.contains(&AgentSessionId::new(child)) {
                        return Err(AgentHistoryStoreError::ReferencedByFork {
                            session_id: *session_id,
                        });
                    }
                }
            }
            drop(children);
            let mut statement = connection
                .prepare("SELECT session_id, rollout_path FROM agent_projection_repairs ORDER BY session_id")
                .map_err(database_error)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(database_error)?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(database_error)?
        };
        let mut kept = self
            .recovery_files()
            .map_err(AgentHistoryStoreError::Io)?
            .into_iter()
            .map(|path| (quarantined_session_id(&path), path))
            .collect::<Vec<_>>();
        for (session_id, path) in repairs {
            let session_id = AgentSessionId::new(from_i64(session_id, "session id")?);
            let path = PathBuf::from(path);
            // The store reads no repair outside itself (`catch_up_pending_repairs`).
            if !deletion_set.contains(&session_id)
                && path.exists()
                && ensure_scoped_path(&path, &self.inner.paths.root).is_ok()
            {
                kept.push((Some(session_id), path));
            }
        }
        if kept.is_empty() {
            return Ok(());
        }
        let blobs = self.blobs_of(&ordered)?;
        let mut dependents = HashMap::<AgentSessionId, Vec<String>>::new();
        for (owner, path) in kept {
            let label = owner.map_or_else(
                || path.display().to_string(),
                |owner| format!("session {owner} at {}", path.display()),
            );
            match read_kept_file(&path) {
                Ok(references) => {
                    for session_id in &ordered {
                        let forks = references.parents.contains(session_id);
                        let names_blob = blobs.get(session_id).is_some_and(|blobs| {
                            references
                                .digests
                                .iter()
                                .any(|digest| blobs.contains(digest))
                        });
                        if forks || names_blob {
                            dependents
                                .entry(*session_id)
                                .or_default()
                                .push(label.clone());
                        }
                    }
                }
                Err(error) => {
                    for session_id in &ordered {
                        dependents
                            .entry(*session_id)
                            .or_default()
                            .push(format!("{label} (could not be read: {error})"));
                    }
                }
            }
        }
        match ordered
            .into_iter()
            .find_map(|session_id| Some((session_id, dependents.remove(&session_id)?)))
        {
            Some((session_id, dependents)) => {
                log::warn!(
                    "Agent session {session_id} is kept: quarantined session files need it: {}",
                    dependents.join("; ")
                );
                Err(AgentHistoryStoreError::ReferencedByQuarantinedSession {
                    session_id,
                    dependents,
                })
            }
            None => Ok(()),
        }
    }

    /// The attachment ids of the blobs each of `session_ids` holds.
    fn blobs_of(
        &self,
        session_ids: &[AgentSessionId],
    ) -> Result<HashMap<AgentSessionId, HashSet<AgentAttachmentId>>, AgentHistoryStoreError> {
        let mut held = HashMap::new();
        for session_id in session_ids {
            let blobs = self
                .inner
                .paths
                .attachments
                .join(session_id.value().to_string())
                .join("blobs");
            if !blobs.exists() {
                continue;
            }
            let mut ids = HashSet::new();
            for entry in fs::read_dir(&blobs).map_err(io_error)? {
                let name = entry.map_err(io_error)?.file_name();
                if let Some(id) = name
                    .to_string_lossy()
                    .strip_suffix(".blob")
                    .and_then(|id| AgentAttachmentId::parse(id.to_string()).ok())
                {
                    ids.insert(id);
                }
            }
            held.insert(*session_id, ids);
        }
        Ok(held)
    }
}
