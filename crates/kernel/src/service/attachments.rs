use super::persistence::history_store_error;
use super::AgentService;
use crate::conversation::AgentHistoryStore;
use crate::model::{AgentError, AgentSessionId};

impl AgentService {
    pub fn stage_image_attachment(
        &self,
        session_id: AgentSessionId,
        image: crate::PreparedAgentImage,
    ) -> Result<crate::AgentImageAttachmentRef, AgentError> {
        self.inner
            .history_store
            .stage_image_attachment(session_id, image)
            .map_err(history_store_error)
    }

    /// A draft may use a staged image owned by this session, or a visible
    /// inherited image. Cross-session references still require history visibility.
    pub fn read_input_image_attachment(
        &self,
        session_id: AgentSessionId,
        attachment: &crate::AgentImageAttachmentRef,
    ) -> Result<Vec<u8>, AgentError> {
        self.session(session_id)?;
        if attachment.source_session_id == session_id {
            self.inner
                .history_store
                .load_image_attachment(attachment)
                .map_err(history_store_error)
        } else {
            self.read_image_attachment(session_id, attachment)
        }
    }

    pub fn read_image_attachment(
        &self,
        session_id: AgentSessionId,
        attachment: &crate::AgentImageAttachmentRef,
    ) -> Result<Vec<u8>, AgentError> {
        self.inner
            .history_store
            .load_visible_image_attachment(session_id, attachment)
            .map_err(history_store_error)
    }

    pub fn cleanup_unreferenced_image_attachments(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentError> {
        self.inner
            .history_store
            .cleanup_unreferenced_image_attachments(session_id)
            .map_err(history_store_error)
    }
}
