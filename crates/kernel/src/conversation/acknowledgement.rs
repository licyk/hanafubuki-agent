//! What a store hands back once queued work (an append, an attachment load)
//! completes.

use std::fmt;

use super::{AgentConversationCommit, AgentHistoryStoreError};
use crate::AgentImageAttachmentRef;

/// The acknowledgement of one queued append (`AgentHistoryStore::
/// enqueue_commit`, `enqueue_delta`).
///
/// The store acknowledges it once, with the outcome, when the append is
/// durable or has failed, on whatever thread completes the append (its
/// writer). It is cheap and never calls back into the store. A store that
/// drops it unacknowledged has not appended; the caller treats that as a
/// failed append.
///
/// Acknowledging may hand back a follow-up: a checkpoint of the same session
/// that was waiting for this append to settle (the terminal commit of a turn
/// dropped while the append was in flight). `acknowledge` makes it with the
/// `append` it is given, next, and acknowledges it in turn, so the store
/// appends it before anything else queued for the session.
pub struct AgentHistoryAppendAck(
    Box<dyn FnOnce(Result<(), AgentHistoryStoreError>) -> Option<AgentHistoryFollowUp> + Send>,
);

/// A checkpoint an acknowledgement hands back for the store to append next,
/// with its own acknowledgement.
pub struct AgentHistoryFollowUp {
    pub commit: AgentConversationCommit,
    pub ack: AgentHistoryAppendAck,
}

impl AgentHistoryAppendAck {
    pub fn new(
        acknowledge: impl FnOnce(Result<(), AgentHistoryStoreError>) -> Option<AgentHistoryFollowUp>
            + Send
            + 'static,
    ) -> Self {
        Self(Box::new(acknowledge))
    }

    /// Acknowledges `outcome`, then appends each follow-up it hands back with
    /// `append` and acknowledges that outcome in turn. A store that cannot
    /// append passes an `append` that fails with its error.
    pub fn acknowledge(
        self,
        outcome: Result<(), AgentHistoryStoreError>,
        mut append: impl FnMut(AgentConversationCommit) -> Result<(), AgentHistoryStoreError>,
    ) {
        let mut follow_up = (self.0)(outcome);
        while let Some(AgentHistoryFollowUp { commit, ack }) = follow_up {
            follow_up = (ack.0)(append(commit));
        }
    }

    /// Acknowledges `error`, and every follow-up with it.
    pub fn fail(self, error: AgentHistoryStoreError) {
        self.acknowledge(Err(error.clone()), |_| Err(error.clone()));
    }
}

impl fmt::Debug for AgentHistoryAppendAck {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AgentHistoryAppendAck")
    }
}

impl fmt::Debug for AgentHistoryFollowUp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentHistoryFollowUp")
            .field("session_id", &self.commit.runtime.id)
            .field("revision", &self.commit.runtime.revision)
            .finish_non_exhaustive()
    }
}

/// The bytes of each attachment of one queued load (`AgentHistoryStore::
/// enqueue_image_attachment_load`), in the order they were asked for.
pub type AgentImageAttachmentLoad = Vec<Result<Vec<u8>, AgentHistoryStoreError>>;

/// The acknowledgement of one queued attachment load. The store acknowledges
/// it once, on whatever thread made the load; one it drops unacknowledged
/// has loaded nothing, which the caller treats as the store shutting down.
pub struct AgentImageAttachmentLoadAck(Box<dyn FnOnce(AgentImageAttachmentLoad) + Send>);

impl AgentImageAttachmentLoadAck {
    pub fn new(acknowledge: impl FnOnce(AgentImageAttachmentLoad) + Send + 'static) -> Self {
        Self(Box::new(acknowledge))
    }

    pub fn acknowledge(self, loaded: AgentImageAttachmentLoad) {
        (self.0)(loaded);
    }

    /// Acknowledges `error` for every attachment of `attachments`.
    pub fn fail(self, attachments: &[AgentImageAttachmentRef], error: AgentHistoryStoreError) {
        self.acknowledge(attachments.iter().map(|_| Err(error.clone())).collect());
    }
}

impl fmt::Debug for AgentImageAttachmentLoadAck {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AgentImageAttachmentLoadAck")
    }
}
