//! Where a session's commits and reads go: the durable history store the
//! kernel was built with, or, for an ephemeral session, a store held in
//! memory. Every session id comes from the durable store, so the two never
//! share one; the session list and recovery read the durable store only, so
//! an ephemeral session is never listed nor recovered, and nothing of it
//! outlives the process.

use super::*;
use crate::{AgentImageAttachmentRef, AgentRecoveredSession, AgentRecoveryLoadFailure};

pub(in crate::service) struct SessionStores {
    durable: Arc<dyn AgentHistoryStore>,
    memory: EphemeralAgentHistoryStore,
    /// Each ephemeral session, with the nearest durable session its history
    /// came from: the images it inherited are read through that one.
    ephemeral: Mutex<HashMap<AgentSessionId, Option<AgentSessionId>>>,
}

impl SessionStores {
    pub(in crate::service) fn new(durable: Arc<dyn AgentHistoryStore>) -> Self {
        Self {
            durable,
            memory: EphemeralAgentHistoryStore::default(),
            ephemeral: Mutex::new(HashMap::new()),
        }
    }

    pub(in crate::service) fn is_ephemeral(&self, session_id: AgentSessionId) -> bool {
        self.ephemeral.lock().contains_key(&session_id)
    }

    /// Routes `session_id` to memory from now on. Called before its first
    /// commit; `parent` is the session its history came from, if any.
    pub(in crate::service) fn admit_ephemeral(
        &self,
        session_id: AgentSessionId,
        parent: Option<AgentSessionId>,
    ) {
        let mut ephemeral = self.ephemeral.lock();
        let anchor = parent.and_then(|parent| match ephemeral.get(&parent) {
            Some(anchor) => *anchor,
            None => Some(parent),
        });
        ephemeral.insert(session_id, anchor);
    }

    /// Drops everything held of the ephemeral `session_id`.
    pub(in crate::service) fn forget_ephemeral(&self, session_id: AgentSessionId) {
        if self.ephemeral.lock().remove(&session_id).is_some() {
            let _ = self.memory.delete_sessions_confirmed(&[session_id]);
        }
    }

    fn store(&self, session_id: AgentSessionId) -> &dyn AgentHistoryStore {
        if self.is_ephemeral(session_id) {
            &self.memory
        } else {
            self.durable.as_ref()
        }
    }

    /// The durable session an ephemeral one reads its inherited images
    /// through; `None` for one whose history came from no durable session.
    fn durable_anchor(&self, session_id: AgentSessionId) -> Option<AgentSessionId> {
        self.ephemeral.lock().get(&session_id).copied().flatten()
    }

    fn partition(
        &self,
        session_ids: &[AgentSessionId],
    ) -> (Vec<AgentSessionId>, Vec<AgentSessionId>) {
        session_ids
            .iter()
            .copied()
            .partition(|session_id| !self.is_ephemeral(*session_id))
    }
}

impl AgentHistoryStore for SessionStores {
    fn allocate_session_id(&self) -> Result<AgentSessionId, AgentHistoryStoreError> {
        self.durable.allocate_session_id()
    }

    fn create_session(
        &self,
        commit: AgentConversationCommit,
    ) -> Result<(), AgentHistoryStoreError> {
        self.store(commit.runtime.id).create_session(commit)
    }

    fn append_commit(&self, commit: AgentConversationCommit) -> Result<(), AgentHistoryStoreError> {
        self.store(commit.runtime.id).append_commit(commit)
    }

    fn append_delta(&self, delta: AgentConversationDelta) -> Result<(), AgentHistoryStoreError> {
        self.store(delta.session_id).append_delta(delta)
    }

    fn enqueue_commit(&self, commit: AgentConversationCommit, ack: crate::AgentHistoryAppendAck) {
        self.store(commit.runtime.id).enqueue_commit(commit, ack);
    }

    fn enqueue_delta(&self, delta: AgentConversationDelta, ack: crate::AgentHistoryAppendAck) {
        self.store(delta.session_id).enqueue_delta(delta, ack);
    }

    fn create_fork(
        &self,
        source_session_id: AgentSessionId,
        before_turn_id: AgentTurnId,
        commit: AgentConversationCommit,
    ) -> Result<AgentHistoryBase, AgentHistoryStoreError> {
        self.store(commit.runtime.id)
            .create_fork(source_session_id, before_turn_id, commit)
    }

    fn create_fork_at_head(
        &self,
        source_session_id: AgentSessionId,
        source_revision: AgentRevision,
        commit: AgentConversationCommit,
    ) -> Result<AgentHistoryBase, AgentHistoryStoreError> {
        self.store(commit.runtime.id).create_fork_at_head(
            source_session_id,
            source_revision,
            commit,
        )
    }

    fn stage_image_attachment(
        &self,
        session_id: AgentSessionId,
        image: crate::PreparedAgentImage,
    ) -> Result<AgentImageAttachmentRef, AgentHistoryStoreError> {
        self.store(session_id)
            .stage_image_attachment(session_id, image)
    }

    fn load_image_attachment(
        &self,
        attachment: &AgentImageAttachmentRef,
    ) -> Result<Vec<u8>, AgentHistoryStoreError> {
        self.store(attachment.source_session_id)
            .load_image_attachment(attachment)
    }

    /// An ephemeral session's own images are in memory and its inherited
    /// ones in the durable store: a load of inherited images only goes
    /// through the durable session they came from, and a mixed one is read
    /// here (an ephemeral session is short and its images few).
    fn enqueue_image_attachment_load(
        &self,
        session_id: AgentSessionId,
        attachments: Vec<AgentImageAttachmentRef>,
        ack: crate::AgentImageAttachmentLoadAck,
    ) {
        if !self.is_ephemeral(session_id) {
            return self
                .durable
                .enqueue_image_attachment_load(session_id, attachments, ack);
        }
        let anchor = self.durable_anchor(session_id);
        match anchor {
            Some(anchor)
                if attachments
                    .iter()
                    .all(|attachment| !self.is_ephemeral(attachment.source_session_id)) =>
            {
                self.durable
                    .enqueue_image_attachment_load(anchor, attachments, ack)
            }
            _ => ack.acknowledge(
                attachments
                    .iter()
                    .map(|attachment| self.load_image_attachment(attachment))
                    .collect(),
            ),
        }
    }

    fn cleanup_unreferenced_image_attachments(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError> {
        self.store(session_id)
            .cleanup_unreferenced_image_attachments(session_id)
    }

    fn load_visible_image_attachment(
        &self,
        session_id: AgentSessionId,
        attachment: &AgentImageAttachmentRef,
    ) -> Result<Vec<u8>, AgentHistoryStoreError> {
        if !self.is_ephemeral(session_id) || self.is_ephemeral(attachment.source_session_id) {
            return self
                .store(session_id)
                .load_visible_image_attachment(session_id, attachment);
        }
        let anchor = self
            .durable_anchor(session_id)
            .ok_or(AgentHistoryStoreError::NotFound { session_id })?;
        self.durable
            .load_visible_image_attachment(anchor, attachment)
    }

    fn flush_session(&self, session_id: AgentSessionId) -> Result<(), AgentHistoryStoreError> {
        self.store(session_id).flush_session(session_id)
    }

    fn shutdown_session(&self, session_id: AgentSessionId) -> Result<(), AgentHistoryStoreError> {
        self.store(session_id).shutdown_session(session_id)
    }

    fn discard_live_session(&self, session_id: AgentSessionId) {
        self.store(session_id).discard_live_session(session_id);
    }

    fn load_recovery_candidates(
        &self,
    ) -> Result<Vec<Result<AgentRecoveredSession, AgentRecoveryLoadFailure>>, AgentHistoryStoreError>
    {
        self.durable.load_recovery_candidates()
    }

    fn load_runtime_state(
        &self,
        session_id: AgentSessionId,
    ) -> Result<PersistedAgentRuntimeState, AgentHistoryStoreError> {
        self.store(session_id).load_runtime_state(session_id)
    }

    fn load_latest_model_context(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Vec<AgentHistoryItem>, AgentHistoryStoreError> {
        self.store(session_id).load_latest_model_context(session_id)
    }

    fn list_sessions_page(
        &self,
        request: AgentSessionListRequest,
    ) -> Result<AgentSessionPage, AgentHistoryStoreError> {
        self.durable.list_sessions_page(request)
    }

    fn list_items_page(
        &self,
        request: AgentTranscriptPageRequest,
    ) -> Result<AgentTranscriptPage, AgentHistoryStoreError> {
        self.store(request.session_id).list_items_page(request)
    }

    fn list_turn_items_page(
        &self,
        request: AgentTurnPageRequest,
    ) -> Result<AgentTranscriptPage, AgentHistoryStoreError> {
        self.store(request.session_id).list_turn_items_page(request)
    }

    fn archive_session(
        &self,
        session_id: AgentSessionId,
        expected_revision: AgentRevision,
    ) -> Result<(), AgentHistoryStoreError> {
        self.store(session_id)
            .archive_session(session_id, expected_revision)
    }

    fn restore_session(
        &self,
        session_id: AgentSessionId,
        expected_revision: AgentRevision,
    ) -> Result<(), AgentHistoryStoreError> {
        self.store(session_id)
            .restore_session(session_id, expected_revision)
    }

    fn check_sessions_deletable(
        &self,
        session_ids: &[AgentSessionId],
    ) -> Result<(), AgentHistoryStoreError> {
        let (durable, _) = self.partition(session_ids);
        if durable.is_empty() {
            return Ok(());
        }
        self.durable.check_sessions_deletable(&durable)
    }

    fn delete_sessions_confirmed(
        &self,
        session_ids: &[AgentSessionId],
    ) -> Result<(), AgentHistoryStoreError> {
        let (durable, ephemeral) = self.partition(session_ids);
        if !durable.is_empty() {
            self.durable.delete_sessions_confirmed(&durable)?;
        }
        for session_id in ephemeral {
            self.forget_ephemeral(session_id);
        }
        Ok(())
    }

    fn repair_session_projection(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError> {
        self.store(session_id).repair_session_projection(session_id)
    }

    fn shutdown(&self, deadline: Instant) -> Vec<AgentHistoryStoreError> {
        self.durable.shutdown(deadline)
    }
}
