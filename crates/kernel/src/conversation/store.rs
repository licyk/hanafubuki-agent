//! The storage interface, independent of the durable record types.

use super::*;

pub trait AgentHistoryStore: Send + Sync {
    fn allocate_session_id(&self) -> Result<AgentSessionId, AgentHistoryStoreError>;
    fn create_session(&self, commit: AgentConversationCommit)
        -> Result<(), AgentHistoryStoreError>;
    /// Appends `commit`, returning once it is durable or failed.
    fn append_commit(&self, commit: AgentConversationCommit) -> Result<(), AgentHistoryStoreError>;
    /// Appends `delta`, returning once it is durable or failed.
    fn append_delta(&self, delta: AgentConversationDelta) -> Result<(), AgentHistoryStoreError>;
    /// Queues `commit` behind the session's earlier appends and returns
    /// without waiting for it; `ack` reports the outcome, and a follow-up it
    /// hands back is appended next. The caller, which may be an async worker
    /// or a drop guard, only finds or starts the session's writer and
    /// queues; it never waits for the append itself.
    fn enqueue_commit(&self, commit: AgentConversationCommit, ack: AgentHistoryAppendAck);
    /// `enqueue_commit` for a delta.
    fn enqueue_delta(&self, delta: AgentConversationDelta, ack: AgentHistoryAppendAck);
    fn create_fork(
        &self,
        source_session_id: AgentSessionId,
        point: AgentHistoryForkPoint,
        commit: AgentConversationCommit,
    ) -> Result<AgentHistoryBase, AgentHistoryStoreError>;
    fn create_fork_at_head(
        &self,
        source_session_id: AgentSessionId,
        source_revision: AgentRevision,
        commit: AgentConversationCommit,
    ) -> Result<AgentHistoryBase, AgentHistoryStoreError> {
        let _ = (source_session_id, source_revision, commit);
        Err(AgentHistoryStoreError::Io(
            "committed-head forks are not supported by this history store".to_string(),
        ))
    }
    fn stage_image_attachment(
        &self,
        session_id: AgentSessionId,
        image: crate::PreparedAgentImage,
    ) -> Result<crate::AgentImageAttachmentRef, AgentHistoryStoreError> {
        let _ = (session_id, image);
        Err(AgentHistoryStoreError::Io(
            "image attachments are not supported by this history store".to_string(),
        ))
    }
    /// Stores a tool's already validated image payload without resizing or re-encoding it.
    fn stage_tool_image_attachment(
        &self,
        session_id: AgentSessionId,
        bytes: Vec<u8>,
        mime_type: String,
    ) -> Result<crate::AgentImageAttachmentRef, AgentHistoryStoreError> {
        self.stage_image_attachment(
            session_id,
            crate::PreparedAgentImage {
                bytes,
                mime_type,
                width: 0,
                height: 0,
                display_name: "tool image".into(),
                detail: crate::AgentImageDetail::Auto,
            },
        )
    }

    fn load_image_attachment(
        &self,
        attachment: &crate::AgentImageAttachmentRef,
    ) -> Result<Vec<u8>, AgentHistoryStoreError> {
        let _ = attachment;
        Err(AgentHistoryStoreError::Io(
            "image attachments are not supported by this history store".to_string(),
        ))
    }
    /// Loads `attachments` for a model request of `session_id` and reports
    /// their bytes through `ack`. The caller may be an async worker: a store
    /// that reads files does so on its own worker and returns at once. This
    /// default, for stores held in memory, loads them inline.
    fn enqueue_image_attachment_load(
        &self,
        session_id: AgentSessionId,
        attachments: Vec<crate::AgentImageAttachmentRef>,
        ack: AgentImageAttachmentLoadAck,
    ) {
        let _ = session_id;
        ack.acknowledge(
            attachments
                .iter()
                .map(|attachment| self.load_image_attachment(attachment))
                .collect(),
        );
    }
    fn cleanup_unreferenced_image_attachments(
        &self,
        _session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError> {
        Ok(())
    }
    /// Writes `contents` as the session's export `file_name` (a plain file
    /// name), replacing an earlier one of that name, and returns its path as
    /// a person or a model reads it. Exports live with the session's data
    /// and are removed when it is deleted. This default, for stores that
    /// keep no files, refuses.
    fn write_session_export(
        &self,
        session_id: AgentSessionId,
        file_name: &str,
        contents: &str,
    ) -> Result<String, AgentHistoryStoreError> {
        let _ = (session_id, file_name, contents);
        Err(AgentHistoryStoreError::Io(
            "this session's history store keeps no files".to_string(),
        ))
    }
    /// Replaces the session's content store (`hierarchical`'s originals,
    /// kept apart from the records) with `store`, atomically. It lives with
    /// the session's data and is removed when the session is deleted. This
    /// default, for stores that keep no files, refuses: the store then lives
    /// in memory only.
    fn write_compression_store(
        &self,
        session_id: AgentSessionId,
        store: &crate::AgentContentStore,
    ) -> Result<(), AgentHistoryStoreError> {
        let _ = (session_id, store);
        Err(AgentHistoryStoreError::Io(
            "this session's history store keeps no files".to_string(),
        ))
    }
    /// The session's content store, none when it has none. One that cannot
    /// be read is set aside (never deleted) and reads as none.
    fn load_compression_store(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Option<crate::AgentContentStore>, AgentHistoryStoreError> {
        let _ = session_id;
        Ok(None)
    }
    fn load_visible_image_attachment(
        &self,
        session_id: AgentSessionId,
        attachment: &crate::AgentImageAttachmentRef,
    ) -> Result<Vec<u8>, AgentHistoryStoreError> {
        if session_id != attachment.source_session_id {
            return Err(AgentHistoryStoreError::NotFound { session_id });
        }
        self.load_image_attachment(attachment)
    }
    fn flush_session(&self, session_id: AgentSessionId) -> Result<(), AgentHistoryStoreError>;
    fn shutdown_session(&self, session_id: AgentSessionId) -> Result<(), AgentHistoryStoreError>;
    fn discard_live_session(&self, session_id: AgentSessionId);
    /// Every session that needs recovery, oldest id first, each loaded on
    /// its own: one that fails to load is its own entry's error, and only a
    /// failure to read the index fails the whole call.
    fn load_recovery_candidates(
        &self,
    ) -> Result<Vec<Result<AgentRecoveredSession, AgentRecoveryLoadFailure>>, AgentHistoryStoreError>;
    fn load_runtime_state(
        &self,
        session_id: AgentSessionId,
    ) -> Result<PersistedAgentRuntimeState, AgentHistoryStoreError>;
    fn load_latest_model_context(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Vec<AgentHistoryItem>, AgentHistoryStoreError>;
    fn list_sessions_page(
        &self,
        request: AgentSessionListRequest,
    ) -> Result<AgentSessionPage, AgentHistoryStoreError>;
    /// What the person sent in the top-level sessions, newest first, each
    /// text once, at most `limit` (and at most
    /// [`RECENT_USER_MESSAGES_MAX`]): their messages, the steers among
    /// them, and the `!` commands they ran, as typed (`!` then the
    /// command). With `before` (Unix milliseconds), only what was recorded
    /// before it. An item that does not decode is left out. This default,
    /// for stores that keep no index, has none.
    fn recent_user_messages(
        &self,
        limit: usize,
        before: Option<u128>,
    ) -> Result<Vec<String>, AgentHistoryStoreError> {
        let _ = (limit, before);
        Ok(Vec::new())
    }
    fn list_items_page(
        &self,
        request: AgentTranscriptPageRequest,
    ) -> Result<AgentTranscriptPage, AgentHistoryStoreError>;
    fn list_turn_items_page(
        &self,
        request: AgentTurnPageRequest,
    ) -> Result<AgentTranscriptPage, AgentHistoryStoreError>;
    fn archive_session(
        &self,
        session_id: AgentSessionId,
        expected_revision: AgentRevision,
    ) -> Result<(), AgentHistoryStoreError>;
    fn restore_session(
        &self,
        session_id: AgentSessionId,
        expected_revision: AgentRevision,
    ) -> Result<(), AgentHistoryStoreError>;
    /// Whether `session_ids` can be deleted together: no session outside
    /// them needs their history or attachments. The deletion's prepare step
    /// asks before anything else of the session is let go;
    /// `delete_sessions_confirmed` checks again.
    fn check_sessions_deletable(
        &self,
        session_ids: &[AgentSessionId],
    ) -> Result<(), AgentHistoryStoreError>;
    fn delete_sessions_confirmed(
        &self,
        session_ids: &[AgentSessionId],
    ) -> Result<(), AgentHistoryStoreError>;
    fn repair_session_projection(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError>;
    /// Stops every writer after the commands queued before it, waiting
    /// until `deadline`. A writer that has not stopped by then is reported;
    /// what it had not acknowledged was never durable, so recovery restores
    /// the last acknowledged revision.
    fn shutdown(&self, deadline: std::time::Instant) -> Vec<AgentHistoryStoreError>;
}
