//! The sweep of attachment blobs nothing references. It only collects
//! garbage: it deletes a blob only when every session file still on disk is
//! known not to reference it, and a session it cannot sweep is logged and
//! kept, never a reason for the store not to open.
//!
//! A blob lives under the session that staged it, and any session can
//! reference it: a fork child's items keep the source session of a restored
//! prompt's images. So the sweep keeps what any projected item references,
//! and what any rollout moved into `.recovery` names, since the index no
//! longer holds those.

use crate::store::kept_files::read_kept_file;
use crate::store::*;
use hanafubuki_agent_kernel::AgentAttachmentId;

/// What the sweep keeps.
struct KeptAttachments {
    /// What projected items reference, by the session that owns the blob.
    indexed: HashSet<(AgentSessionId, AgentAttachmentId)>,
    /// Every digest a quarantined rollout names, whichever session owns it.
    quarantined: Arc<HashSet<AgentAttachmentId>>,
}

impl KeptAttachments {
    fn contains(&self, session_id: AgentSessionId, attachment_id: &AgentAttachmentId) -> bool {
        self.quarantined.contains(attachment_id)
            || self.indexed.contains(&(session_id, attachment_id.clone()))
    }
}

impl FileAgentHistoryStore {
    /// Reads the digests the quarantined rollouts name, for the sweeps until
    /// the store closes. Rollouts are moved aside only by `open`, before
    /// this runs; one the user restores meanwhile only keeps its blobs a
    /// little longer.
    pub(in crate::store) fn record_quarantined_references(&self) {
        let mut found = HashSet::new();
        let scanned = self.recovery_files().and_then(|files| {
            files.iter().try_for_each(|path| {
                read_kept_file(path)
                    .map(|references| found.extend(references.digests))
                    .map_err(|error| format!("{}: {error}", path.display()))
            })
        });
        *self.inner.quarantined_references.lock() = match scanned {
            Ok(()) => Some(Arc::new(found)),
            Err(error) => {
                log::warn!(
                    "Agent history keeps every attachment: a quarantined rollout could not be read: {error}"
                );
                None
            }
        };
    }

    /// The start-up sweep, over every session's blobs.
    pub(in crate::store) fn cleanup_all_unreferenced_attachments(
        &self,
    ) -> Result<(), AgentHistoryStoreError> {
        let session_ids = fs::read_dir(&self.inner.paths.attachments)
            .map_err(io_error)?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().to_string_lossy().parse::<u64>().ok())
            .map(AgentSessionId::new)
            .collect::<Vec<_>>();
        let kept = match self.kept_attachments() {
            Ok(Some(kept)) => kept,
            Ok(None) => return Ok(()),
            Err(error) => {
                log::warn!("Agent history kept every attachment: {error}");
                return Ok(());
            }
        };
        for session_id in session_ids {
            if let Err(error) = self.sweep_session_attachments(session_id, &kept) {
                log::warn!("Agent history kept the attachments of session {session_id}: {error}");
            }
        }
        Ok(())
    }

    /// Deletes the blobs of `session_id` that nothing references.
    pub(in crate::store) fn cleanup_unreferenced_attachments_for(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError> {
        match self.kept_attachments()? {
            Some(kept) => self.sweep_session_attachments(session_id, &kept),
            None => Ok(()),
        }
    }

    /// What every session file references, or `None` while that is unknown:
    /// a session's projection is unavailable (its items, which may reference
    /// any session's blobs, are not all in the index), or a quarantined
    /// rollout could not be read. An item that does not decode fails it.
    fn kept_attachments(&self) -> Result<Option<KeptAttachments>, AgentHistoryStoreError> {
        if !self.inner.projection_lag.lock().is_empty() {
            log::debug!(
                "Agent history keeps every attachment while a session's projection is unavailable"
            );
            return Ok(None);
        }
        let Some(quarantined) = self.inner.quarantined_references.lock().clone() else {
            return Ok(None);
        };
        let _database = self.inner.database_gate.lock();
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT item_json FROM agent_history_items ORDER BY session_id, first_item_order",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(database_error)?;
        let mut indexed = HashSet::new();
        for row in rows {
            let item: AgentHistoryItem =
                serde_json::from_str(&row.map_err(database_error)?).map_err(json_error)?;
            let images = match item.content {
                ConversationItem::UserMessage { images, .. }
                | ConversationItem::SteeringMessage { images, .. }
                | ConversationItem::ToolOutput { images, .. } => images,
                _ => continue,
            };
            indexed.extend(
                images
                    .into_iter()
                    .map(|image| (image.source_session_id, image.attachment_id)),
            );
        }
        Ok(Some(KeptAttachments {
            indexed,
            quarantined,
        }))
    }

    /// Deletes the blobs of `session_id` that `kept` does not hold, and
    /// nothing of a session the index does not hold.
    fn sweep_session_attachments(
        &self,
        session_id: AgentSessionId,
        kept: &KeptAttachments,
    ) -> Result<(), AgentHistoryStoreError> {
        {
            let _database = self.inner.database_gate.lock();
            let indexed: i64 = self
                .connection()?
                .query_row(
                    "SELECT COUNT(*) FROM agent_sessions WHERE session_id = ?",
                    params![to_i64(session_id.value(), "session id")?],
                    |row| row.get(0),
                )
                .map_err(database_error)?;
            if indexed == 0 {
                return Ok(());
            }
        }
        let blobs = self
            .inner
            .paths
            .attachments
            .join(session_id.value().to_string())
            .join("blobs");
        if !blobs.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(&blobs).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let name = entry.file_name().to_string_lossy().to_string();
            let id = name
                .strip_suffix(".blob")
                .and_then(|id| AgentAttachmentId::parse(id.to_string()).ok());
            let Some(id) = id else {
                log::warn!(
                    "Agent history keeps an unexpected file among the attachments of session {session_id}: {name}"
                );
                continue;
            };
            if !kept.contains(session_id, &id) {
                fs::remove_file(entry.path()).map_err(io_error)?;
            }
        }
        sync_directory(&blobs)
    }
}
