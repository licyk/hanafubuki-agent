// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The chat's images: what the app attaches to the draft (the clipboard's
//! image, Ctrl+V), and the message with images a model is known not to
//! read, refused before it is sent with its draft kept (Codex's
//! `image_inputs_not_supported_message`, there at the paste). The session
//! says what it knows ([`SessionStatus::images_refused`]); while nothing
//! says, the message is sent and the Agent's own fallback applies.
//!
//! [`SessionStatus::images_refused`]: crate::agent_bridge::SessionStatus

use super::ChatWidget;
use crate::user_message::{MessageImage, UserMessage};

impl ChatWidget {
    /// Whether the clipboard's image would go into the composer's draft.
    pub(crate) fn composer_takes_images(&self) -> bool {
        self.bottom_pane.composer_takes_images()
    }

    /// Why no image can be attached to the draft now; `Ok` when one can.
    pub(crate) fn image_attachable(&self) -> Result<(), String> {
        self.bottom_pane.image_attachable()
    }

    pub(crate) fn attach_image(&mut self, image: MessageImage) {
        self.bottom_pane.attach_image(image);
        self.request_redraw();
    }

    /// Why `message` is not sent: it has images, and the session's model
    /// is known to read none.
    pub(super) fn images_refusal(&self, message: &UserMessage) -> Option<String> {
        if message.images.is_empty() {
            return None;
        }
        let status = self.appearance.values().status.as_ref()?;
        let why = status.images_refused.as_ref()?;
        Some(format!(
            "This model does not read images ({why}); remove the image or switch with /model."
        ))
    }
}
