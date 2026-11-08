//! The image bytes of a model request, read with no session lock held.
//!
//! A request is built in two steps around the read. Under the session's
//! lock the builder collects the attachments the history references; with
//! the lock released, the history store loads them on its own worker while
//! the builder awaits; under the lock again the builder checks that what it
//! was building for is unchanged and assembles the request from the loaded
//! bytes. An attachment the history references but that was not loaded
//! means the history changed in between, which the builder answers with
//! `StaleRevision`. Attachments are content-addressed, so the request is the
//! one a read under the lock would have produced, byte for byte, and a load
//! that failed fails the request only if the request uses that attachment.

use super::*;
use crate::{AgentImageAttachmentLoadAck, AgentImageAttachmentRef};

/// The loaded bytes of the attachments one request may use.
#[derive(Default)]
pub(in crate::service) struct LoadedImages {
    loaded: Vec<(
        AgentImageAttachmentRef,
        Result<Vec<u8>, AgentHistoryStoreError>,
    )>,
}

impl LoadedImages {
    /// The bytes of `attachment`, for `ConversationItem::
    /// as_model_input_resolved`.
    pub(in crate::service) fn bytes(
        &self,
        attachment: &AgentImageAttachmentRef,
    ) -> Result<Vec<u8>, AgentError> {
        match self.loaded.iter().find(|(loaded, _)| loaded == attachment) {
            Some((_, Ok(bytes))) => Ok(bytes.clone()),
            Some((_, Err(error))) => Err(history_store_error(error.clone())),
            None => Err(AgentError::StaleRevision),
        }
    }
}

/// The distinct attachments `items` reference, in order.
pub(in crate::service) fn referenced_images<'a>(
    items: impl IntoIterator<Item = &'a ConversationItem>,
) -> Vec<AgentImageAttachmentRef> {
    let mut referenced = Vec::<AgentImageAttachmentRef>::new();
    for attachment in items
        .into_iter()
        .flat_map(ConversationItem::image_attachments)
    {
        if !referenced.contains(attachment) {
            referenced.push(attachment.clone());
        }
    }
    referenced
}

impl AgentService {
    /// Loads the attachments the session's history references now: read
    /// under a brief lock, loaded with it released.
    pub(in crate::service) async fn load_history_images(
        &self,
        session_id: AgentSessionId,
    ) -> Result<LoadedImages, AgentError> {
        let attachments = {
            let cell = self.session(session_id)?;
            let record = cell.lock()?;
            referenced_images(record.conversation.history.iter().map(|item| &item.content))
        };
        Ok(self.load_images(session_id, attachments).await)
    }

    /// Loads `attachments` on the history store's worker and awaits them.
    /// No attachment, no load.
    pub(in crate::service) async fn load_images(
        &self,
        session_id: AgentSessionId,
        attachments: Vec<AgentImageAttachmentRef>,
    ) -> LoadedImages {
        if attachments.is_empty() {
            return LoadedImages::default();
        }
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.inner.history_store.enqueue_image_attachment_load(
            session_id,
            attachments.clone(),
            AgentImageAttachmentLoadAck::new(move |loaded| {
                let _ = sender.send(loaded);
            }),
        );
        let loaded = receiver.await.unwrap_or_default();
        let mut loaded = loaded.into_iter();
        LoadedImages {
            loaded: attachments
                .into_iter()
                .map(|attachment| {
                    let bytes = loaded
                        .next()
                        .unwrap_or(Err(AgentHistoryStoreError::ShuttingDown));
                    (attachment, bytes)
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests;
